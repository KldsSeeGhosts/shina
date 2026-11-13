//! Per-machine updates. There is no fleet RPC or caller-selected release.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
};

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
}
impl ServerUpdates {
    pub fn new(directory: &Path) -> Self {
        let outcome = read_outcome(directory);
        let request: Value = std::fs::read(directory.join("updates/request.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let pending = request["updateId"]
            .as_str()
            .filter(|id| outcome.as_ref().is_none_or(|o| o.update_id != *id));
        Self {
            phase: if pending.is_some() {
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
        }
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
            "state":phase,"updateId":self.id,"tag":self.tag,"lastOutcome":self.outcome,"blocker":blocker})
    }
    /// Called only after the accepted receipt and audit row commit.
    pub fn start(&mut self, directory: PathBuf, id: String) {
        self.phase = "checking".into();
        self.id = Some(id.clone());
        self.tag.clear();
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
        if let Some(events) = &self.events {
            while let Ok((phase, detail)) = events.try_recv() {
                if phase == "failed" {
                    let outcome = Outcome {
                        update_id: self.id.clone().unwrap_or_default(),
                        tag: self.tag.clone(),
                        success: false,
                        reason: Some(crate::update::redact(&detail)),
                    };
                    let _ = save_outcome(directory, &outcome);
                    self.outcome = Some(outcome);
                } else if !detail.is_empty() {
                    self.tag = detail;
                }
                self.phase = phase;
            }
        }
        if let Some(outcome) = read_outcome(directory) {
            if self.id.as_deref().is_none_or(|id| id == outcome.update_id) {
                if self.active() {
                    self.phase = if outcome.success { "idle" } else { "failed" }.into();
                }
                self.outcome = Some(outcome);
            }
        }
        if !self.active() && self.host.is_none() {
            self.host = Some(Box::new(InstalledHost));
        }
        before != self.status(None, &Value::Null)
    }
}

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
                let online = remote.commands.connected();
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
                        row.state = if requested {
                            format!("failed: {reason}")
                        } else {
                            format!("skipped: offline, {reason}")
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
