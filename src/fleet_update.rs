//! Per-machine updates. There is no fleet RPC or caller-selected release.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

const HANDOFF_GRACE: Duration = Duration::from_secs(30);
const HANDOFF_GRACE_MS: i64 = 30_000;
const PROBE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub update_id: String,
    pub tag: String,
    pub success: bool,
    pub reason: Option<String>,
}

pub fn read_outcome(directory: &Path) -> Option<Outcome> {
    serde_json::from_slice(&std::fs::read(directory.join("updates/outcome.json")).ok()?).ok()
}

pub fn save_outcome(directory: &Path, outcome: &Outcome) -> Result<(), String> {
    std::fs::create_dir_all(directory.join("updates")).map_err(|e| e.to_string())?;
    crate::store::write_atomic(
        &directory.join("updates/outcome.json"),
        &serde_json::to_vec(outcome).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn self_update_target(directory: &Path) -> Option<crate::install::InstalledTarget> {
    // A checkout is never remotely updateable, even on Linux.
    let exe = std::env::current_exe().ok()?;
    if let Some(target) = crate::install::installed_target() {
        return Some(target);
    }
    let service: Value =
        serde_json::from_slice(&std::fs::read(directory.join("service.json")).ok()?).ok()?;
    if cfg!(target_os = "linux") && service["executable"].as_str() == exe.to_str() {
        return Some(crate::install::InstalledTarget::Linux { exe });
    }
    None
}

pub trait Host: Send {
    fn run(
        &mut self,
        directory: &Path,
        id: &str,
        progress: &dyn Fn(&str, &str),
    ) -> Result<Option<String>, String>;
}
struct InstalledHost;
impl Host for InstalledHost {
    fn run(
        &mut self,
        directory: &Path,
        id: &str,
        progress: &dyn Fn(&str, &str),
    ) -> Result<Option<String>, String> {
        crate::update::request_latest(directory, id, progress)
    }
}

pub struct ServerUpdates {
    pub phase: String,
    pub id: Option<String>,
    pub tag: String,
    pub outcome: Option<Outcome>,
    events: Option<mpsc::Receiver<(String, String)>>,
    host: Option<Box<dyn Host>>,
    pub capable: bool,
    unheld_since: Option<Instant>,
    last_probe: Option<Instant>,
    diagnostic: Option<String>,
    pending_failure: Option<Outcome>,
}
fn request_file(path: &Path) -> Result<Option<Value>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "The pending update request is malformed.".to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(crate::update::redact(&error.to_string())),
    }
}
impl ServerUpdates {
    pub fn new(directory: &Path) -> Self {
        let outcome = read_outcome(directory);
        let path = directory.join("updates/request.json");
        let (request, diagnostic) = match request_file(&path) {
            Ok(request) => (request.unwrap_or(Value::Null), None),
            Err(error) => (
                Value::Null,
                Some(format!(
                    "{error} The update stays active; repair the request."
                )),
            ),
        };
        let pending = request["updateId"]
            .as_str()
            .filter(|id| outcome.as_ref().is_none_or(|o| o.update_id != *id));
        let mut updates = Self {
            phase: if diagnostic.is_some() {
                "checking"
            } else if pending.is_some() {
                "restarting"
            } else if outcome.as_ref().is_some_and(|o| !o.success) {
                "failed"
            } else {
                "idle"
            }
            .into(),
            id: pending.map(str::to_owned),
            tag: request["tag"].as_str().unwrap_or("").into(),
            outcome,
            events: None,
            host: Some(Box::new(InstalledHost)),
            capable: self_update_target(directory).is_some(),
            unheld_since: None,
            last_probe: None,
            diagnostic,
            pending_failure: None,
        };
        updates.reconcile(directory, Instant::now());
        updates
    }
    fn grace_expired(&mut self, request: &Value, path: &Path, now: Instant) -> bool {
        let now_ms = crate::lifecycle::now() as i64;
        let started_ms = request["startedAt"].as_i64().or_else(|| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
        });
        if let Some(started) = started_ms.filter(|s| *s <= now_ms)
            && now_ms.saturating_sub(started) >= HANDOFF_GRACE_MS
        {
            return true;
        }
        now.duration_since(*self.unheld_since.get_or_insert(now)) >= HANDOFF_GRACE
    }
    fn interrupted_reason(directory: &Path) -> &'static str {
        if directory.join("update-validation").exists() {
            "The updater was interrupted while restart validation was still pending. The \
             validation hold is untouched. Retry the update explicitly."
        } else {
            "The updater was interrupted before the helper finished. Retry the update \
             explicitly."
        }
    }
    fn persist(&mut self, directory: &Path, outcome: Outcome) {
        match save_outcome(directory, &outcome) {
            Ok(()) => {
                self.phase = if outcome.success { "idle" } else { "failed" }.into();
                self.outcome = Some(outcome);
                self.diagnostic = None;
            }
            Err(error) => {
                self.diagnostic = Some(crate::update::redact(&error));
                self.pending_failure = Some(outcome);
            }
        }
    }
    fn reconcile(&mut self, directory: &Path, now: Instant) {
        if self.phase != "restarting" || self.id.is_none() {
            return;
        }
        self.last_probe = Some(now);
        let updates = directory.join("updates");
        let _guard = match crate::install::InstallLock::try_acquire(&updates) {
            Ok(Some(guard)) => guard,
            Ok(None) => return,
            Err(error) => {
                self.diagnostic = Some(crate::update::redact(&error));
                return;
            }
        };
        if let Some(outcome) = read_outcome(directory)
            && outcome.update_id.as_str() == self.id.as_deref().unwrap_or_default()
        {
            self.phase = if outcome.success { "idle" } else { "failed" }.into();
            self.outcome = Some(outcome);
            self.diagnostic = None;
            return;
        }
        let path = updates.join("request.json");
        let request = match request_file(&path) {
            Ok(request) => request.unwrap_or(Value::Null),
            Err(error) => {
                self.diagnostic = Some(format!(
                    "{error} The update stays active; repair the request."
                ));
                return;
            }
        };
        if request["updateId"].is_string() && request["updateId"].as_str() != self.id.as_deref() {
            if let Some(failure) = self.pending_failure.take() {
                self.persist(directory, failure);
            }
            return;
        }
        let matching = request["updateId"].is_string();
        if let Some(failure) = self.pending_failure.take() {
            if matching && !self.grace_expired(&request, &path, now) {
                self.pending_failure = Some(failure);
                return;
            }
            self.persist(directory, failure);
            return;
        }
        if !self.grace_expired(&request, &path, now) {
            return;
        }
        let outcome = Outcome {
            update_id: self.id.clone().unwrap_or_default(),
            tag: request["tag"]
                .as_str()
                .filter(|tag| !tag.is_empty())
                .unwrap_or(&self.tag)
                .to_owned(),
            success: false,
            reason: Some(Self::interrupted_reason(directory).into()),
        };
        if let Err(error) = save_outcome(directory, &outcome) {
            self.diagnostic = Some(crate::update::redact(&error));
            return;
        }
        if let Ok(Some(current)) = request_file(&path)
            && current["updateId"].as_str() == Some(outcome.update_id.as_str())
            && let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            self.diagnostic = Some(crate::update::redact(&error.to_string()));
        }
        self.outcome = Some(outcome);
        self.phase = "failed".into();
    }
    #[cfg(test)]
    pub fn fake(&mut self, host: impl Host + 'static) {
        self.capable = true;
        self.host = Some(Box::new(host));
    }
    pub fn active(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "checking" | "downloading" | "ready" | "draining" | "restarting"
        )
    }
    pub fn status(&self, blocker: Option<&str>, maintenance: &Value) -> Value {
        let phase = maintenance["phase"]
            .as_str()
            .filter(|p| matches!(*p, "draining" | "restarting"))
            .unwrap_or(&self.phase);
        json!({"runningVersion":env!("CARGO_PKG_VERSION"),"canSelfUpdate":self.capable,
            "state":phase,"updateId":self.id,"tag":self.tag,"lastOutcome":self.outcome,"blocker":blocker,
            "diagnostic":self.diagnostic})
    }
    /// Called only after the accepted receipt and audit row commit.
    pub fn start(&mut self, directory: PathBuf, id: String) {
        self.phase = "checking".into();
        self.id = Some(id.clone());
        self.tag.clear();
        self.unheld_since = None;
        self.last_probe = None;
        self.diagnostic = None;
        self.pending_failure = None;
        let (send, receive) = mpsc::channel();
        self.events = Some(receive);
        let Some(mut host) = self.host.take() else {
            return;
        };
        std::thread::spawn(move || {
            let result = host.run(&directory, &id, &|phase, tag| {
                let _ = send.send((phase.into(), tag.into()));
            });
            match result {
                Ok(Some(tag)) => {
                    let _ = send.send(("restarting".into(), tag));
                }
                Ok(None) => {
                    let _ = save_outcome(
                        &directory,
                        &Outcome {
                            update_id: id,
                            tag: env!("CARGO_PKG_VERSION").into(),
                            success: true,
                            reason: None,
                        },
                    );
                    let _ = send.send(("idle".into(), String::new()));
                }
                Err(reason) => {
                    let _ = send.send(("failed".into(), reason));
                }
            }
        });
    }
    pub fn poll(&mut self, directory: &Path) -> bool {
        let before = self.status(None, &Value::Null);
        let mut disconnected = false;
        if let Some(events) = &self.events {
            loop {
                match events.try_recv() {
                    Ok((phase, detail)) => {
                        if phase == "failed" {
                            self.pending_failure = Some(Outcome {
                                update_id: self.id.clone().unwrap_or_default(),
                                tag: self.tag.clone(),
                                success: false,
                                reason: Some(crate::update::redact(&detail)),
                            });
                            self.phase = "restarting".into();
                        } else {
                            if !detail.is_empty() {
                                self.tag = detail;
                            }
                            self.phase = phase;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if disconnected {
            self.events = None;
            if matches!(self.phase.as_str(), "checking" | "downloading" | "ready") {
                self.phase = "restarting".into();
            }
        }
        if let Some(outcome) = read_outcome(directory)
            && (!self.active() || self.id.is_some())
            && self.id.as_deref().is_none_or(|id| id == outcome.update_id)
        {
            self.phase = if outcome.success { "idle" } else { "failed" }.into();
            self.outcome = Some(outcome);
            self.diagnostic = None;
        }
        let probe_due = self
            .last_probe
            .is_none_or(|probed| probed.elapsed() >= PROBE_INTERVAL);
        if !probe_due {
            return before != self.status(None, &Value::Null);
        }
        if self.phase == "checking" && self.id.is_none() {
            self.last_probe = Some(Instant::now());
            match request_file(&directory.join("updates/request.json")) {
                Ok(Some(request)) => {
                    if let Some(id) = request["updateId"].as_str() {
                        self.id = Some(id.to_owned());
                        self.tag = request["tag"].as_str().unwrap_or("").into();
                        self.phase = "restarting".into();
                        self.diagnostic = None;
                    } else {
                        self.diagnostic =
                            Some("The pending update request is malformed.".to_owned());
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.diagnostic = Some(format!(
                        "{error} The update stays active; repair the request."
                    ));
                }
            }
        } else {
            self.reconcile(directory, Instant::now());
        }
        if !self.active() && self.host.is_none() {
            self.host = Some(Box::new(InstalledHost));
        }
        before != self.status(None, &Value::Null)
    }
}

/// The first release whose update helper leaves the service's process group, control group
/// or task tree. Older targets start the helper inside the service, so the service manager
/// kills it as soon as the server stops for the restart. The update never installs and the
/// service stays stopped, because it exited cleanly. Those targets must be updated from a
/// shell on the machine, outside the service.
const DETACHED_HELPER_SINCE: &str = "0.12.28";
const OLD_HELPER_SKIP: &str =
    "its updater would stop with its service. Run shina update --yes on that machine";

/// Ported from KSG Code's updateEverything.logic collectServerUpdateTargets.
pub fn skip_reason(online: bool, hello: &Value, status: &Value, target: &str) -> Option<String> {
    if !online {
        return Some("offline".into());
    }
    if hello["protocolMinor"].as_u64().unwrap_or(0) < 6 {
        return Some("older protocol".into());
    }
    if hello["canUpdate"] != true {
        return Some("no update grant".into());
    }
    if !crate::update::is_newer(target, status["runningVersion"].as_str().unwrap_or("")) {
        return None;
    }
    if status["canSelfUpdate"] != true {
        return Some("cannot self-update".into());
    }
    if crate::update::is_newer(
        DETACHED_HELPER_SINCE,
        status["runningVersion"].as_str().unwrap_or(""),
    ) {
        return Some(OLD_HELPER_SKIP.into());
    }
    if let Some(blocker) = status["blocker"].as_str() {
        return Some(format!("busy: {blocker}"));
    }
    if matches!(
        status["state"].as_str(),
        Some("checking" | "downloading" | "ready" | "draining" | "restarting")
    ) {
        return Some("already updating".into());
    }
    None
}

/// A link that reached the machine but was refused (for example an older build) is not
/// connected, yet a one-off request still answers. Only a link that never started counts as
/// offline without asking.
fn should_probe(connected: bool, phase: crate::client::ConnectionPhase) -> bool {
    connected || phase != crate::client::ConnectionPhase::Offline
}

#[derive(Clone)]
pub struct Row {
    pub server: String,
    pub name: String,
    pub current: String,
    pub target: String,
    pub state: String,
    pub eligible: bool,
    pub failed: bool,
    pub update_id: Option<String>,
    pub prior_outcome: Option<String>,
    pub attempted: bool,
    pub link: Option<crate::peer::Link>,
}
#[derive(Default)]
pub struct Fleet {
    pub open: bool,
    pub running: bool,
    pub rows: Vec<Row>,
    events: Option<mpsc::Receiver<(usize, Result<Value, String>, bool)>>,
    sender: Option<mpsc::Sender<(usize, Result<Value, String>, bool)>>,
    pending_handoffs: usize,
    local_started: bool,
}

impl crate::model::State {
    pub fn fleet_available(&self) -> bool {
        let target = self
            .updater
            .prepared_update()
            .map(|u| u.tag.as_str())
            .unwrap_or(env!("CARGO_PKG_VERSION"));
        self.client.as_ref().is_some_and(|client| {
            client.linked.values().any(|remote| {
                remote
                    .version()
                    .is_some_and(|version| crate::update::is_newer(target, &version))
            })
        }) || self.machines.views.iter().any(|(id, view)| {
            id != "local"
                && crate::update::is_newer(
                    target,
                    view["updateStatus"]["runningVersion"]
                        .as_str()
                        .unwrap_or(target),
                )
        })
    }
    pub fn open_fleet(&mut self) {
        if self.fleet.running {
            self.fleet.open = true;
            return;
        }
        let target = self
            .updater
            .prepared_update()
            .map(|u| u.tag.clone())
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());
        let local_update = self.updater.prepared_update().is_some();
        self.fleet = Fleet {
            open: true,
            ..Default::default()
        };
        self.fleet.rows.push(Row {
            server: "local".into(),
            name: "Local".into(),
            current: env!("CARGO_PKG_VERSION").into(),
            target: target.clone(),
            state: if local_update {
                self.update_restart_blocker()
                    .map(|s| format!("skipped: {s}"))
                    .unwrap_or("will update".into())
            } else {
                "up to date".into()
            },
            eligible: local_update && self.update_restart_blocker().is_none(),
            failed: false,
            update_id: None,
            prior_outcome: None,
            attempted: false,
            link: None,
        });
        let (send, receive) = mpsc::channel();
        self.fleet.events = Some(receive);
        self.fleet.sender = Some(send.clone());
        if let Some(client) = &self.client {
            for (id, remote) in &client.linked {
                let Some(link) = remote.remote.clone() else {
                    continue;
                };
                let index = self.fleet.rows.len();
                let current = remote.version().unwrap_or_else(|| {
                    self.machines
                        .views
                        .get(id)
                        .and_then(|v| v["updateStatus"]["runningVersion"].as_str())
                        .unwrap_or("unknown")
                        .to_owned()
                });
                let online =
                    should_probe(remote.commands.connected(), remote.connection_state().phase);
                self.fleet.rows.push(Row {
                    server: id.clone(),
                    name: link.host.clone(),
                    current,
                    target: target.clone(),
                    state: if online {
                        "checking".into()
                    } else {
                        "skipped: offline".into()
                    },
                    eligible: false,
                    failed: false,
                    update_id: None,
                    prior_outcome: None,
                    attempted: false,
                    link: Some(link.clone()),
                });
                if !online {
                    continue;
                }
                let send = send.clone();
                let wake = self.wake.clone();
                let target = target.clone();
                std::thread::spawn(move || {
                    let result = tokio::runtime::Runtime::new().map_err(|e| e.to_string()).and_then(|rt| rt.block_on(async {
                        let hello = crate::client::remote_rpc(&link, "hello", json!({})).await?;
                        let status = if hello["canUpdate"] == true && hello["protocolMinor"].as_u64().unwrap_or(0) >= 6 {
                            crate::client::remote_rpc(&link, "server.updateStatus", json!({})).await?
                        } else { Value::Null };
                        let skipped = skip_reason(true, &hello, &status, &target);
                        Ok(json!({"current":hello["binaryVersion"],"skip":skipped,"status":status}))
                    }));
                    let _ = send.send((index, result, false));
                    wake();
                });
            }
        }
    }
    pub fn confirm_fleet(&mut self) {
        if self.demo {
            return;
        }
        self.fleet.running = true;
        self.fleet.local_started = false;
        self.fleet.pending_handoffs = 0;
        let indices: Vec<_> = self
            .fleet
            .rows
            .iter()
            .enumerate()
            .filter(|(i, r)| *i != 0 && r.eligible)
            .map(|(i, _)| i)
            .collect();
        for index in indices {
            self.start_fleet_row(index);
        }
    }
    pub fn start_fleet_row(&mut self, index: usize) {
        if index == 0 {
            self.restart_notice = None;
            if let Some(blocker) = self.update_restart_blocker() {
                self.fleet.rows[0].state = format!("failed: {blocker}");
            } else {
                self.fleet.rows[0].failed = false;
                self.fleet.rows[0].state = "restarting".into();
                self.restart_for_update = true;
            }
            return;
        }
        let Some(link) = self.fleet.rows[index].link.clone() else {
            return;
        };
        let Some(send) = self.fleet.sender.clone() else {
            return;
        };
        self.fleet.pending_handoffs += 1;
        self.fleet.rows[index].state = "downloading".into();
        self.fleet.rows[index].failed = false;
        self.fleet.rows[index].update_id = None;
        self.fleet.rows[index].attempted = true;
        self.fleet.rows[index].prior_outcome = self
            .machines
            .views
            .get(&self.fleet.rows[index].server)
            .and_then(|view| view["updateStatus"]["lastOutcome"]["updateId"].as_str())
            .map(str::to_owned);
        self.fleet.rows[index].eligible = false;
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Runtime::new()
                .map_err(|e| e.to_string())
                .and_then(|rt| {
                    rt.block_on(crate::client::remote_rpc(
                        &link,
                        "server.requestUpdate",
                        json!({}),
                    ))
                });
            let _ = send.send((index, result, true));
            wake();
        });
    }
    pub fn poll_fleet(&mut self) -> bool {
        let mut changed = false;
        if let Some(events) = &self.fleet.events {
            while let Ok((index, result, requested)) = events.try_recv() {
                changed = true;
                let Some(row) = self.fleet.rows.get_mut(index) else {
                    continue;
                };
                if requested {
                    self.fleet.pending_handoffs = self.fleet.pending_handoffs.saturating_sub(1);
                }
                match result {
                    Ok(value) if requested && value["status"] == "accepted" => {
                        row.failed = false;
                        row.update_id = value["result"]["updateId"].as_str().map(str::to_owned);
                        row.state = "downloading".into();
                    }
                    Ok(value) if !requested => {
                        row.prior_outcome = value["status"]["lastOutcome"]["updateId"]
                            .as_str()
                            .map(str::to_owned);
                        row.current = value["current"].as_str().unwrap_or(&row.current).into();
                        row.state = if let Some(reason) = value["skip"].as_str() {
                            format!("skipped: {reason}")
                        } else if crate::update::is_newer(&row.target, &row.current) {
                            row.eligible = true;
                            "will update".into()
                        } else {
                            "up to date".into()
                        };
                    }
                    result => {
                        let reason = match result {
                            Err(e) => e,
                            Ok(v) => v["result"]["error"]
                                .as_str()
                                .unwrap_or("Update refused.")
                                .into(),
                        };
                        // The link was not offline, the probe reached the network and failed.
                        // The reason says how, for example a stopped server.
                        row.state = if requested {
                            format!("failed: {reason}")
                        } else {
                            format!("skipped: {reason}")
                        };
                        row.failed = requested;
                    }
                }
            }
        }
        let offline: Vec<_> = self
            .fleet
            .rows
            .iter()
            .filter(|r| !self.machine_connected(&r.server))
            .map(|r| r.server.clone())
            .collect();
        for row in self.fleet.rows.iter_mut().skip(1).filter(|r| r.attempted) {
            if let Some(view) = self.machines.views.get(&row.server) {
                let status = &view["updateStatus"];
                let outcome = &status["lastOutcome"];
                if row.update_id.is_none() {
                    // A transport loss is not proof of refusal. Observe a new target-side
                    // attempt/outcome after reconnect, but never resend an update request.
                    let next = if matches!(
                        status["state"].as_str(),
                        Some("checking" | "downloading" | "ready" | "draining" | "restarting")
                    ) {
                        status["updateId"].as_str()
                    } else {
                        outcome["updateId"]
                            .as_str()
                            .filter(|id| Some(*id) != row.prior_outcome.as_deref())
                    };
                    if let Some(id) = next {
                        row.update_id = Some(id.into());
                        row.failed = false;
                    } else {
                        continue;
                    }
                }
                let previous = row.state.clone();
                if outcome["updateId"].as_str() == row.update_id.as_deref() {
                    if outcome["success"] == true {
                        row.current = status["runningVersion"]
                            .as_str()
                            .unwrap_or(&row.current)
                            .into();
                        if crate::update::is_newer(&row.target, &row.current) {
                            row.state =
                                "failed: No newer release is available for this machine yet."
                                    .into();
                            row.failed = true;
                        } else {
                            row.state = format!("updated to {}", row.current);
                            row.failed = false;
                        }
                    } else {
                        row.state = format!(
                            "failed: {}",
                            outcome["reason"].as_str().unwrap_or("Unknown failure")
                        );
                        row.failed = true;
                    }
                } else if matches!(status["state"].as_str(), Some("draining" | "restarting")) {
                    row.state = "restarting".into();
                    row.failed = false;
                } else if offline.contains(&row.server) {
                    row.state =
                        "failed: Connection lost after acceptance. The target may still update."
                            .into();
                    row.failed = true;
                } else if matches!(
                    status["state"].as_str(),
                    Some("checking" | "downloading" | "ready")
                ) {
                    row.state = "downloading".into();
                    row.failed = false;
                }
                changed |= previous != row.state;
            }
        }
        if self.fleet.running
            && self.fleet.pending_handoffs == 0
            && !self.fleet.local_started
            && self
                .fleet
                .rows
                .iter()
                .skip(1)
                .all(|row| !matches!(row.state.as_str(), "downloading" | "checking" | "ready"))
        {
            self.fleet.local_started = true;
            if self.fleet.rows.first().is_some_and(|r| r.eligible) {
                // Remote requests have all returned. The local restart stays last.
                self.restart_for_update = true;
                self.fleet.rows[0].state = "restarting".into();
                changed = true;
            }
        }
        if self.fleet.local_started
            && let Some(reason) = &self.restart_notice
            && let Some(row) = self.fleet.rows.first_mut()
        {
            let message = format!("failed: {reason}");
            changed |= row.state != message;
            row.state = message;
            row.failed = true;
        }
        changed
    }
    pub fn demo_fleet(&mut self) {
        self.fleet = Fleet {
            open: true,
            ..Default::default()
        };
        for (index, state) in [
            "will update",
            "up to date",
            "skipped: offline",
            "skipped: no update grant",
            "skipped: cannot self-update",
            "skipped: busy: 2 threads are running",
            "skipped: older protocol",
            "downloading",
            "restarting",
            "updated to 99.0.0",
            "failed: Readiness validation failed. Rolled back.",
        ]
        .iter()
        .enumerate()
        {
            self.fleet.rows.push(Row {
                server: format!("fixture-{index}"),
                name: if index == 0 {
                    "Local".into()
                } else {
                    format!("Machine {index}")
                },
                current: if *state == "up to date" || state.starts_with("updated to") {
                    "99.0.0"
                } else {
                    "0.12.0"
                }
                .into(),
                target: "99.0.0".into(),
                state: (*state).into(),
                eligible: index == 0,
                failed: state.starts_with("failed"),
                update_id: None,
                prior_outcome: None,
                attempted: false,
                link: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("shina-fleet-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(path.join("updates")).unwrap();
        path
    }

    fn write_request(dir: &Path, request: Value) {
        crate::store::write_atomic(
            &dir.join("updates/request.json"),
            request.to_string().as_bytes(),
        )
        .unwrap();
    }

    fn age_request(dir: &Path, seconds: u64) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("updates/request.json"))
            .unwrap();
        file.set_modified(std::time::SystemTime::now() - Duration::from_secs(seconds))
            .unwrap();
    }

    fn probe_again(updates: &mut ServerUpdates) {
        updates.last_probe = None;
    }

    fn past_grace(updates: &mut ServerUpdates) {
        updates.unheld_since = Some(Instant::now() - HANDOFF_GRACE - Duration::from_secs(1));
    }

    #[test]
    fn an_orphaned_old_request_fails_at_startup_and_stays_terminal() {
        let dir = tempdir("orphan");
        write_request(&dir, json!({"updateId":"orphan-1","tag":"v0.19.0"}));
        age_request(&dir, 120);
        let mut updates = ServerUpdates::new(&dir);
        assert_eq!(updates.phase, "failed");
        assert!(!updates.active());
        let outcome = updates.outcome.clone().unwrap();
        assert_eq!(outcome.update_id, "orphan-1");
        assert_eq!(outcome.tag, "v0.19.0");
        assert!(!outcome.success);
        assert!(outcome.reason.unwrap().contains("interrupted"));
        assert!(!dir.join("updates/request.json").exists());
        let again = ServerUpdates::new(&dir);
        assert_eq!(again.phase, "failed");
        probe_again(&mut updates);
        assert!(
            !updates.poll(&dir),
            "a terminal attempt reports no new change"
        );
        assert_eq!(updates.phase, "failed");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_held_lock_keeps_an_old_request_restarting_until_the_helper_is_gone() {
        let dir = tempdir("held");
        write_request(
            &dir,
            json!({"updateId":"held-1","tag":"v0.19.0","startedAt":0}),
        );
        let guard = crate::install::InstallLock::acquire(&dir.join("updates")).unwrap();
        let mut updates = ServerUpdates::new(&dir);
        assert_eq!(updates.phase, "restarting");
        assert!(updates.active());
        assert!(updates.outcome.is_none());
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "restarting");
        assert!(
            read_outcome(&dir).is_none(),
            "a live helper is never failed"
        );
        drop(guard);
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "failed");
        assert_eq!(
            read_outcome(&dir).unwrap().update_id,
            "held-1",
            "the released abandoned request reconciles to a durable failure"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_fresh_request_survives_the_handoff_grace_then_fails() {
        let dir = tempdir("fresh");
        write_request(
            &dir,
            json!({"updateId":"fresh-1","tag":"v0.19.0","startedAt":crate::lifecycle::now()}),
        );
        let mut updates = ServerUpdates::new(&dir);
        assert_eq!(updates.phase, "restarting");
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(
            updates.phase, "restarting",
            "the helper may still be starting"
        );
        past_grace(&mut updates);
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "failed");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_future_or_invalid_start_time_cannot_hold_an_attempt_active_forever() {
        for started_at in [
            json!(crate::lifecycle::now() + 3_600_000),
            json!("tomorrow"),
        ] {
            let dir = tempdir("future");
            write_request(
                &dir,
                json!({"updateId":"future-1","tag":"v0.19.0","startedAt":started_at}),
            );
            let mut updates = ServerUpdates::new(&dir);
            assert_eq!(updates.phase, "restarting");
            past_grace(&mut updates);
            probe_again(&mut updates);
            updates.poll(&dir);
            assert_eq!(updates.phase, "failed");
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn a_pending_validation_hold_is_reported_and_left_untouched() {
        let dir = tempdir("validation");
        std::fs::write(dir.join("update-validation"), b"hold").unwrap();
        write_request(
            &dir,
            json!({"updateId":"held-2","tag":"v0.19.0","startedAt":0}),
        );
        let updates = ServerUpdates::new(&dir);
        assert_eq!(updates.phase, "failed");
        let reason = updates.outcome.unwrap().reason.unwrap();
        assert!(reason.contains("validation"), "{reason}");
        assert!(reason.contains("Retry"));
        assert!(dir.join("update-validation").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_outcome_that_cannot_be_persisted_keeps_the_pending_request() {
        let dir = tempdir("unsaved");
        write_request(
            &dir,
            json!({"updateId":"unsaved-1","tag":"v0.19.0","startedAt":0}),
        );
        std::fs::create_dir(dir.join("updates/outcome.json")).unwrap();
        let mut updates = ServerUpdates::new(&dir);
        assert_eq!(updates.phase, "restarting");
        assert!(updates.diagnostic.is_some());
        assert!(dir.join("updates/request.json").exists());
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "restarting");
        assert!(updates.outcome.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_stale_outcome_for_another_attempt_never_completes_the_current_one() {
        let dir = tempdir("stale");
        save_outcome(
            &dir,
            &Outcome {
                update_id: "older-attempt".into(),
                tag: "v0.18.0".into(),
                success: true,
                reason: None,
            },
        )
        .unwrap();
        write_request(
            &dir,
            json!({"updateId":"current-1","tag":"v0.19.0","startedAt":crate::lifecycle::now()}),
        );
        let guard = crate::install::InstallLock::acquire(&dir.join("updates")).unwrap();
        let mut updates = ServerUpdates::new(&dir);
        assert_eq!(updates.phase, "restarting");
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "restarting");
        save_outcome(
            &dir,
            &Outcome {
                update_id: "current-1".into(),
                tag: "v0.19.0".into(),
                success: true,
                reason: None,
            },
        )
        .unwrap();
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "idle");
        assert_eq!(updates.outcome.unwrap().update_id, "current-1");
        drop(guard);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_success_written_inside_the_probe_window_is_never_overwritten() {
        for remove_request in [false, true] {
            let dir = tempdir("race");
            write_request(
                &dir,
                json!({"updateId":"race-1","tag":"v0.19.0","startedAt":0}),
            );
            let guard = crate::install::InstallLock::acquire(&dir.join("updates")).unwrap();
            let mut updates = ServerUpdates::new(&dir);
            assert_eq!(updates.phase, "restarting");
            save_outcome(
                &dir,
                &Outcome {
                    update_id: "race-1".into(),
                    tag: "v0.19.0".into(),
                    success: true,
                    reason: None,
                },
            )
            .unwrap();
            if remove_request {
                std::fs::remove_file(dir.join("updates/request.json")).unwrap();
            }
            drop(guard);
            updates.reconcile(&dir, Instant::now());
            assert_eq!(updates.phase, "idle", "remove_request={remove_request}");
            assert!(updates.outcome.unwrap().success);
            assert!(read_outcome(&dir).unwrap().success);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn an_extreme_start_time_fails_immediately_instead_of_overflowing() {
        let dir = tempdir("extreme");
        write_request(
            &dir,
            json!({"updateId":"extreme-1","tag":"v0.19.0","startedAt":i64::MIN}),
        );
        let updates = ServerUpdates::new(&dir);
        assert_eq!(updates.phase, "failed");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unreadable_request_preserves_the_active_state() {
        let dir = tempdir("unreadable");
        write_request(
            &dir,
            json!({"updateId":"unreadable-1","tag":"v0.19.0","startedAt":crate::lifecycle::now()}),
        );
        let mut updates = ServerUpdates::new(&dir);
        assert_eq!(updates.phase, "restarting");
        std::fs::remove_file(dir.join("updates/request.json")).unwrap();
        std::fs::create_dir(dir.join("updates/request.json")).unwrap();
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "restarting");
        assert!(updates.diagnostic.is_some());
        assert!(read_outcome(&dir).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    struct PanicHost;
    impl Host for PanicHost {
        fn run(
            &mut self,
            _directory: &Path,
            _id: &str,
            _progress: &dyn Fn(&str, &str),
        ) -> Result<Option<String>, String> {
            panic!("worker gone");
        }
    }

    struct HandoffThenPanic;
    impl Host for HandoffThenPanic {
        fn run(
            &mut self,
            directory: &Path,
            id: &str,
            progress: &dyn Fn(&str, &str),
        ) -> Result<Option<String>, String> {
            progress("downloading", "v0.19.0");
            crate::store::write_atomic(
                &directory.join("updates/request.json"),
                json!({"updateId":id,"tag":"v0.19.0","startedAt":crate::lifecycle::now()})
                    .to_string()
                    .as_bytes(),
            )
            .map_err(|e| e.to_string())?;
            panic!("worker gone after handoff");
        }
    }

    #[test]
    fn a_worker_that_vanishes_mid_attempt_fails_and_can_be_retried() {
        let dir = tempdir("panic");
        let mut updates = ServerUpdates::new(&dir);
        updates.fake(PanicHost);
        updates.start(dir.clone(), "panic-1".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while updates.events.is_some() {
            updates.poll(&dir);
            assert!(
                Instant::now() < deadline,
                "worker disconnect was not detected"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(updates.phase, "restarting");
        past_grace(&mut updates);
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "failed");
        assert!(!updates.active());
        let outcome = read_outcome(&dir).unwrap();
        assert_eq!(outcome.update_id, "panic-1");
        assert!(!outcome.success);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_worker_lost_after_helper_handoff_is_not_failed_while_the_helper_lives() {
        let dir = tempdir("panic-handoff");
        let guard = crate::install::InstallLock::acquire(&dir.join("updates")).unwrap();
        let mut updates = ServerUpdates::new(&dir);
        updates.fake(HandoffThenPanic);
        updates.start(dir.clone(), "lost-worker".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while updates.events.is_some() {
            updates.poll(&dir);
            assert!(
                Instant::now() < deadline,
                "worker disconnect was not detected"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(updates.phase, "restarting");
        assert!(updates.outcome.is_none());
        assert!(
            read_outcome(&dir).is_none(),
            "a live helper is never failed"
        );
        drop(guard);
        past_grace(&mut updates);
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "failed");
        assert_eq!(read_outcome(&dir).unwrap().update_id, "lost-worker");
        std::fs::remove_dir_all(dir).unwrap();
    }

    struct ErrHost;
    impl Host for ErrHost {
        fn run(
            &mut self,
            _directory: &Path,
            _id: &str,
            _progress: &dyn Fn(&str, &str),
        ) -> Result<Option<String>, String> {
            Err("download exploded".into())
        }
    }

    #[test]
    fn a_worker_loss_stays_active_until_the_failure_is_durable() {
        let dir = tempdir("unpersisted");
        std::fs::create_dir(dir.join("updates/outcome.json")).unwrap();
        let mut updates = ServerUpdates::new(&dir);
        updates.fake(ErrHost);
        updates.start(dir.clone(), "unpersisted-1".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while updates.pending_failure.is_none() {
            probe_again(&mut updates);
            updates.poll(&dir);
            assert!(Instant::now() < deadline, "worker failure was not received");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(updates.phase, "restarting");
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(
            updates.phase, "restarting",
            "no durable failure, still active"
        );
        assert!(updates.diagnostic.is_some());
        std::fs::remove_dir(dir.join("updates/outcome.json")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while updates.phase != "failed" {
            probe_again(&mut updates);
            updates.poll(&dir);
            assert!(Instant::now() < deadline, "failure was never persisted");
            std::thread::sleep(Duration::from_millis(5));
        }
        let outcome = read_outcome(&dir).unwrap();
        assert_eq!(outcome.update_id, "unpersisted-1");
        assert!(!outcome.success);
        assert!(outcome.reason.unwrap().contains("download exploded"));
        assert!(updates.diagnostic.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_known_failure_yields_to_a_live_helper_before_finalizing() {
        let dir = tempdir("known-failure");
        let guard = crate::install::InstallLock::acquire(&dir.join("updates")).unwrap();
        let mut updates = ServerUpdates::new(&dir);
        struct FailAfterRequest;
        impl Host for FailAfterRequest {
            fn run(
                &mut self,
                directory: &Path,
                id: &str,
                progress: &dyn Fn(&str, &str),
            ) -> Result<Option<String>, String> {
                progress("downloading", "v0.19.0");
                crate::store::write_atomic(
                    &directory.join("updates/request.json"),
                    json!({"updateId":id,"tag":"v0.19.0","startedAt":crate::lifecycle::now()})
                        .to_string()
                        .as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                Err("validation exploded".into())
            }
        }
        updates.fake(FailAfterRequest);
        updates.start(dir.clone(), "known-1".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while updates.events.is_some() {
            probe_again(&mut updates);
            updates.poll(&dir);
            assert!(Instant::now() < deadline, "worker failure was not received");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(updates.phase, "restarting");
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "restarting");
        assert!(
            read_outcome(&dir).is_none(),
            "a live helper is never overridden"
        );
        drop(guard);
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(
            updates.phase, "restarting",
            "a matching request gets the helper grace"
        );
        assert!(read_outcome(&dir).is_none());
        past_grace(&mut updates);
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "failed");
        let outcome = read_outcome(&dir).unwrap();
        assert_eq!(outcome.update_id, "known-1");
        assert!(outcome.reason.unwrap().contains("validation exploded"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_attempt_becomes_idle_when_its_outcome_succeeds() {
        let dir = tempdir("late-success");
        let mut updates = ServerUpdates::new(&dir);
        updates.phase = "failed".into();
        updates.id = Some("late-1".into());
        updates.tag = "v0.19.0".into();
        save_outcome(
            &dir,
            &Outcome {
                update_id: "late-1".into(),
                tag: "v0.19.0".into(),
                success: true,
                reason: None,
            },
        )
        .unwrap();
        updates.poll(&dir);
        assert_eq!(updates.phase, "idle");
        assert!(updates.outcome.as_ref().unwrap().success);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_startup_request_that_cannot_be_read_stays_active_until_repaired() {
        for kind in ["directory", "malformed"] {
            for prior in [None, Some(true), Some(false)] {
                let dir = tempdir(&format!("corrupt-{kind}-{}", prior.is_some()));
                if let Some(success) = prior {
                    save_outcome(
                        &dir,
                        &Outcome {
                            update_id: "older-1".into(),
                            tag: "v0.18.0".into(),
                            success,
                            reason: None,
                        },
                    )
                    .unwrap();
                }
                let request = dir.join("updates/request.json");
                if kind == "directory" {
                    std::fs::create_dir(&request).unwrap();
                } else {
                    std::fs::write(&request, b"{ not json").unwrap();
                }
                let mut updates = ServerUpdates::new(&dir);
                assert_eq!(updates.phase, "checking");
                assert!(updates.active());
                assert!(updates.diagnostic.is_some());
                assert!(updates.id.is_none());
                probe_again(&mut updates);
                updates.poll(&dir);
                assert!(
                    updates.active(),
                    "an unresolved request stays active over a stale outcome"
                );
                assert!(updates.diagnostic.is_some());
                assert!(updates.id.is_none());
                if kind == "directory" {
                    std::fs::remove_dir(&request).unwrap();
                }
                std::fs::write(
                    &request,
                    json!({"updateId":"repaired-1","tag":"v0.19.0","startedAt":0}).to_string(),
                )
                .unwrap();
                probe_again(&mut updates);
                updates.poll(&dir);
                assert_eq!(updates.phase, "restarting");
                assert_eq!(updates.id.as_deref(), Some("repaired-1"));
                probe_again(&mut updates);
                updates.poll(&dir);
                assert_eq!(updates.phase, "failed");
                assert_eq!(read_outcome(&dir).unwrap().update_id, "repaired-1");
                std::fs::remove_dir_all(dir).unwrap();
            }
        }
    }

    struct HelperStarted;
    impl Host for HelperStarted {
        fn run(
            &mut self,
            directory: &Path,
            id: &str,
            _progress: &dyn Fn(&str, &str),
        ) -> Result<Option<String>, String> {
            crate::store::write_atomic(
                &directory.join("updates/request.json"),
                json!({"updateId":id,"tag":"v0.19.0","startedAt":crate::lifecycle::now()})
                    .to_string()
                    .as_bytes(),
            )
            .map_err(|e| e.to_string())?;
            Ok(Some("v0.19.0".into()))
        }
    }

    #[test]
    fn a_normal_restart_handoff_is_not_failed_by_the_worker_channel_closing() {
        let dir = tempdir("handoff");
        let guard = crate::install::InstallLock::acquire(&dir.join("updates")).unwrap();
        let mut updates = ServerUpdates::new(&dir);
        updates.fake(HelperStarted);
        updates.start(dir.clone(), "handoff-1".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while updates.phase != "restarting" || updates.events.is_some() {
            updates.poll(&dir);
            assert!(Instant::now() < deadline, "handoff event never arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(
            updates.phase, "restarting",
            "the held helper owns the request"
        );
        assert!(updates.outcome.is_none());
        drop(guard);
        past_grace(&mut updates);
        probe_again(&mut updates);
        updates.poll(&dir);
        assert_eq!(updates.phase, "failed");
        assert_eq!(read_outcome(&dir).unwrap().update_id, "handoff-1");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn local_restart_waits_for_remote_handoff_even_after_the_dialog_closes() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.demo_fleet();
        state.fleet.rows.truncate(2);
        state.fleet.rows[0].eligible = true;
        state.fleet.rows[1].state = "downloading".into();
        state.fleet.rows[1].update_id = Some("accepted-update".into());
        state.fleet.rows[1].attempted = true;
        state.fleet.rows[1].server = "remote".into();
        let (mut client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, _) = crate::client::Client::test_connected(state.data_dir.clone());
        client.linked.insert("remote".into(), remote);
        state.client = Some(client);
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{"state":"downloading"}}),
        );
        state.fleet.running = true;
        state.fleet.open = false;
        state.poll_fleet();
        assert!(!state.restart_for_update);
        state.machines.views.get_mut("remote").unwrap()["updateStatus"]["state"] =
            json!("restarting");
        state.poll_fleet();
        assert!(state.restart_for_update);
        assert!(!state.fleet.open);
    }
    #[test]
    fn a_refused_link_is_still_asked_for_its_update_status() {
        use crate::client::ConnectionPhase::*;
        assert!(should_probe(true, Available));
        assert!(should_probe(false, Error));
        assert!(should_probe(false, Reconnecting));
        assert!(!should_probe(false, Offline));
    }
    #[test]
    fn a_target_whose_helper_dies_with_its_service_is_never_asked_to_update() {
        // 0.12.19 services killed their own update helper when the server stopped, which
        // left serverseesghosts, kidsseemac and kidsseembp stopped on the old release.
        let hello = json!({"protocolMinor":35,"canUpdate":true});
        let status = |version: &str| json!({"runningVersion":version,"canSelfUpdate":true,"state":"idle","blocker":null});
        assert_eq!(
            skip_reason(true, &hello, &status("0.12.19"), "v0.12.28").as_deref(),
            Some(OLD_HELPER_SKIP)
        );
        assert_eq!(
            skip_reason(true, &hello, &status("0.12.28"), "v0.12.29"),
            None
        );
        assert_eq!(
            skip_reason(true, &hello, &status("0.12.19"), "v0.12.19"),
            None,
            "an up-to-date row is not a skip"
        );
    }
    #[test]
    fn a_failed_probe_reports_its_reason_instead_of_calling_the_machine_offline() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (send, receive) = mpsc::channel();
        state.fleet = Fleet {
            open: true,
            events: Some(receive),
            sender: Some(send.clone()),
            ..Default::default()
        };
        for name in ["Local", "studio"] {
            state.fleet.rows.push(Row {
                server: name.into(),
                name: name.into(),
                current: "0.12.19".into(),
                target: "0.12.28".into(),
                state: "checking".into(),
                eligible: false,
                failed: false,
                update_id: None,
                prior_outcome: None,
                attempted: false,
                link: None,
            });
        }
        let reason = "Shina server is not running there. The machine refused the connection.";
        send.send((1, Err(reason.into()), false)).unwrap();
        state.poll_fleet();
        assert_eq!(state.fleet.rows[1].state, format!("skipped: {reason}"));
        assert!(!state.fleet.rows[1].eligible);
        assert!(!state.fleet.rows[1].failed);
    }
    #[test]
    fn old_protocol_version_metadata_still_enables_the_review_and_offline_is_never_eligible() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let mut client = crate::client::Client::new(state.data_dir.clone());
        let mut remote = crate::client::Client::new(state.data_dir.clone());
        *remote.info.lock().unwrap() = json!({"binaryVersion":"0.1.0","protocolMinor":4});
        remote.remote = Some(crate::peer::Link {
            server_id: "old".into(),
            host: "old-machine".into(),
            device_id: "fixture".into(),
            token: "not-real".into(),
            enabled: true,
        });
        client.linked.insert("old".into(), remote);
        state.client = Some(client);
        assert!(state.fleet_available());
        state.open_fleet();
        assert_eq!(state.fleet.rows[1].state, "skipped: offline");
        assert!(!state.fleet.rows[1].eligible);
    }
    #[test]
    fn an_uncertain_response_learns_the_new_outcome_without_retrying_the_request() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.demo_fleet();
        state.fleet.rows.truncate(2);
        let row = &mut state.fleet.rows[1];
        row.server = "remote".into();
        row.attempted = true;
        row.failed = true;
        row.state = "failed: connection lost".into();
        row.prior_outcome = Some("previous-update".into());
        state.machines.views.insert("remote".into(),json!({"updateStatus":{
            "runningVersion":"99.0.1","state":"idle","lastOutcome":{"updateId":"new-update","success":true,"tag":"v99.0.1"}
        }}));
        state.poll_fleet();
        assert_eq!(state.fleet.rows[1].state, "updated to 99.0.1");
        assert_eq!(state.fleet.rows[1].update_id.as_deref(), Some("new-update"));
        assert!(!state.fleet.rows[1].failed);
        assert_eq!(state.fleet.pending_handoffs, 0);
    }
    #[test]
    fn client_poll_applies_fleet_progress_from_the_same_projection_batch() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.demo_fleet();
        state.fleet.rows.truncate(2);
        state.fleet.rows[1].server = "remote".into();
        state.fleet.rows[1].attempted = true;
        state.fleet.rows[1].update_id = Some("same-batch".into());
        let (mut client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, _) = crate::client::Client::test_connected(state.data_dir.clone());
        let mut projection = state.projection();
        projection["updateStatus"] = json!({"runningVersion":"99.0.0","state":"idle","lastOutcome":{"updateId":"same-batch","success":true,"tag":"v99.0.0"}});
        remote.test_snapshot(projection);
        client.linked.insert("remote".into(), remote);
        state.client = Some(client);
        crate::model::poll(&mut state, 0.0, &mut rune::Tx::default());
        assert_eq!(state.fleet.rows[1].state, "updated to 99.0.0");
    }
}
