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
pub(crate) const PROBE_INTERVAL: Duration = Duration::from_secs(1);

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
    pub release: crate::update::ReleaseDiscovery,
    release_target: Option<crate::install::InstalledTarget>,
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
            release: Default::default(),
            release_target: self_update_target(directory),
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
            "diagnostic":self.diagnostic,
            "release":self.release.for_target(self.release_target.as_ref())})
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
        self.poll_at(directory, Instant::now())
    }

    pub(crate) fn poll_at(&mut self, directory: &Path, now: Instant) -> bool {
        let before = self.status(None, &Value::Null);
        let release_changed = self.release.poll();
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
        if self
            .last_probe
            .is_some_and(|probed| now.saturating_duration_since(probed) < PROBE_INTERVAL)
        {
            return release_changed || before != self.status(None, &Value::Null);
        }
        self.last_probe = Some(now);
        if let Some(outcome) = read_outcome(directory)
            && (!self.active() || self.id.is_some())
            && self.id.as_deref().is_none_or(|id| id == outcome.update_id)
        {
            self.phase = if outcome.success { "idle" } else { "failed" }.into();
            self.outcome = Some(outcome);
            self.diagnostic = None;
        }
        if self.phase == "checking" && self.id.is_none() {
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
            self.reconcile(directory, now);
        }
        if !self.active() && self.host.is_none() {
            self.host = Some(Box::new(InstalledHost));
        }
        release_changed || before != self.status(None, &Value::Null)
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

/// KSG's `provider-updates-v1` capability, advertised by machines at protocol minor 36.
/// The matching `protocol::PROVIDER_UPDATES` constant is wired by the protocol owner.
use crate::protocol::PROVIDER_UPDATES;
/// An unreceipted `providers.update` may be one of two serialized provider updates at
/// 30 minutes each; confirmation waits up to 65 minutes before the send expires.
const PROVIDER_CONFIRM_MS: i64 = 65 * 60_000;
/// How often a pending `providers.update` is looked up by its exact command id.
const PROVIDER_LOOKUP_INTERVAL: Duration = Duration::from_secs(10);
/// A `providers.refresh` answer stops counting after a minute, so the dialog's
/// "Checking for more..." always ends.
const REFRESH_DEADLINE: Duration = Duration::from_secs(60);
const PROVIDER_CONFIRM_TIMEOUT: &str =
    "Timed out waiting for provider confirmation. Check versions before retrying.";
/// A success result notice behaves like KSG's toast: it clears itself. Failures
/// stay until Retry or Dismiss, like the capsule's settled rows.
const RESULT_NOTICE_DISMISS: Duration = Duration::from_secs(12);

/// KSG `updateEverything.logic`'s run phases. Provider installs on every machine run
/// first, servers follow once they settle, and this app installs last behind a countdown.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RunPhase {
    #[default]
    Idle,
    Providers,
    Servers,
    Countdown,
    Installing,
}

/// One driver update inside a run, KSG's `ServerProviderUpdateState` mirror on this side.
/// `succeeded` and `unchanged` are terminal; only the server's own record with the exact
/// accepted `updateId` settles a run, like the server rows' `updateId` rule.
#[derive(Clone)]
pub struct ProviderRun {
    pub driver: String,
    pub name: String,
    pub target: String,
    pub update_id: Option<String>,
    /// Epoch milliseconds of the send. A receipt lost in transport is never resent; the
    /// exact command is looked up with `command.get` instead.
    pub requested_ms: i64,
    pub outcome: Option<String>,
    pub message: Option<String>,
}
impl ProviderRun {
    fn settled(&self) -> bool {
        self.outcome.is_some()
    }
}

/// A server that accepted its update and restarted before this app did. It survives in
/// `updates/everything.json` so a relaunched app keeps following it without resending.
#[derive(Clone)]
pub struct HandedOff {
    pub target: String,
    pub update_id: String,
    pub started_ms: i64,
}

#[derive(Clone)]
pub struct Row {
    pub server: String,
    pub name: String,
    pub current: String,
    pub target: String,
    pub state: String,
    pub eligible: bool,
    /// An already-current desktop can still have an older local service.
    pub local_server_only: bool,
    /// Read-only hello/update status used to re-plan after release discovery.
    pub probe: Option<Value>,
    pub failed: bool,
    pub update_id: Option<String>,
    pub prior_outcome: Option<String>,
    /// The machine's in-flight `updateStatus.updateId` before our request; a
    /// transport-loss adoption must not claim it.
    pub prior_status_id: Option<String>,
    /// The machine answered the request and refused. A definitive refusal is never
    /// erased by adopting a different attempt seen in the projection.
    pub refused: bool,
    pub attempted: bool,
    pub link: Option<crate::peer::Link>,
    /// Provider display names this machine can one-click update, as the dialog lists them.
    pub providers: Vec<String>,
    /// The providers array from the last `providers.refresh` answer, when the projection
    /// has not caught up yet.
    pub providers_view: Option<Value>,
    /// The machine answers `providers.*` (capability, grant and a connected link).
    pub provider_capable: bool,
    /// The release its server installs, when the server updates in this run.
    pub server_target: Option<String>,
    /// Its server settled on the target; provider failures can still fail the machine.
    pub server_updated: bool,
    pub provider_runs: Vec<ProviderRun>,
    /// "<machine> <Name>: <message>" lines from provider records that were not `succeeded`.
    pub provider_failure: Option<String>,
    pub handed_off: Option<HandedOff>,
    /// Run start for the elapsed readout; servers reset it when the server phase begins.
    pub started_at: Instant,
    /// `Some(true)` is KSG's `done`, `Some(false)` is `failed` (`failed` mirrors it).
    pub result: Option<bool>,
}
impl Row {
    fn updates_providers(&self) -> bool {
        !self.provider_runs.is_empty()
    }
    /// KSG's machine with no server target: it settles once its providers do.
    fn provider_only(&self) -> bool {
        self.server_target.is_none() && self.updates_providers()
    }
}

/// The one notice shown after a run, KSG `getUpdateEverythingToastView` in `sea` or
/// `danger`: a title plus one line per failure and the still-restarting sentence.
#[derive(Clone)]
pub struct ResultNotice {
    pub success: bool,
    pub title: String,
    pub lines: Vec<String>,
}

/// A `providers.update` sent through a connected machine's command channel; the receipt
/// arrives later as an `Event::Receipt` matched by `command_id`.
struct ProviderSend {
    server: String,
    driver: String,
    /// The fleet row this send belongs to; `usize::MAX` for Settings sends.
    row: usize,
    /// The exact `targetVersion` sent; combined with `hash` a looked-up record is
    /// provably our command.
    /// `Command::hash` of the sent method and body.
    hash: String,
    /// Epoch milliseconds of the send; the confirmation deadline counts from it.
    requested_ms: i64,
    /// When the last `command.get` lookup was issued, for throttling.
    lookup_at: Option<Instant>,
}

#[derive(Default)]
pub struct Fleet {
    pub release: crate::update::ReleaseDiscovery,
    pub open: bool,
    pub running: bool,
    pub rows: Vec<Row>,
    events: Option<mpsc::Receiver<(usize, Result<Value, String>, bool)>>,
    sender: Option<mpsc::Sender<(usize, Result<Value, String>, bool)>>,
    pending_handoffs: usize,
    local_started: bool,
    /// KSG `UpdateEverythingRun.localVersion`: the tag this app installs, when it does.
    pub local_version: Option<String>,
    pub phase: RunPhase,
    pub restart_at: Option<Instant>,
    /// `providers.refresh` answers still in flight; the dialog reads "Checking for more...".
    pub refreshing: usize,
    /// Which machines still owe a refresh answer and since when; each entry clears on
    /// its answer, on disconnect or after `REFRESH_DEADLINE`.
    refresh_pending: std::collections::HashMap<String, Instant>,
    provider_sends: Vec<(uuid::Uuid, ProviderSend)>,
    /// Send errors and refused receipts for Settings sends, keyed (server, driver).
    /// Cleared by the next explicit press on that row.
    pub provider_send_errors: std::collections::HashMap<(String, String), String>,
    /// The durable `updates/everything.json` was read (or found absent) once per launch.
    restored: bool,
    wake_deadline: Option<Instant>,
    /// The result notice under the capsule (KSG `showUpdateEverythingResult`).
    pub result_notice: Option<ResultNotice>,
    /// When the notice appeared; a success notice is a toast and clears itself.
    notice_shown: Option<Instant>,
    /// The capsule's anchored popover is open.
    pub popover: bool,
    /// The dialog's "N skipped" disclosure is open.
    pub skipped_open: bool,
    /// Provider pill views dismissed by key until their status changes.
    pub pill_dismissed: std::collections::HashSet<String>,
    /// Only provider results finished after this instant (epoch ms) may show in the pill.
    pill_visible_after: Option<i64>,
    /// The visible success pill and when it appeared, for its 3 s dismissal.
    pill_shown: Option<(String, Instant)>,
    /// The current provider pill view, refreshed by `poll_provider_pill`.
    pub pill: Option<ProviderPillView>,
    /// `Later` was pressed during the countdown; the popover offers "Restart to update".
    pub postponed_local: bool,
}
impl crate::model::State {
    fn discovered_target(&self, status: &Value) -> Result<Option<String>, String> {
        // New services resolve their own platform release. Older services use the
        // controller's metadata-only catalog, never its executable version.
        let own = &status["release"];
        let fallback = self.fleet.release.view();
        let use_own =
            own.is_object() && (own["state"] != "checking" || fallback["state"] != "checked");
        let catalog = if use_own { own.clone() } else { fallback };
        match catalog["state"].as_str() {
            Some("failed") => Err(catalog["error"]
                .as_str()
                .unwrap_or("Release check failed.")
                .into()),
            Some("checked") => {
                if use_own && own["assetAvailable"] == false {
                    return Err("The latest release has no download for this machine.".into());
                }
                // Legacy hello has no OS/architecture. Only offer a fleet release
                // once all supported platforms have actually been published.
                if !use_own
                    && ![
                        "shina.exe",
                        "Shina-macos-aarch64.zip",
                        "Shina-linux-x86_64.tar.gz",
                    ]
                    .iter()
                    .all(|name| {
                        catalog["assets"].as_array().is_some_and(|assets| {
                            assets.iter().any(|asset| {
                                asset["name"] == *name && asset["size"].as_u64().unwrap_or(0) > 0
                            })
                        })
                    })
                {
                    return Err(
                        "The latest release is not published for every platform yet.".into(),
                    );
                }
                catalog["tag"]
                    .as_str()
                    .map(|tag| Some(tag.to_owned()))
                    .ok_or_else(|| "The release check returned no version.".into())
            }
            _ => Ok(None),
        }
    }

    fn plan_fleet_versions(&mut self) -> bool {
        self.plan_fleet_versions_for(env!("CARGO_PKG_VERSION"))
    }
    fn plan_fleet_versions_for(&mut self, app_version: &str) -> bool {
        if self.fleet.running || !self.fleet.open {
            return false;
        }
        let mut changed = false;
        for index in 0..self.fleet.rows.len() {
            let row = &self.fleet.rows[index];
            if row.server != "local" && row.probe.is_none() {
                continue;
            }
            let probe = row.probe.clone().unwrap_or(Value::Null);
            let status = self
                .machines
                .views
                .get(&row.server)
                .map(|view| &view["updateStatus"])
                .filter(|status| status["release"].is_object() || row.server == "local")
                .cloned()
                .unwrap_or_else(|| probe["status"].clone());
            let discovered = self.discovered_target(&status);
            let local = row.server == "local";
            let current = row.current.clone();
            let mut target = String::new();
            let mut eligible = false;
            let mut local_server_only = false;
            let state = match discovered {
                Err(error) => format!("skipped: {error}"),
                Ok(None) => "checking".into(),
                Ok(Some(tag)) => {
                    target = tag;
                    if local {
                        let server_version = status["runningVersion"].as_str().unwrap_or(&current);
                        let app_behind = crate::update::is_newer(&target, app_version);
                        let server_behind = crate::update::is_newer(&target, server_version);
                        if self.client.is_some() && !status["runningVersion"].is_string() {
                            "checking".into()
                        } else if let Some(blocker) = self
                            .update_restart_blocker()
                            .filter(|_| app_behind || server_behind)
                        {
                            format!("skipped: {blocker}")
                        } else if app_behind {
                            if self
                                .updater
                                .prepared_update()
                                .is_some_and(|update| update.tag == target)
                            {
                                eligible = true;
                                "will update app and service".into()
                            } else {
                                self.updater.check_for_newer_prepared(&target);
                                if let Some(error) = self.updater.notice() {
                                    format!("skipped: {error}")
                                } else {
                                    "waiting for desktop download".into()
                                }
                            }
                        } else if server_behind && self.client.is_some() {
                            let hello = self
                                .client
                                .as_ref()
                                .unwrap()
                                .info
                                .lock()
                                .expect("connection info")
                                .clone();
                            if let Some(reason) = skip_reason(true, &hello, &status, &target) {
                                format!("skipped: {reason}")
                            } else {
                                eligible = true;
                                local_server_only = true;
                                "will update local service".into()
                            }
                        } else {
                            "up to date".into()
                        }
                    } else if let Some(reason) =
                        skip_reason(true, &probe["hello"], &status, &target)
                    {
                        format!("skipped: {reason}")
                    } else if crate::update::is_newer(&target, &current) {
                        eligible = true;
                        "will update".into()
                    } else {
                        "up to date".into()
                    }
                }
            };
            let row = &mut self.fleet.rows[index];
            if !target.is_empty() && crate::update::is_newer(&current, &target) {
                target = current;
            }
            changed |= row.target != target
                || row.state != state
                || row.eligible != eligible
                || row.local_server_only != local_server_only;
            row.target = target;
            row.state = state;
            row.eligible = eligible;
            row.local_server_only = local_server_only;
            if local_server_only {
                row.current = status["runningVersion"]
                    .as_str()
                    .unwrap_or(&row.current)
                    .to_owned();
            }
        }
        changed
    }

    /// KSG's `hello.capabilities` + `canUpdate` gate for provider rows on a machine.
    /// Older machines get no provider row and no skip line for providers.
    fn provider_capable(&self, server: &str) -> bool {
        if server == "local" {
            if let Some(client) = &self.client {
                return client.connected_ready()
                    && client.has_capability(PROVIDER_UPDATES)
                    && client.info.lock().expect("connection info")["canUpdate"] == true;
            }
            // Embedded mode has no provider backend status integration: the rows
            // stay visible but one-click updates need the Shina server.
            return false;
        }
        self.machine_connected(server)
            && self
                .client
                .as_ref()
                .is_some_and(|client| client.supports_capability(server, PROVIDER_UPDATES))
            && self
                .client
                .as_ref()
                .and_then(|client| client.linked.get(server))
                .is_some_and(|remote| {
                    remote.info.lock().expect("connection info")["canUpdate"] == true
                })
    }
    /// The providers a machine reports. During a run this is always the live
    /// projection, never the frozen refresh snapshot. For planning, the refresh
    /// snapshot wins only while its `checkedAt` is at least as new as the
    /// projection's for the same server.
    pub fn machine_providers(&self, server: &str) -> Vec<Value> {
        let projection = self
            .machines
            .views
            .get(server)
            .and_then(|view| view["providers"].as_array())
            .cloned()
            .unwrap_or_default();
        if self.fleet.phase != RunPhase::Idle {
            return projection;
        }
        if let Some(row) = self.fleet.rows.iter().find(|row| row.server == server)
            && let Some(list) = row.providers_view.as_ref().and_then(Value::as_array)
        {
            let checked = |providers: &[Value]| {
                providers
                    .iter()
                    .filter_map(|provider| provider["checkedAt"].as_i64())
                    .max()
                    .unwrap_or(0)
            };
            if checked(list) >= checked(&projection) {
                return list.clone();
            }
        }
        projection
    }
    /// Drivers whose update is one click on this machine: behind a known latest with a
    /// proven strategy and no active record, KSG `isProviderUpdateCandidate`.
    /// Each entry is (driver, display name, target version).
    pub fn provider_candidates(&self, server: &str) -> Vec<(String, String, String)> {
        self.machine_providers(server)
            .iter()
            .filter(|provider| {
                provider["installed"] == true
                    && provider["advisory"].as_str() == Some("behind")
                    && provider["strategy"].is_string()
                    && provider["latestVersion"].is_string()
                    && !matches!(
                        provider["update"]["status"].as_str(),
                        Some("queued" | "running")
                    )
            })
            .map(|provider| {
                (
                    provider["driver"].as_str().unwrap_or("").to_owned(),
                    provider["name"]
                        .as_str()
                        .or_else(|| provider["driver"].as_str())
                        .unwrap_or("")
                        .to_owned(),
                    provider["latestVersion"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect()
    }
    /// Every one-click provider update over every connected, capable machine; each
    /// entry is (server, driver, display name, target version).
    pub(crate) fn all_provider_candidates(&self) -> Vec<(String, String, String, String)> {
        let mut machines: Vec<String> = self
            .fleet
            .rows
            .iter()
            .filter(|row| row.provider_capable)
            .map(|row| row.server.clone())
            .collect();
        for server in self.machines.views.keys() {
            if !machines.iter().any(|known| known == server) && self.provider_capable(server) {
                machines.push(server.clone());
            }
        }
        let mut all = Vec::new();
        for server in machines {
            for (driver, name, target) in self.provider_candidates(&server) {
                all.push((server.clone(), driver, name, target));
            }
        }
        all
    }
    /// Machines whose server this run can move to the target release (KSG
    /// `collectServerUpdateTargets` on the client side).
    pub(crate) fn server_update_offers(&self) -> std::collections::BTreeMap<String, String> {
        let mut offers = std::collections::BTreeMap::new();
        if let Some(client) = &self.client {
            for (id, remote) in &client.linked {
                let current = remote.version().unwrap_or_else(|| {
                    self.machines
                        .views
                        .get(id)
                        .and_then(|v| v["updateStatus"]["runningVersion"].as_str())
                        .unwrap_or("")
                        .to_owned()
                });
                let hello = remote.info.lock().expect("connection info").clone();
                let status = self
                    .machines
                    .views
                    .get(id)
                    .map(|view| view["updateStatus"].clone())
                    .unwrap_or(Value::Null);
                let Ok(Some(target)) = self.discovered_target(&status) else {
                    continue;
                };
                if !crate::update::is_newer(&target, &current) {
                    continue;
                }
                let online = remote.commands.connected();
                if skip_reason(online, &hello, &status, &target).is_none() {
                    offers.insert(id.clone(), target);
                }
            }
        }
        // Only connected, eligible linked machines count; a stale cached view for a
        // machine that is not linked right now must not join the summary.
        let local_status = self
            .machines
            .views
            .get("local")
            .map(|view| &view["updateStatus"])
            .unwrap_or(&Value::Null);
        let local_service = self
            .discovered_target(local_status)
            .ok()
            .flatten()
            .filter(|target| {
                !crate::update::is_newer(&target, env!("CARGO_PKG_VERSION"))
                    && local_status["runningVersion"]
                        .as_str()
                        .is_some_and(|version| crate::update::is_newer(&target, version))
                    && self.client.as_ref().is_some_and(|client| {
                        skip_reason(
                            client.commands.connected(),
                            &client.info.lock().expect("connection info"),
                            local_status,
                            &target,
                        )
                        .is_none()
                    })
            });
        if let Some(target) = local_service {
            offers.insert("local".into(), target);
        }
        offers
    }
    /// KSG `describeUpdateEverything`'s button summary, or None when nothing updates:
    /// "Update this app, 2 servers and 3 providers".
    pub fn update_everything_summary(&self) -> Option<String> {
        let local = self.updater.prepared_update().is_some();
        let servers = self.server_update_offers().len();
        let providers = self.all_provider_candidates().len();
        let mut parts = Vec::new();
        if local {
            parts.push("this app".to_owned());
        }
        parts.extend(count_label(servers, "server"));
        parts.extend(count_label(providers, "provider"));
        // The everything button only replaces the local update row when there is
        // work beyond this app; a local-only update keeps `update_row`.
        if servers == 0 && providers == 0 {
            return None;
        }
        Some(format!("Update {}", join_list(&parts)))
    }
    /// The capsule takes over the footer while a run exists (KSG `SidebarUpdateCapsule`).
    pub fn fleet_run_visible(&self) -> bool {
        (!self.fleet.rows.is_empty()
            && matches!(
                self.fleet.phase,
                RunPhase::Providers | RunPhase::Servers | RunPhase::Countdown
            ))
            || self.fleet.phase == RunPhase::Installing
    }
    #[cfg(test)]
    pub fn fleet_available(&self) -> bool {
        self.update_everything_summary().is_some()
    }
    /// KSG `useUpdateEverything`'s open: list every machine, then refresh provider
    /// versions on each connected, capable one while the dialog reads "Checking for
    /// more...".
    pub fn open_fleet(&mut self) {
        if self.fleet.running || self.fleet_run_visible() {
            self.fleet.open = true;
            return;
        }
        let target = String::new();
        let restored = self.fleet.restored;
        let pill_dismissed = std::mem::take(&mut self.fleet.pill_dismissed);
        let pill_visible_after = self.fleet.pill_visible_after.take();
        let pill = self.fleet.pill.take();
        let mut release = std::mem::take(&mut self.fleet.release);
        release.refresh(true);
        self.fleet = Fleet {
            release,
            open: true,
            restored,
            pill_dismissed,
            pill_visible_after,
            pill,
            ..Default::default()
        };
        let local_capable = self.provider_capable("local");
        let local_providers = self
            .provider_candidates("local")
            .into_iter()
            .map(|(_, name, _)| name)
            .collect();
        self.fleet.rows.push(Row {
            server: "local".into(),
            name: "Local".into(),
            current: env!("CARGO_PKG_VERSION").into(),
            target: target.clone(),
            state: "checking".into(),
            provider_capable: local_capable,
            providers: local_providers,
            ..Default::default()
        });
        let (send, receive) = mpsc::channel();
        self.fleet.events = Some(receive);
        self.fleet.sender = Some(send.clone());
        if local_capable && self.client.is_some() {
            self.fleet
                .refresh_pending
                .insert("local".into(), Instant::now());
            self.fleet.refreshing = self.fleet.refresh_pending.len();
            self.refresh_providers("local");
        }
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
                let capable = online && self.provider_capable(id);
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
                    link: Some(link.clone()),
                    provider_capable: capable,
                    ..Default::default()
                });
                if !online {
                    continue;
                }
                if capable {
                    self.fleet
                        .refresh_pending
                        .insert(id.clone(), Instant::now());
                    self.fleet.refreshing = self.fleet.refresh_pending.len();
                }
                let send = send.clone();
                let wake = self.wake.clone();
                let target = target.clone();
                std::thread::spawn(move || {
                    let result = tokio::runtime::Runtime::new().map_err(|e| e.to_string()).and_then(|rt| rt.block_on(async {
                        let hello = crate::client::remote_rpc(&link, "hello", json!({})).await?;
                        let capable = hello["canUpdate"] == true
                            && hello["capabilities"].as_array().is_some_and(|caps| {
                                caps.iter().any(|cap| cap == PROVIDER_UPDATES)
                            });
                        let status = if hello["canUpdate"] == true && hello["protocolMinor"].as_u64().unwrap_or(0) >= 6 {
                            crate::client::remote_rpc(&link, "server.updateStatus", json!({"refresh":true})).await?
                        } else { Value::Null };
                        let skipped = skip_reason(true, &hello, &status, &target);
                        let providers = if capable {
                            crate::client::remote_rpc(&link, "providers.refresh", json!({}))
                                .await
                                .ok()
                                .and_then(|value| value["providers"].as_array().cloned())
                                .unwrap_or_default()
                        } else { Vec::new() };
                        Ok(json!({"current":hello["binaryVersion"],"hello":hello,"skip":skipped,"status":status,
                            "providerCapable":capable,"providers":providers}))
                    }));
                    let _ = send.send((index, result, false));
                    wake();
                });
            }
        }
        self.plan_fleet_versions();
    }
    /// An unreceipted `providers.refresh` for one machine. The answer lands on
    /// `fleet_rpc`; the next projection carries the same statuses regardless.
    fn refresh_providers(&self, server: &str) {
        let Some(client) = &self.client else {
            return;
        };
        let target = if server == "local" {
            Some(client)
        } else {
            client.linked.get(server)
        };
        if let Some(target) = target {
            target.call_rpc(
                format!("fleet:refresh:{server}"),
                "providers.refresh",
                json!({}),
            );
        }
    }
    /// An unreceipted `server.updateStatus` refresh for one machine. The answer is
    /// consumed by `fleet_rpc`; the refreshed status reaches the UI on the next
    /// projection either way.
    fn refresh_update_status(&self, server: &str) {
        let Some(client) = &self.client else {
            return;
        };
        let target = if server == "local" {
            Some(client)
        } else {
            client.linked.get(server)
        };
        if let Some(target) = target {
            target.call_rpc(
                format!("fleet:update-status:{server}"),
                "server.updateStatus",
                json!({"refresh": true}),
            );
        }
    }
    /// KSG `SidebarUpdatePill`'s idle press: one button checks the app release,
    /// this profile's release catalog and every connected machine's providers and
    /// server update status. Everything is quiet; results surface through the
    /// usual pill, button and dialog rows.
    pub fn check_for_updates(&mut self) {
        if self.demo {
            return;
        }
        self.updater.retry();
        self.fleet.release.refresh(true);
        let mut servers = vec!["local".to_owned()];
        if let Some(client) = &self.client {
            servers.extend(client.linked.keys().cloned());
        }
        for server in servers {
            if !self.machine_enabled(&server) || !self.machine_connected(&server) {
                continue;
            }
            self.refresh_providers(&server);
            self.refresh_update_status(&server);
        }
    }
    /// A receipted `providers.update` for one driver on one machine. The receipt lands
    /// on `fleet_receipt` matched by `command_id`; a lost receipt is never resent, the
    /// run adopts the record the projection reports instead.
    fn send_provider_update(
        &mut self,
        server: &str,
        driver: &str,
        row: usize,
        target: &str,
    ) -> Result<(), String> {
        let params = json!({"driver":driver,"targetVersion":target});
        let hash = crate::protocol::Command::new("providers.update", params.clone()).hash();
        let id = self
            .client
            .as_ref()
            .and_then(|client| client.commands_for(server))
            .ok_or_else(|| "This machine is disconnected.".to_owned())
            .and_then(|commands| commands.send("providers.update", params))?;
        self.fleet.provider_sends.push((
            id,
            ProviderSend {
                server: server.into(),
                driver: driver.into(),
                row,
                hash,
                requested_ms: crate::lifecycle::now() as i64,
                lookup_at: None,
            },
        ));
        Ok(())
    }
    /// A `providers.update` for this driver is still waiting on its exact receipt or
    /// the confirmation deadline; the Settings row stays gated until then.
    pub fn provider_send_pending(&self, server: &str, driver: &str) -> bool {
        self.fleet
            .provider_sends
            .iter()
            .any(|(_, send)| send.server == server && send.driver == driver)
    }
    /// The Settings "Update" action outside a fleet run; the pill and the row show the
    /// outcome from the projection, no run tracks it.
    pub fn update_provider(&mut self, server: &str, driver: &str) {
        // An explicit press clears the last error; an unreceipted send still gates a
        // duplicate until its exact receipt or the confirmation deadline.
        self.fleet
            .provider_send_errors
            .remove(&(server.to_owned(), driver.to_owned()));
        if self.provider_send_pending(server, driver) {
            return;
        }
        let Some((_, _, target)) = self
            .provider_candidates(server)
            .into_iter()
            .find(|(d, _, _)| d == driver)
        else {
            return;
        };
        let row = self
            .fleet
            .rows
            .iter()
            .position(|row| row.server == server)
            .unwrap_or(usize::MAX);
        if let Err(error) = self.send_provider_update(server, driver, row, &target) {
            self.fleet
                .provider_send_errors
                .insert((server.into(), driver.into()), error);
        }
    }
    /// Every eligible provider update on every capable machine, for Settings' "Update
    /// all" (KSG `ProviderUpdatesAction`).
    pub fn update_all_providers(&mut self) {
        for (server, driver, _, target) in self.all_provider_candidates() {
            if self.provider_send_pending(&server, &driver) {
                continue;
            }
            let row = self
                .fleet
                .rows
                .iter()
                .position(|row| row.server == server)
                .unwrap_or(usize::MAX);
            if let Err(error) = self.send_provider_update(&server, &driver, row, &target) {
                self.fleet
                    .provider_send_errors
                    .insert((server, driver), error);
            }
        }
    }
    /// Receipts for `providers.*` reach here from the client event dispatch. Returns
    /// true when the command belonged to the fleet, so the dispatch can skip it.
    /// Root wires one line in `client_state`'s `Event::Receipt` handling:
    /// `if self.fleet_receipt(&server, &command, &value) { continue; }` (or
    /// `changed = true` and continue).
    pub fn fleet_receipt(
        &mut self,
        server: &str,
        command: &crate::protocol::Command,
        value: &Value,
    ) -> bool {
        match command.method.as_str() {
            "providers.update" => {
                if let Some(index) = self.fleet.provider_sends.iter().position(|(id, send)| {
                    *id == command.command_id
                        && send.server == server
                        && send.hash == command.hash()
                }) {
                    let (_, send) = self.fleet.provider_sends.remove(index);
                    self.provider_receipt(&send, value);
                }
                true
            }
            "providers.refresh" => {
                self.fleet.refresh_pending.remove(server);
                self.fleet.refreshing = self.fleet.refresh_pending.len();
                self.record_providers(server, &value["result"]["providers"]);
                true
            }
            _ => false,
        }
    }
    /// `Event::Rpc` answers whose tag starts `fleet:` (root dispatches the prefix
    /// here). `providers.refresh` is an unreceipted read, so its answer lands on this
    /// path. Returns true when the tag belonged to the fleet.
    /// Root wires one arm in `client_state`'s event loop:
    /// `crate::client::Event::Rpc { tag, result } if self.fleet_rpc(&server, tag, &result) => {}`.
    pub fn fleet_rpc(&mut self, server: &str, tag: &str, result: &Result<Value, String>) -> bool {
        let Some(kind) = tag.strip_prefix("fleet:") else {
            return false;
        };
        if kind.starts_with("refresh:") {
            self.fleet.refresh_pending.remove(server);
            self.fleet.refreshing = self.fleet.refresh_pending.len();
            if let Ok(value) = result {
                self.record_providers(server, &value["providers"]);
            }
            return true;
        }
        // `fleet:update-status:<server>` answers a manual "Check for updates" probe;
        // the refreshed status arrives on the next projection instead.
        if kind.starts_with("update-status:") {
            return true;
        }
        // `fleet:receipt:<uuid>` answers a read-only `command.get` for a send whose
        // live receipt was lost. The stored receipt must name the exact command id
        // and carry the hash of the method and body we sent; anything else leaves
        // the send uncertain until its confirmation deadline.
        if let Some(id) = kind.strip_prefix("receipt:") {
            let Ok(command_id) = uuid::Uuid::parse_str(id) else {
                return true;
            };
            let Some(index) = self
                .fleet
                .provider_sends
                .iter()
                .position(|(send_id, _)| *send_id == command_id)
            else {
                return true;
            };
            if let Ok(receipt) = result {
                let send = &self.fleet.provider_sends[index].1;
                let ours = send.server == server
                    && receipt["commandId"].as_str() == Some(id)
                    && receipt["hash"].as_str() == Some(send.hash.as_str());
                let status = receipt["status"].as_str().unwrap_or("");
                if ours && matches!(status, "accepted" | "rejected" | "interrupted") {
                    let (_, send) = self.fleet.provider_sends.remove(index);
                    self.provider_receipt(&send, receipt);
                }
            }
            return true;
        }
        false
    }
    /// The freshest provider list for a row: a `providers.refresh` answer until the
    /// projection carries the same array.
    fn record_providers(&mut self, server: &str, providers: &Value) {
        let Some(list) = providers.as_array() else {
            return;
        };
        let names: Vec<String> = list
            .iter()
            .filter(|provider| {
                provider["installed"] == true
                    && provider["advisory"].as_str() == Some("behind")
                    && provider["strategy"].is_string()
                    && provider["latestVersion"].is_string()
                    && !matches!(
                        provider["update"]["status"].as_str(),
                        Some("queued" | "running")
                    )
            })
            .map(|provider| {
                provider["name"]
                    .as_str()
                    .or_else(|| provider["driver"].as_str())
                    .unwrap_or("")
                    .to_owned()
            })
            .collect();
        if let Some(row) = self.fleet.rows.iter_mut().find(|row| row.server == server) {
            row.providers_view = Some(Value::Array(list.clone()));
            row.providers = names;
        }
    }
    /// A `providers.update` receipt: the accepted id arms the run's projection watch,
    /// a refusal settles the driver at once.
    fn provider_receipt(&mut self, send: &ProviderSend, value: &Value) {
        let receipt = if value["result"]["status"].is_string() {
            &value["result"]
        } else {
            value
        };
        let accepted_id = receipt["result"]["updateId"]
            .as_str()
            .filter(|_| {
                receipt["status"] == "accepted" && receipt["result"]["updateAccepted"] == true
            })
            .map(str::to_owned);
        let refused = if accepted_id.is_none() {
            Some(
                value["error"]["message"]
                    .as_str()
                    .or_else(|| value["result"]["error"]["message"].as_str())
                    .or_else(|| value["result"]["error"].as_str())
                    .unwrap_or("Update refused.")
                    .to_owned(),
            )
        } else {
            None
        };
        // Settings sends own no run row; their refusal lands inline on the row.
        if send.row == usize::MAX {
            let key = (send.server.clone(), send.driver.clone());
            match refused {
                Some(reason) => {
                    self.fleet.provider_send_errors.insert(key, reason);
                }
                None => {
                    self.fleet.provider_send_errors.remove(&key);
                }
            }
            return;
        }
        let Some(row) = self.fleet.rows.get_mut(send.row) else {
            return;
        };
        let Some(run) = row
            .provider_runs
            .iter_mut()
            .find(|run| run.driver == send.driver && !run.settled())
        else {
            return;
        };
        match accepted_id {
            Some(id) => run.update_id = Some(id),
            None => {
                run.outcome = Some("failed".into());
                run.message = refused;
            }
        }
    }
    /// KSG `runUpdateEverything`'s confirm. The dialog closes and the capsule takes
    /// over; providers run on every machine first.
    pub fn confirm_fleet(&mut self) {
        if self.demo {
            return;
        }
        let now = Instant::now();
        self.fleet.running = true;
        self.fleet.open = false;
        self.fleet.local_started = false;
        self.fleet.pending_handoffs = 0;
        self.fleet.local_version = if self
            .fleet
            .rows
            .first()
            .is_some_and(|r| r.server == "local" && r.eligible && !r.local_server_only)
        {
            self.updater.prepared_update().map(|u| u.tag.clone())
        } else {
            None
        };
        self.fleet.restart_at = None;
        self.fleet.result_notice = None;
        self.fleet.notice_shown = None;
        // Phase Providers: every eligible (machine, driver) in parallel.
        let started_ms = crate::lifecycle::now() as i64;
        let candidates: Vec<Vec<(String, String, String)>> = self
            .fleet
            .rows
            .iter()
            .map(|row| {
                if row.provider_capable {
                    self.provider_candidates(&row.server)
                } else {
                    Vec::new()
                }
                .into_iter()
                .collect()
            })
            .collect();
        for (index, row) in self.fleet.rows.iter_mut().enumerate() {
            row.started_at = now;
            row.provider_runs = candidates[index]
                .iter()
                .map(|(driver, name, target)| ProviderRun {
                    driver: driver.clone(),
                    name: name.clone(),
                    target: target.clone(),
                    update_id: None,
                    requested_ms: started_ms,
                    outcome: None,
                    message: None,
                })
                .collect();
            row.server_target = if (row.server != "local" || row.local_server_only) && row.eligible
            {
                Some(row.target.clone())
            } else {
                None
            };
        }
        // The run tracks participating machines only (KSG `run.machines`): the local
        // row when it installs or updates providers, servers, and provider-only
        // machines. Skipped and up-to-date rows leave with the dialog.
        let local_version = self.fleet.local_version.is_some();
        self.fleet.rows.retain(|row| {
            row.server_target.is_some()
                || !row.provider_runs.is_empty()
                || (local_version && row.server == "local")
        });
        self.fleet.phase = if self
            .fleet
            .rows
            .iter()
            .any(|row| !row.provider_runs.is_empty())
        {
            RunPhase::Providers
        } else {
            RunPhase::Servers
        };
        let mut sends = Vec::new();
        for (index, row) in self.fleet.rows.iter().enumerate() {
            for run in &row.provider_runs {
                sends.push((
                    row.server.clone(),
                    run.driver.clone(),
                    index,
                    run.target.clone(),
                ));
            }
        }
        for (server, driver, index, target) in sends {
            if let Err(reason) = self.send_provider_update(&server, &driver, index, &target)
                && let Some(run) = self.fleet.rows[index]
                    .provider_runs
                    .iter_mut()
                    .find(|run| run.driver == driver && !run.settled())
            {
                run.outcome = Some("failed".into());
                run.message = Some(reason);
            }
        }
        if self.fleet.phase == RunPhase::Servers {
            self.start_server_phase();
        }
    }
    /// Phase Servers: each server waits only on the provider phase, not other servers
    /// (KSG `runUpdateEverything`'s sequencing; there is no v1 staging).
    fn start_server_phase(&mut self) {
        let now = Instant::now();
        let indices: Vec<_> = self
            .fleet
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.server_target.is_some() && r.result.is_none())
            .map(|(i, _)| i)
            .collect();
        for index in indices {
            self.fleet.rows[index].started_at = now;
            self.start_fleet_row(index);
        }
    }
    pub fn start_fleet_row(&mut self, index: usize) {
        if self.fleet.rows[index].server == "local" && !self.fleet.rows[index].local_server_only {
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
        // A server that already runs the target settles without a request (KSG's retry
        // of a machine whose host rejects a same-version update).
        if let Some(target) = self.fleet.rows[index].server_target.clone()
            && self.machine_runs_version(&self.fleet.rows[index].server, &target)
        {
            self.fleet.rows[index].result = Some(true);
            self.fleet.rows[index].server_updated = true;
            self.fleet.rows[index].failed = false;
            self.fleet.rows[index].state = format!("updated to {target}");
            return;
        }
        let local = self.fleet.rows[index].local_server_only;
        let link = self.fleet.rows[index].link.clone();
        if !local && link.is_none() {
            return;
        }
        let directory = self.data_dir.clone();
        let Some(send) = self.fleet.sender.clone() else {
            return;
        };
        self.fleet.pending_handoffs += 1;
        self.fleet.rows[index].state = "downloading".into();
        self.fleet.rows[index].failed = false;
        self.fleet.rows[index].result = None;
        self.fleet.rows[index].update_id = None;
        self.fleet.rows[index].attempted = true;
        let status_before = self
            .machines
            .views
            .get(&self.fleet.rows[index].server)
            .map(|view| view["updateStatus"].clone());
        self.fleet.rows[index].prior_outcome = status_before
            .as_ref()
            .and_then(|status| status["lastOutcome"]["updateId"].as_str())
            .map(str::to_owned);
        self.fleet.rows[index].prior_status_id = status_before
            .as_ref()
            .and_then(|status| status["updateId"].as_str())
            .map(str::to_owned);
        self.fleet.rows[index].refused = false;
        self.fleet.rows[index].eligible = false;
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let result = tokio::runtime::Runtime::new()
                .map_err(|e| e.to_string())
                .and_then(|rt| {
                    rt.block_on(async {
                        if local {
                            crate::client::local_rpc(&directory, "server.requestUpdate", json!({}))
                                .await
                        } else {
                            crate::client::remote_rpc(
                                link.as_ref().unwrap(),
                                "server.requestUpdate",
                                json!({}),
                            )
                            .await
                        }
                    })
                });
            let _ = send.send((index, result, true));
            wake();
        });
    }
    /// The machine's server already runs `target` (KSG `runsVersion`).
    fn machine_runs_version(&self, server: &str, target: &str) -> bool {
        let running = self
            .machines
            .views
            .get(server)
            .and_then(|view| view["updateStatus"]["runningVersion"].as_str());
        running
            .is_some_and(|running| !running.is_empty() && !crate::update::is_newer(target, running))
    }
    /// `Later` cancels the countdown; the capsule popover then offers "Restart to
    /// update" (KSG `postponeUpdateRestart` + `onRestart`).
    pub fn postpone_update_restart(&mut self) {
        if self.fleet.phase == RunPhase::Countdown {
            self.fleet.phase = RunPhase::Servers;
            self.fleet.restart_at = None;
            self.fleet.local_version = None;
            self.fleet.postponed_local = true;
            // A local row that only waited to install leaves the run without being
            // counted updated; settled provider results and failures stay. The
            // postponed restart is still offered while the row is gone.
            self.fleet.rows.retain(|row| {
                row.server != "local" || row.result.is_some() || !row.provider_runs.is_empty()
            });
            if self.fleet.rows.is_empty() {
                self.fleet.running = false;
                self.fleet.phase = RunPhase::Idle;
            }
        }
    }
    /// The popover's "Restart to update" after a postponed countdown.
    pub fn restart_postponed_update(&mut self) {
        if self.fleet.local_version.is_none()
            && self.update_restart_blocker().is_none()
            && self.updater.prepared_update().is_some()
        {
            self.fleet.local_version = self.updater.prepared_update().map(|u| u.tag.clone());
            self.fleet.postponed_local = false;
            self.install_local_now();
        }
    }
    /// Countdown zero: the durable result lands first, then the restart request. A
    /// refused restart deletes the file in `record_fleet_restart_result`.
    fn install_local_now(&mut self) {
        if let Err(error) = self.save_update_everything() {
            self.record_local_failure(format!(
                "This app: Could not save update recovery state: {error}"
            ));
            self.fleet.phase = RunPhase::Servers;
            self.fleet.local_version = None;
            self.fleet.restart_at = None;
            return;
        }
        self.fleet.phase = RunPhase::Installing;
        self.fleet.restart_at = None;
        self.restart_for_update = true;
    }
    /// `prepared_exit` refused the restart the capsule asked for. Root wires one line
    /// in `client_state`'s restart check, after `restart_notice` is set:
    /// `self.record_fleet_restart_result(&error)`. A coordinated-restart "error" is a
    /// pending state, not a refusal.
    pub fn record_fleet_restart_result(&mut self, error: &str) {
        let service_restart =
            self.fleet.rows.iter().any(|row| {
                row.server == "local" && row.local_server_only && row.handed_off.is_some()
            });
        if (self.fleet.phase != RunPhase::Installing && !service_restart)
            || error.starts_with("Coordinated restart requested")
        {
            return;
        }
        if let Err(error) = std::fs::remove_file(self.data_dir.join("updates/everything.json"))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            self.storage_error = Some(format!(
                "Could not clear the cancelled update's recovery record: {error}"
            ));
        }
        self.record_local_failure(format!("This app: {error}"));
        if service_restart
            && let Some(row) = self.fleet.rows.iter_mut().find(|row| row.server == "local")
        {
            row.handed_off = None;
        }
        self.fleet.phase = RunPhase::Servers;
        self.fleet.local_version = None;
    }
    fn record_local_failure(&mut self, failure: String) {
        if let Some(row) = self.fleet.rows.first_mut()
            && row.server == "local"
        {
            row.result = Some(false);
            row.failed = true;
            row.state = format!("failed: {}", failure.trim_start_matches("This app: "));
        }
        if let Some(notice) = &mut self.fleet.result_notice {
            notice.lines.push(failure);
            notice.success = false;
        } else {
            self.show_result_notice(ResultNotice {
                success: false,
                title: "Update failed".into(),
                lines: vec![failure],
            });
        }
    }
    /// The durable `<data>/updates/everything.json`, written right before this app's
    /// restart and read once at the next launch (KSG `saveUpdateEverythingResult`).
    fn save_update_everything(&self) -> Result<(), String> {
        let updated_servers = self
            .fleet
            .rows
            .iter()
            .filter(|row| row.server_updated)
            .count();
        let updated_providers = self
            .fleet
            .rows
            .iter()
            .flat_map(|row| &row.provider_runs)
            .filter(|run| run.outcome.as_deref() == Some("succeeded"))
            .count();
        let failures = self.run_failures();
        let handed_off: Vec<Value> = self
            .fleet
            .rows
            .iter()
            .filter(|row| row.result.is_none() || row.state == "restarting")
            .filter_map(|row| {
                let handoff = row.handed_off.as_ref()?;
                Some(json!({"server":row.server,"label":row.name,
                    "targetVersion":handoff.target,"updateId":handoff.update_id,
                    "startedAt":handoff.started_ms}))
            })
            .collect();
        let saved = json!({"savedAt":crate::lifecycle::now(),
            "localVersion":self.fleet.local_version,
            "updatedServers":updated_servers,"updatedProviders":updated_providers,
            "failures":failures,"handedOff":handed_off});
        let path = self.data_dir.join("updates/everything.json");
        std::fs::create_dir_all(path.parent().unwrap_or(&self.data_dir))
            .map_err(|e| e.to_string())?;
        crate::store::write_atomic(&path, saved.to_string().as_bytes()).map_err(|e| e.to_string())
    }
    pub fn prepare_fleet_service_restart(&mut self) -> Result<(), String> {
        let Some(row) = self.fleet.rows.iter_mut().find(|row| {
            row.server == "local" && row.local_server_only && row.attempted && row.result.is_none()
        }) else {
            return Ok(());
        };
        let id = row
            .update_id
            .clone()
            .ok_or("The local service update receipt has not arrived. Restart cancelled.")?;
        row.handed_off = Some(HandedOff {
            target: row.target.clone(),
            update_id: id,
            started_ms: crate::lifecycle::now() as i64,
        });
        self.save_update_everything()
    }
    /// One failure line per machine/provider result, for the saved file and the notice.
    fn run_failures(&self) -> Vec<String> {
        let mut failures = Vec::new();
        for row in &self.fleet.rows {
            if let Some(failure) = &row.provider_failure {
                failures.extend(failure.lines().map(str::to_owned));
            }
            if row.result == Some(false)
                && let Some(reason) = row.state.strip_prefix("failed: ")
                && Some(reason) != row.provider_failure.as_deref()
            {
                let name = if row.server == "local" {
                    "This app"
                } else {
                    &row.name
                };
                failures.push(format!("{name}: {reason}"));
            }
        }
        failures
    }
    /// Read and delete `everything.json` once per launch: show the saved result, then
    /// rebuild a Servers-phase run that follows each handed-off machine until it reports
    /// the target version, its outcome, or 32 minutes (KSG `followHandedOffServers`).
    fn restore_update_everything(&mut self) {
        self.fleet.restored = true;
        let path = self.data_dir.join("updates/everything.json");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let _ = std::fs::remove_file(&path);
        let Ok(saved) = serde_json::from_slice::<Value>(&bytes) else {
            return;
        };
        let now_ms = crate::lifecycle::now() as i64;
        let saved_ms = saved["savedAt"].as_i64().unwrap_or(0);
        if now_ms.saturating_sub(saved_ms) > 10 * 60 * 1000 {
            return;
        }
        let mut failures: Vec<String> = saved["failures"]
            .as_array()
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|line| line.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let local_version = saved["localVersion"].as_str();
        if let Some(version) = local_version
            && env!("CARGO_PKG_VERSION") != version
        {
            failures.insert(
                0,
                format!(
                    "Update did not apply. Shina is still on {}. Install {version} again \
                     from the update button.",
                    env!("CARGO_PKG_VERSION")
                ),
            );
        }
        let handed_off = saved["handedOff"].as_array().cloned().unwrap_or_default();
        let restarting: Vec<String> = handed_off
            .iter()
            .filter_map(|server| server["label"].as_str().map(str::to_owned))
            .collect();
        let updated_servers = saved["updatedServers"].as_u64().unwrap_or(0) as usize;
        let updated_providers = saved["updatedProviders"].as_u64().unwrap_or(0) as usize;
        let local_updated = local_version.is_some_and(|v| v == env!("CARGO_PKG_VERSION"));
        if local_updated || updated_servers + updated_providers > 0 || !failures.is_empty() {
            let mut notice =
                result_notice(local_updated, updated_servers, updated_providers, failures);
            if !restarting.is_empty() {
                notice.lines.push(format!(
                    "{} {} still restarting.",
                    join_list(&restarting),
                    if restarting.len() == 1 { "is" } else { "are" }
                ));
            }
            self.show_result_notice(notice);
        }
        if handed_off.is_empty() {
            return;
        }
        self.fleet.running = true;
        self.fleet.phase = RunPhase::Servers;
        for entry in &handed_off {
            let server = entry["server"].as_str().unwrap_or("").to_owned();
            let started_ms = entry["startedAt"].as_i64().unwrap_or(now_ms);
            let elapsed = now_ms.saturating_sub(started_ms).max(0) as u64;
            let name = entry["label"]
                .as_str()
                .unwrap_or(server.as_str())
                .to_owned();
            let target = entry["targetVersion"].as_str().unwrap_or("").to_owned();
            self.fleet.rows.push(Row {
                server,
                name,
                target: target.clone(),
                state: "restarting".into(),
                update_id: entry["updateId"].as_str().map(str::to_owned),
                handed_off: Some(HandedOff {
                    target,
                    update_id: entry["updateId"].as_str().unwrap_or("").to_owned(),
                    started_ms,
                }),
                started_at: Instant::now() - Duration::from_millis(elapsed.min(600_000)),
                ..Default::default()
            });
        }
    }
    /// KSG `resolveUpdateRunMachineStatus`: result if set, then restarting for a handed
    /// off machine, the provider phase word, the server's own state, else waiting.
    pub fn run_status(&self, row: &Row) -> &'static str {
        match row.result {
            Some(true) => return "done",
            Some(false) => return "failed",
            None => {}
        }
        if row.failed {
            return "failed";
        }
        if row.handed_off.is_some() || row.state == "restarting" {
            return "restarting";
        }
        if self.fleet.phase == RunPhase::Providers && row.updates_providers() {
            return "providers";
        }
        match row.state.as_str() {
            "downloading" | "checking" | "ready" => "downloading",
            _ => "waiting",
        }
    }
    /// The fleet progress label and action, based on KSG `describeUpdateCapsule`.
    pub fn describe_capsule(&self) -> CapsuleView {
        let now = Instant::now();
        if self.fleet.phase == RunPhase::Countdown
            && let Some(at) = self.fleet.restart_at
        {
            let seconds = (at.saturating_duration_since(now).as_secs() + 1).min(5);
            return CapsuleView {
                label: format!("Restart in {seconds}s"),
                elapsed: None,
                action: CapsuleAction::Later,
            };
        }
        if self.fleet.phase == RunPhase::Installing {
            return CapsuleView {
                label: "Restarting...".into(),
                elapsed: None,
                action: CapsuleAction::None,
            };
        }
        let statuses: Vec<(&Row, &'static str)> = self
            .fleet
            .rows
            .iter()
            .map(|row| (row, self.run_status(row)))
            .collect();
        let active: Vec<(&Row, &'static str)> = statuses
            .iter()
            .copied()
            .filter(|(_, status)| matches!(*status, "providers" | "downloading" | "restarting"))
            .collect();
        if let Some((row, _)) = active
            .iter()
            .min_by_key(|(row, _)| row.started_at)
            .filter(|(row, _)| now.duration_since(row.started_at) >= Duration::from_secs(180))
        {
            return CapsuleView {
                label: row.name.clone(),
                elapsed: Some(format_update_elapsed(now.duration_since(row.started_at))),
                action: CapsuleAction::None,
            };
        }
        let done = statuses
            .iter()
            .filter(|(_, status)| *status == "done")
            .count();
        if statuses
            .iter()
            .any(|(_, status)| !matches!(*status, "done" | "failed"))
        {
            return CapsuleView {
                label: format!("{done} of {} updated", self.fleet.rows.len()),
                elapsed: None,
                action: CapsuleAction::None,
            };
        }
        if self.fleet.local_version.is_some()
            && let crate::update::Status::Downloading { .. } = self.updater.status()
        {
            // The updater exposes no real progress, so the capsule says only
            // "Downloading" rather than an invented percentage.
            return CapsuleView {
                label: "Downloading".into(),
                elapsed: None,
                action: CapsuleAction::None,
            };
        }
        let failed: Vec<&Row> = statuses
            .iter()
            .filter(|(_, status)| *status == "failed")
            .map(|(row, _)| *row)
            .collect();
        if !failed.is_empty() {
            let label = if failed.len() == 1 {
                format!("{} failed", failed[0].name)
            } else {
                format!("{} failed", failed.len())
            };
            return CapsuleView {
                label,
                elapsed: None,
                action: if failed
                    .iter()
                    .any(|row| row.server_target.is_some() && !row.server_updated)
                {
                    CapsuleAction::Retry
                } else {
                    CapsuleAction::Dismiss
                },
            };
        }
        CapsuleView {
            label: format!("{done} of {} updated", self.fleet.rows.len()),
            elapsed: None,
            action: CapsuleAction::None,
        }
    }
    /// The capsule action button: Later cancels the countdown, Retry resends only
    /// failed servers (providers are retried from Settings, as KSG does), Dismiss
    /// drops settled machines.
    pub fn capsule_action(&mut self) {
        match self.describe_capsule().action {
            CapsuleAction::Later => self.postpone_update_restart(),
            CapsuleAction::Retry => {
                let retry: Vec<usize> = self
                    .fleet
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| {
                        row.result == Some(false)
                            && row.server_target.is_some()
                            && !row.server_updated
                    })
                    .map(|(index, _)| index)
                    .collect();
                for index in retry {
                    self.fleet.rows[index].result = None;
                    self.fleet.rows[index].failed = false;
                    self.start_fleet_row(index);
                }
            }
            CapsuleAction::Dismiss => self.dismiss_settled_machines(),
            CapsuleAction::None => {}
        }
    }
    /// KSG `dismissSettledUpdateRunMachines`; the capsule disappears when none remain
    /// and no local install is pending.
    pub fn dismiss_settled_machines(&mut self) {
        self.fleet.rows.retain(|row| row.result.is_none());
        if self.fleet.rows.is_empty() && self.fleet.local_version.is_none() {
            self.fleet.running = false;
            self.fleet.phase = RunPhase::Idle;
        }
        self.fleet.popover = false;
    }
    /// The local row's popover line, KSG `localStatusLabel`.
    pub fn capsule_local_status(&self) -> Option<String> {
        if self.fleet.local_version.is_none() && !self.fleet.postponed_local {
            return None;
        }
        if self.fleet.phase == RunPhase::Countdown
            && let Some(at) = self.fleet.restart_at
        {
            let seconds = (at.saturating_duration_since(Instant::now()).as_secs() + 1).min(5);
            return Some(format!("restarts in {seconds}s"));
        }
        if self.fleet.phase == RunPhase::Installing {
            return Some("restarting".into());
        }
        if self.fleet.local_version.is_some()
            && let crate::update::Status::Downloading { .. } = self.updater.status()
        {
            return Some("downloading".into());
        }
        Some("restarts after the servers".into())
    }
    /// Recompute the provider pill from the local machine's providers (KSG reads the
    /// primary server only). Runs in `poll_fleet`, so the 3 s success dismissal needs
    /// no extra timer.
    fn poll_provider_pill(&mut self) {
        if self.fleet.pill_visible_after.is_none() {
            let checked = self
                .machine_providers("local")
                .iter()
                .filter_map(|provider| provider["checkedAt"].as_i64())
                .max();
            self.fleet.pill_visible_after = checked.or(Some(crate::lifecycle::now() as i64));
        }
        let visible_after = self.fleet.pill_visible_after.unwrap_or(0);
        let providers = self.machine_providers("local");
        let dismissed = |key: &str| self.fleet.pill_dismissed.contains(key);
        let names = |list: &[&Value]| -> Vec<String> {
            list.iter()
                .map(|provider| {
                    provider["name"]
                        .as_str()
                        .or_else(|| provider["driver"].as_str())
                        .unwrap_or("")
                        .to_owned()
                })
                .collect()
        };
        let key_of = |prefix: &str, list: &[&Value]| -> String {
            let mut keys: Vec<String> = list
                .iter()
                .map(|provider| {
                    format!(
                        "{}:{}:{}",
                        provider["driver"].as_str().unwrap_or(""),
                        provider["update"]["finishedAt"].as_i64().unwrap_or(0),
                        provider["update"]["message"].as_str().unwrap_or("")
                    )
                })
                .collect();
            keys.sort();
            format!("{prefix}:{}", keys.join("|"))
        };
        let active: Vec<&Value> = providers
            .iter()
            .filter(|provider| {
                matches!(
                    provider["update"]["status"].as_str(),
                    Some("queued" | "running")
                )
            })
            .collect();
        if !active.is_empty() {
            let title = if active.len() == 1 {
                format!("Updating {}", names(&active)[0])
            } else {
                format!("Updating {} providers", active.len())
            };
            self.fleet.pill = Some(ProviderPillView {
                key: key_of("loading", &active),
                tone: PillTone::Loading,
                title,
                dismissible: false,
            });
            return;
        }
        let terminal = |status: &str| -> Vec<&Value> {
            providers
                .iter()
                .filter(|provider| {
                    provider["update"]["status"].as_str() == Some(status)
                        && provider["update"]["finishedAt"].as_i64().unwrap_or(0) > visible_after
                })
                .collect()
        };
        let failed = terminal("failed");
        if !failed.is_empty() {
            let key = key_of("failed", &failed);
            if dismissed(&key) {
                self.fleet.pill = None;
                return;
            }
            let title = if failed.len() == 1 {
                let version = failed[0]["update"]["targetVersion"]
                    .as_str()
                    .map(|v| format!(" v{v}"))
                    .unwrap_or_default();
                format!("{}{version} update failed", names(&failed)[0])
            } else {
                format!("{} provider updates failed", failed.len())
            };
            self.fleet.pill = Some(ProviderPillView {
                key,
                tone: PillTone::Error,
                title,
                dismissible: true,
            });
            return;
        }
        let unchanged = terminal("unchanged");
        if !unchanged.is_empty() {
            let key = key_of("unchanged", &unchanged);
            if dismissed(&key) {
                self.fleet.pill = None;
                return;
            }
            let title = if unchanged.len() == 1 {
                format!("{} still needs an update", names(&unchanged)[0])
            } else {
                format!("{} providers still need updates", unchanged.len())
            };
            self.fleet.pill = Some(ProviderPillView {
                key,
                tone: PillTone::Warning,
                title,
                dismissible: true,
            });
            return;
        }
        let succeeded = terminal("succeeded");
        if !succeeded.is_empty() {
            let key = key_of("succeeded", &succeeded);
            if dismissed(&key) {
                self.fleet.pill = None;
                return;
            }
            let title = succeeded[0]["update"]["message"]
                .as_str()
                .unwrap_or("Provider updated")
                .to_owned();
            self.fleet.pill = Some(ProviderPillView {
                key,
                tone: PillTone::Success,
                title,
                dismissible: false,
            });
            return;
        }
        self.fleet.pill = None;
    }
    /// Shows the run's one result notice (KSG `showUpdateEverythingResult`) and
    /// arms the success toast's self-dismissal.
    fn show_result_notice(&mut self, notice: ResultNotice) {
        let success = notice.success;
        self.fleet.result_notice = Some(notice);
        self.fleet.notice_shown = Some(Instant::now());
        if success {
            self.schedule_fleet_wake(Instant::now() + RESULT_NOTICE_DISMISS);
        }
    }
    /// The `update-result` notice's dismiss; the notice does not come back.
    pub fn dismiss_result_notice(&mut self) {
        self.fleet.result_notice = None;
        self.fleet.notice_shown = None;
        if self.fleet.running
            && !self.fleet.rows.is_empty()
            && self.fleet.rows.iter().all(|row| row.result.is_some())
        {
            self.fleet.running = false;
            self.fleet.phase = RunPhase::Idle;
            self.fleet.rows.clear();
        }
    }
    /// The pill's dismiss button ("Dismiss provider update notice"), keyed until the
    /// status changes.
    pub fn dismiss_provider_pill(&mut self) {
        if let Some(view) = self.fleet.pill.take() {
            self.fleet.pill_dismissed.insert(view.key);
        }
    }
    /// A sleeper wake so the countdown, elapsed readouts, the 32 minute handoff limit
    /// and the pill's 3 s dismissal refresh without a timer thread. `tx.wake_after`
    /// lives in the poll callers, which fleet code cannot reach.
    fn schedule_fleet_wake(&mut self, at: Instant) {
        if let Some(deadline) = self.fleet.wake_deadline
            && deadline > Instant::now()
            && deadline <= at
        {
            return;
        }
        self.fleet.wake_deadline = Some(at);
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let wait = at.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            wake();
        });
    }
    pub fn poll_fleet(&mut self) -> bool {
        let mut changed = false;
        if !self.headless && !self.demo {
            self.fleet.release.refresh(false);
            changed |= self.fleet.release.poll();
            if let Some(wait) = self.fleet.release.remaining() {
                self.schedule_fleet_wake(Instant::now() + wait);
            }
        }
        if !self.fleet.restored {
            self.restore_update_everything();
            changed = true;
        }
        let mut drained = Vec::new();
        if let Some(events) = &self.fleet.events {
            while let Ok(event) = events.try_recv() {
                drained.push(event);
            }
        }
        for (index, result, requested) in drained {
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
                    row.probe = Some(value.clone());
                    row.prior_outcome = value["status"]["lastOutcome"]["updateId"]
                        .as_str()
                        .map(str::to_owned);
                    row.prior_status_id = value["status"]["updateId"].as_str().map(str::to_owned);
                    row.current = value["current"].as_str().unwrap_or(&row.current).into();
                    row.provider_capable = value["providerCapable"] == true;
                    if let Some(list) = value["providers"].as_array() {
                        row.providers_view = Some(Value::Array(list.clone()));
                    }
                    self.fleet.refresh_pending.remove(&row.server);
                    self.fleet.refreshing = self.fleet.refresh_pending.len();
                    row.state = "checking".into();
                    let server = row.server.clone();
                    let providers = row.providers_view.clone();
                    let _ = row;
                    if let Some(providers) = providers {
                        self.record_providers(&server, &providers);
                    }
                }
                result => {
                    if !requested {
                        self.fleet.refresh_pending.remove(&row.server);
                        self.fleet.refreshing = self.fleet.refresh_pending.len();
                    }
                    // An answered refusal is definitive; a transport error stays
                    // uncertain so a later exact id may still adopt.
                    let refused = matches!(&result, Ok(_));
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
                    if requested {
                        row.result = Some(false);
                        row.refused = refused;
                    }
                }
            }
        }
        changed |= self.plan_fleet_versions();
        // Provider runs settle from the projection: the record with the accepted
        // updateId reaching a terminal status. `succeeded` counts; `unchanged` and
        // `failed` record "<machine> <Name>: <message>".
        let now_ms = crate::lifecycle::now() as i64;
        for index in 0..self.fleet.rows.len() {
            let server = self.fleet.rows[index].server.clone();
            let providers = self.machine_providers(&server);
            for run_index in 0..self.fleet.rows[index].provider_runs.len() {
                if self.fleet.rows[index].provider_runs[run_index].settled() {
                    continue;
                }
                if now_ms
                    .saturating_sub(self.fleet.rows[index].provider_runs[run_index].requested_ms)
                    >= PROVIDER_CONFIRM_MS
                {
                    let run = &mut self.fleet.rows[index].provider_runs[run_index];
                    run.outcome = Some("failed".into());
                    run.message = Some(PROVIDER_CONFIRM_TIMEOUT.into());
                    changed = true;
                    continue;
                }
                let driver = self.fleet.rows[index].provider_runs[run_index]
                    .driver
                    .clone();
                let record_update = providers
                    .iter()
                    .find(|provider| provider["driver"].as_str() == Some(driver.as_str()))
                    .map(|provider| provider["update"].clone());
                // No start-time adoption: only the exact accepted id, from the live
                // receipt or a `command.get` lookup, marks this run's record.
                let accepted = self.fleet.rows[index].provider_runs[run_index]
                    .update_id
                    .clone();
                let Some(update) = record_update else {
                    continue;
                };
                let status = update["status"].as_str().unwrap_or("");
                match accepted.as_deref() == update["updateId"].as_str() {
                    true if matches!(status, "succeeded" | "unchanged" | "failed") => {
                        let run = &mut self.fleet.rows[index].provider_runs[run_index];
                        run.outcome = Some(status.into());
                        run.message = update["message"].as_str().map(str::to_owned);
                        changed = true;
                    }
                    _ => {}
                }
            }
            // Provider failures collect into the machine's provider_failure.
            let failures: Vec<String> = self.fleet.rows[index]
                .provider_runs
                .iter()
                .filter(|run| matches!(run.outcome.as_deref(), Some("unchanged" | "failed")))
                .map(|run| {
                    format!(
                        "{} {}: {}",
                        self.fleet.rows[index].name,
                        run.name,
                        run.message.as_deref().unwrap_or("update failed")
                    )
                })
                .collect();
            let joined = (!failures.is_empty()).then(|| failures.join("\n"));
            if self.fleet.rows[index].provider_failure != joined {
                self.fleet.rows[index].provider_failure = joined;
                changed = true;
            }
            // A provider-only machine settles when its providers do.
            if self.fleet.rows[index].provider_only()
                && self.fleet.rows[index].result.is_none()
                && self.fleet.rows[index]
                    .provider_runs
                    .iter()
                    .all(ProviderRun::settled)
            {
                let failed = self.fleet.rows[index].provider_failure.is_some();
                self.fleet.rows[index].result = Some(!failed);
                self.fleet.rows[index].failed = failed;
                self.fleet.rows[index].state = if failed {
                    format!(
                        "failed: {}",
                        self.fleet.rows[index]
                            .provider_failure
                            .clone()
                            .unwrap_or_default()
                    )
                } else {
                    "done".into()
                };
                changed = true;
            }
        }
        // Refresh answers stop counting on disconnect or after the deadline, so
        // "Checking for more..." never sticks.
        if !self.fleet.refresh_pending.is_empty() {
            let now = Instant::now();
            let pending: Vec<String> = self.fleet.refresh_pending.keys().cloned().collect();
            for server in pending {
                let connected = self
                    .client
                    .as_ref()
                    .and_then(|client| client.commands_for(&server))
                    .is_some_and(|commands| commands.connected());
                let stale = self
                    .fleet
                    .refresh_pending
                    .get(&server)
                    .is_some_and(|at| now.duration_since(*at) >= REFRESH_DEADLINE);
                if !connected || stale {
                    self.fleet.refresh_pending.remove(&server);
                }
            }
            if self.fleet.refreshing != self.fleet.refresh_pending.len() {
                self.fleet.refreshing = self.fleet.refresh_pending.len();
                changed = true;
            }
            if let Some(oldest) = self.fleet.refresh_pending.values().min() {
                self.schedule_fleet_wake(*oldest + REFRESH_DEADLINE);
            }
        }
        // Unreceipted `providers.update` sends never resend. The exact command is
        // looked up read-only by its id; a machine that cannot produce its receipt
        // stays uncertain until the 65 minute confirmation deadline.
        let connectivity: std::collections::BTreeMap<String, bool> = self
            .fleet
            .provider_sends
            .iter()
            .map(|(_, send)| send.server.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|server| {
                let connected = self.machine_connected(&server);
                (server, connected)
            })
            .collect();
        let mut lookups = Vec::new();
        let mut expired = Vec::new();
        for (index, (id, send)) in self.fleet.provider_sends.iter_mut().enumerate() {
            if now_ms.saturating_sub(send.requested_ms) >= PROVIDER_CONFIRM_MS {
                expired.push(index);
                continue;
            }
            if connectivity.get(&send.server).copied().unwrap_or(false)
                && send
                    .lookup_at
                    .is_none_or(|at| at.elapsed() >= PROVIDER_LOOKUP_INTERVAL)
            {
                send.lookup_at = Some(Instant::now());
                lookups.push((send.server.clone(), *id));
            }
        }
        for index in expired.into_iter().rev() {
            let (_, send) = self.fleet.provider_sends.remove(index);
            changed = true;
            if send.row == usize::MAX {
                self.fleet.provider_send_errors.insert(
                    (send.server.clone(), send.driver.clone()),
                    PROVIDER_CONFIRM_TIMEOUT.to_owned(),
                );
                continue;
            }
            if let Some(run) = self.fleet.rows.get_mut(send.row).and_then(|row| {
                row.provider_runs
                    .iter_mut()
                    .find(|run| run.driver == send.driver && !run.settled())
            }) {
                run.outcome = Some("failed".into());
                run.message = Some(PROVIDER_CONFIRM_TIMEOUT.into());
            }
        }
        for (server, id) in lookups {
            let target = if server == "local" {
                self.client.as_ref()
            } else {
                self.client
                    .as_ref()
                    .and_then(|client| client.linked.get(&server))
            };
            if let Some(target) = target {
                target.call_rpc(
                    format!("fleet:receipt:{id}"),
                    "command.get",
                    json!({"commandId": id}),
                );
            }
        }
        if let Some(deadline) = self
            .fleet
            .provider_sends
            .iter()
            .map(|(_, send)| send.requested_ms + PROVIDER_CONFIRM_MS)
            .chain(
                self.fleet
                    .rows
                    .iter()
                    .flat_map(|row| &row.provider_runs)
                    .filter(|run| !run.settled())
                    .map(|run| run.requested_ms + PROVIDER_CONFIRM_MS),
            )
            .min()
        {
            let wait = (deadline - now_ms)
                .min(PROVIDER_LOOKUP_INTERVAL.as_millis() as i64)
                .clamp(1_000, 30_000) as u64;
            self.schedule_fleet_wake(Instant::now() + Duration::from_millis(wait));
        }
        // Phase Providers ends when every run settles; servers wait only on it.
        if self.fleet.phase == RunPhase::Providers
            && self
                .fleet
                .rows
                .iter()
                .flat_map(|row| &row.provider_runs)
                .all(ProviderRun::settled)
        {
            self.fleet.phase = RunPhase::Servers;
            self.start_server_phase();
            changed = true;
        }
        let offline: Vec<_> = self
            .fleet
            .rows
            .iter()
            .filter(|r| !self.machine_connected(&r.server))
            .map(|r| r.server.clone())
            .collect();
        for row in self.fleet.rows.iter_mut().filter(|r| r.attempted) {
            if let Some(view) = self.machines.views.get(&row.server) {
                let status = &view["updateStatus"];
                let outcome = &status["lastOutcome"];
                if row.update_id.is_none() {
                    // A transport loss is not proof of refusal, but an answered
                    // refusal is definitive: never adopt another client's attempt
                    // over it. Any new attempt must also differ from the attempt
                    // and the outcome this machine already reported.
                    if row.refused {
                        continue;
                    }
                    let is_new = |id: Option<&str>| {
                        id.is_some_and(|id| {
                            Some(id) != row.prior_outcome.as_deref()
                                && Some(id) != row.prior_status_id.as_deref()
                        })
                    };
                    let next = if matches!(
                        status["state"].as_str(),
                        Some("checking" | "downloading" | "ready" | "draining" | "restarting")
                    ) {
                        status["updateId"].as_str().filter(|id| is_new(Some(*id)))
                    } else {
                        outcome["updateId"].as_str().filter(|id| is_new(Some(*id)))
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
                            row.result = Some(false);
                        } else {
                            row.state = format!("updated to {}", row.current);
                            row.failed = false;
                            row.result = Some(true);
                            row.server_updated = true;
                        }
                    } else {
                        row.state = format!(
                            "failed: {}",
                            outcome["reason"].as_str().unwrap_or("Unknown failure")
                        );
                        row.failed = true;
                        row.result = Some(false);
                    }
                } else if matches!(status["state"].as_str(), Some("draining" | "restarting")) {
                    row.state = "restarting".into();
                    row.failed = false;
                    if row.handed_off.is_none()
                        && let Some(id) = row.update_id.clone()
                    {
                        row.handed_off = Some(HandedOff {
                            target: row.target.clone(),
                            update_id: id,
                            started_ms: crate::lifecycle::now() as i64,
                        });
                    }
                } else if offline.contains(&row.server) {
                    row.state =
                        "failed: Connection lost after acceptance. The target may still update."
                            .into();
                    row.failed = true;
                    row.result = Some(false);
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
        // Handed-off rows, including those restored after a desktop restart, settle on
        // the outcome with the exact updateId, on a version match, or fail at
        // 32 minutes. Nothing is re-requested.
        for row in self
            .fleet
            .rows
            .iter_mut()
            .filter(|row| row.handed_off.is_some() && row.result.is_none())
        {
            let handoff = row.handed_off.clone().expect("handed off");
            let settled = self.machines.views.get(&row.server).and_then(|view| {
                let status = &view["updateStatus"];
                let outcome = &status["lastOutcome"];
                let running = status["runningVersion"].as_str().unwrap_or("");
                if outcome["updateId"].as_str() == Some(handoff.update_id.as_str())
                    && outcome["success"] == false
                {
                    return Some(Err(outcome["reason"]
                        .as_str()
                        .unwrap_or("Unknown failure")
                        .to_owned()));
                }
                if outcome["updateId"].as_str() == Some(handoff.update_id.as_str())
                    && outcome["success"] == true
                    && crate::update::is_newer(&handoff.target, running)
                {
                    // The exact outcome won but the version is behind: not done.
                    return None;
                }
                if !crate::update::is_newer(&handoff.target, running) && !running.is_empty() {
                    return Some(Ok(()));
                }
                None
            });
            let deadline = handoff.started_ms + 32 * 60_000;
            match settled {
                Some(Ok(())) => {
                    row.result = Some(true);
                    row.server_updated = true;
                    row.failed = false;
                    row.state = format!("updated to {}", handoff.target);
                    changed = true;
                }
                Some(Err(reason)) => {
                    row.result = Some(false);
                    row.failed = true;
                    row.state = format!("failed: {reason}");
                    changed = true;
                }
                None if crate::lifecycle::now() as i64 >= deadline => {
                    row.result = Some(false);
                    row.failed = true;
                    row.state = "failed: Not back after 32 minutes".into();
                    changed = true;
                }
                None => {}
            }
        }
        if self
            .fleet
            .rows
            .iter()
            .any(|row| row.handed_off.is_some() && row.result.is_none())
        {
            let deadline = self
                .fleet
                .rows
                .iter()
                .filter(|row| row.result.is_none())
                .filter_map(|row| row.handed_off.as_ref())
                .map(|handoff| handoff.started_ms + 32 * 60_000)
                .min()
                .unwrap_or(i64::MAX);
            let wait = (deadline - crate::lifecycle::now() as i64).clamp(1_000, 30_000) as u64;
            self.schedule_fleet_wake(Instant::now() + Duration::from_millis(wait));
        }
        // A settled server still carries its provider failure (KSG's machine result is
        // failed when either side failed).
        for row in &mut self.fleet.rows {
            if row.result == Some(true) && row.provider_failure.is_some() {
                row.result = Some(false);
                row.failed = true;
                changed = true;
            }
        }
        // The local install waits for every server handoff, then counts down 5 s. A
        // download still running delays the countdown (KSG `downloadLocal`).
        let servers_pending = self.fleet.rows.iter().any(|row| {
            row.server_target.is_some() && row.result.is_none() && row.handed_off.is_none()
        });
        if self.fleet.phase == RunPhase::Servers
            && self.fleet.local_version.is_some()
            && !servers_pending
        {
            match self.updater.status() {
                crate::update::Status::Ready { .. } => {
                    self.fleet.phase = RunPhase::Countdown;
                    self.fleet.restart_at = Some(Instant::now() + Duration::from_secs(5));
                    if let Some(row) = self.fleet.rows.first_mut()
                        && row.server == "local"
                        && row.result.is_none()
                    {
                        row.state = "restarting".into();
                    }
                    changed = true;
                }
                crate::update::Status::Downloading { .. } | crate::update::Status::Checking => {
                    self.schedule_fleet_wake(Instant::now() + Duration::from_secs(1));
                }
                _ => {
                    if self
                        .fleet
                        .rows
                        .first()
                        .is_some_and(|row| row.server == "local" && row.result.is_none())
                    {
                        let notice = self
                            .updater
                            .notice()
                            .map(str::to_owned)
                            .unwrap_or_else(|| "the update download did not finish".into());
                        self.record_local_failure(format!("This app: {notice}"));
                        self.fleet.local_version = None;
                        changed = true;
                    }
                }
            }
        }
        if self.fleet.phase == RunPhase::Countdown {
            match self.fleet.restart_at {
                Some(at) if Instant::now() >= at => self.install_local_now(),
                Some(at) => {
                    self.schedule_fleet_wake(at.min(Instant::now() + Duration::from_secs(1)))
                }
                None => {}
            }
            changed = true;
        }
        // An active machine refreshes its elapsed readout every 30 s.
        if self.fleet.rows.iter().any(|row| {
            matches!(
                self.run_status(row),
                "providers" | "downloading" | "restarting"
            )
        }) {
            self.schedule_fleet_wake(Instant::now() + Duration::from_secs(30));
        }
        // KSG `closeFinishedUpdateRun`: in Servers with no local install, the run ends
        // when every machine is done; the result notice appears then.
        if self.fleet.phase == RunPhase::Servers
            && self.fleet.local_version.is_none()
            && self.fleet.running
            && !self.fleet.rows.is_empty()
            && self.fleet.rows.iter().all(|row| row.result == Some(true))
        {
            let updated_servers = self
                .fleet
                .rows
                .iter()
                .filter(|row| row.server_updated)
                .count();
            let updated_providers = self
                .fleet
                .rows
                .iter()
                .flat_map(|row| &row.provider_runs)
                .filter(|run| run.outcome.as_deref() == Some("succeeded"))
                .count();
            self.show_result_notice(result_notice(
                false,
                updated_servers,
                updated_providers,
                Vec::new(),
            ));
            self.fleet.running = false;
            self.fleet.phase = RunPhase::Idle;
            self.fleet.rows.clear();
            changed = true;
        }
        // Runs ending with any failure stay for Retry or Dismiss; their notice shows
        // once (KSG `showUpdateEverythingResult` at run end).
        if self.fleet.phase == RunPhase::Servers
            && self.fleet.local_version.is_none()
            && self.fleet.running
            && !self.fleet.rows.is_empty()
            && self.fleet.rows.iter().all(|row| row.result.is_some())
            && self.fleet.result_notice.is_none()
        {
            let updated_servers = self
                .fleet
                .rows
                .iter()
                .filter(|row| row.server_updated)
                .count();
            let updated_providers = self
                .fleet
                .rows
                .iter()
                .flat_map(|row| &row.provider_runs)
                .filter(|run| run.outcome.as_deref() == Some("succeeded"))
                .count();
            self.show_result_notice(result_notice(
                false,
                updated_servers,
                updated_providers,
                self.run_failures(),
            ));
            changed = true;
        }
        // A success notice is a toast: it clears itself. Failures stay for
        // Retry or Dismiss, like KSG's run capsule.
        if self.fleet.result_notice.as_ref().is_some_and(|n| n.success)
            && self
                .fleet
                .notice_shown
                .is_some_and(|at| at.elapsed() >= RESULT_NOTICE_DISMISS)
        {
            self.fleet.result_notice = None;
            self.fleet.notice_shown = None;
            changed = true;
        }
        self.poll_provider_pill();
        // The success pill dismisses itself 3 s after it appears.
        if let Some(view) = &self.fleet.pill
            && view.tone == PillTone::Success
        {
            match &self.fleet.pill_shown {
                Some((key, at)) if *key == view.key => {
                    if at.elapsed() >= Duration::from_secs(3) {
                        let key = key.clone();
                        self.fleet.pill = None;
                        self.fleet.pill_dismissed.insert(key);
                        self.fleet.pill_shown = None;
                        changed = true;
                    }
                }
                _ => {
                    self.fleet.pill_shown = Some((view.key.clone(), Instant::now()));
                    self.schedule_fleet_wake(Instant::now() + Duration::from_secs(3));
                }
            }
        }
        changed
    }
    /// Demo fixtures for `--snap` scenes (`update-everything-dialog`,
    /// `update-capsule-running`, `update-capsule-failed`, `provider-update-pill`).
    pub fn demo_update_scene(&mut self, scene: &str) {
        self.demo_fleet();
        match scene {
            "update-everything-dialog" => {}
            "update-capsule-running" => {
                self.fleet.open = false;
                self.fleet.running = true;
                self.fleet.phase = RunPhase::Servers;
                self.fleet.rows.retain(|row| {
                    matches!(
                        row.state.as_str(),
                        "downloading" | "restarting" | "updated to 99.0.0" | "done"
                    )
                });
                for row in &mut self.fleet.rows {
                    row.started_at = Instant::now() - Duration::from_secs(50);
                    if row.state == "updated to 99.0.0" {
                        row.result = Some(true);
                    }
                    if row.state == "downloading" {
                        row.server_target = Some(row.target.clone());
                    }
                }
            }
            "update-capsule-failed" => {
                self.fleet.open = false;
                self.fleet.running = true;
                self.fleet.phase = RunPhase::Servers;
                self.fleet.rows.retain(|row| {
                    row.failed || row.state == "updated to 99.0.0" || row.state == "restarting"
                });
                for row in &mut self.fleet.rows {
                    if row.failed {
                        row.result = Some(false);
                        row.server_target = Some(row.target.clone());
                    } else if row.state.starts_with("updated") {
                        row.result = Some(true);
                    }
                }
            }
            "provider-update-pill" => {
                self.fleet.open = false;
                if let Some(view) = self.machines.views.get_mut("local") {
                    view["providers"] = json!([{
                        "driver":"codex","name":"Codex","installed":true,
                        "version":"0.48.0","latestVersion":"0.50.0","advisory":"behind",
                        "strategy":"npm-global","disabledReason":null,"manualCommand":null,
                        "installPath":"~/.npm-global","checkedAt":crate::lifecycle::now(),
                        "update":{"updateId":"demo","status":"running",
                                  "message":"Running npm install -g ...","startedAt":crate::lifecycle::now(),
                                  "finishedAt":null,"fromVersion":"0.48.0","targetVersion":"0.50.0"}
                    }]);
                }
                self.poll_provider_pill();
            }
            _ => {}
        }
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
                provider_capable: index < 2,
                providers: if index == 0 {
                    vec!["Codex".into(), "Claude".into()]
                } else if index == 1 {
                    vec!["Codex".into()]
                } else {
                    Vec::new()
                },
                ..Default::default()
            });
        }
    }
}

impl Default for Row {
    fn default() -> Self {
        Self {
            server: String::new(),
            name: String::new(),
            current: String::new(),
            target: String::new(),
            state: String::new(),
            eligible: false,
            local_server_only: false,
            probe: None,
            failed: false,
            update_id: None,
            prior_outcome: None,
            prior_status_id: None,
            refused: false,
            attempted: false,
            link: None,
            providers: Vec::new(),
            providers_view: None,
            provider_capable: false,
            server_target: None,
            server_updated: false,
            provider_runs: Vec::new(),
            provider_failure: None,
            handed_off: None,
            started_at: Instant::now(),
            result: None,
        }
    }
}

/// KSG `joinList`: "a and b", "a, b, and c".
fn join_list(parts: &[String]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        _ => format!(
            "{} and {}",
            parts[..parts.len() - 1].join(", "),
            parts[parts.len() - 1]
        ),
    }
}
fn count_label(count: usize, noun: &str) -> Option<String> {
    (count > 0).then(|| format!("{count} {noun}{}", if count == 1 { "" } else { "s" }))
}
/// KSG `formatUpdateElapsed`: "<1m", "4m", "1h 5m".
pub fn format_update_elapsed(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs() / 60;
    if minutes < 1 {
        "<1m".into()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h {}m", minutes / 60, minutes % 60)
    }
}
/// KSG `RUN_STATUS_LABELS` for the capsule popover.
pub fn status_word(status: &str) -> &'static str {
    match status {
        "providers" => "updating providers",
        "downloading" => "downloading",
        "restarting" => "restarting",
        "done" => "done",
        "failed" => "failed",
        _ => "waiting",
    }
}
/// KSG `getUpdateEverythingToastView`: success title "Updated this app, 1 server and 3
/// providers"; failure title "Update failed" or the same Updated line in `danger`.
fn result_notice(
    local_updated: bool,
    updated_servers: usize,
    updated_providers: usize,
    failures: Vec<String>,
) -> ResultNotice {
    let mut parts = Vec::new();
    if local_updated {
        parts.push("this app".to_owned());
    }
    parts.extend(count_label(updated_servers, "server"));
    parts.extend(count_label(updated_providers, "provider"));
    if failures.is_empty() {
        return ResultNotice {
            success: true,
            title: format!("Updated {}", join_list(&parts)),
            lines: Vec::new(),
        };
    }
    ResultNotice {
        success: false,
        title: if parts.is_empty() {
            "Update failed".into()
        } else {
            format!("Updated {}", join_list(&parts))
        },
        lines: failures,
    }
}

/// What the capsule's left side and action show, KSG `UpdateCapsuleView`.
pub struct CapsuleView {
    pub label: String,
    /// "4m"-style elapsed after the label once a machine is slow (3 minutes).
    pub elapsed: Option<String>,
    pub action: CapsuleAction,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapsuleAction {
    None,
    Later,
    Retry,
    Dismiss,
}

/// `getProviderUpdateSidebarPillView`'s reduced view for the sidebar pill.
pub struct ProviderPillView {
    pub key: String,
    pub tone: PillTone,
    pub title: String,
    pub dismissible: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillTone {
    Loading,
    Success,
    Warning,
    Error,
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
        state.updater = crate::update::Updater::test_ready(crate::update::PreparedUpdate {
            tag: "v99.0.0".into(),
            target: crate::install::InstalledTarget::Windows {
                exe: "shina.exe".into(),
            },
            asset: "shina.exe".into(),
            size: 1,
            sha256: "0".repeat(64),
            download: "download.exe".into(),
            prepared: "prepared.exe".into(),
        });
        state.demo_fleet();
        state.fleet.rows.truncate(2);
        state.fleet.rows[0].server = "local".into();
        state.fleet.rows[0].eligible = true;
        state.fleet.rows[1].state = "downloading".into();
        state.fleet.rows[1].update_id = Some("accepted-update".into());
        state.fleet.rows[1].attempted = true;
        state.fleet.rows[1].server = "remote".into();
        state.fleet.rows[1].server_target = Some("v99.0.0".into());
        let (mut client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, _) = crate::client::Client::test_connected(state.data_dir.clone());
        client.linked.insert("remote".into(), remote);
        state.client = Some(client);
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{"state":"downloading"}}).into(),
        );
        state.fleet.running = true;
        state.fleet.open = false;
        state.fleet.phase = RunPhase::Servers;
        state.fleet.local_version = Some("v99.0.0".into());
        state.poll_fleet();
        assert!(!state.restart_for_update);
        assert_eq!(state.fleet.phase, RunPhase::Servers);
        state.machines.views.get_mut("remote").unwrap()["updateStatus"]["state"] =
            json!("restarting");
        state.poll_fleet();
        // The handoff is recorded and the local countdown begins; the app is still up.
        assert_eq!(state.fleet.phase, RunPhase::Countdown);
        assert!(!state.restart_for_update);
        assert!(state.fleet.rows[1].handed_off.is_some());
        state.fleet.restart_at = Some(Instant::now());
        state.poll_fleet();
        assert!(state.restart_for_update);
        assert!(!state.fleet.open);
        // The durable result is on disk before the restart request.
        let saved: Value = serde_json::from_slice(
            &std::fs::read(state.data_dir.join("updates/everything.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["handedOff"][0]["updateId"], "accepted-update");
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
                ..Default::default()
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
        // Nothing on the link can update, so the footer keeps the local row only;
        // Settings still opens the review and lists the machine as skipped.
        assert!(!state.fleet_available());
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
        }}).into());
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

    fn prepared(tag: &str) -> crate::update::PreparedUpdate {
        crate::update::PreparedUpdate {
            tag: tag.into(),
            target: crate::install::InstalledTarget::Windows {
                exe: "shina.exe".into(),
            },
            asset: "shina.exe".into(),
            size: 1,
            sha256: "0".repeat(64),
            download: "download.exe".into(),
            prepared: "prepared.exe".into(),
        }
    }
    fn behind(driver: &str, name: &str, version: &str, latest: &str) -> Value {
        json!({
            "driver": driver, "name": name, "installed": true, "version": version,
            "latestVersion": latest, "advisory": "behind", "strategy": "npm-global",
            "disabledReason": null, "manualCommand": null
        })
    }
    fn catalog(tag: &str) -> Value {
        json!({"state":"checked","tag":tag,"assets":[
            {"name":"shina.exe","size":1},
            {"name":"Shina-macos-aarch64.zip","size":1},
            {"name":"Shina-linux-x86_64.tar.gz","size":1}
        ]})
    }
    #[test]
    fn services_discover_a_release_without_a_prepared_desktop_download() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.fleet.open = true;
        state.fleet.release.set(catalog("v0.12.39"));
        for (id, version) in [("old", "0.12.34"), ("new", "0.12.39")] {
            state.fleet.rows.push(Row {
                server: id.into(),
                current: version.into(),
                state: "checking".into(),
                probe: Some(json!({"hello":{"protocolMinor":36,"canUpdate":true},
                    "status":{"runningVersion":version,"canSelfUpdate":true,"state":"idle"}})),
                ..Default::default()
            });
        }
        state.plan_fleet_versions();
        assert!(state.updater.prepared_update().is_none());
        assert!(state.fleet.rows[0].eligible);
        assert_eq!(state.fleet.rows[0].target, "v0.12.39");
        assert_eq!(state.fleet.rows[1].state, "up to date");
        assert!(!state.fleet.rows[1].eligible);
        assert_eq!(state.fleet.rows[1].target, "v0.12.39");
    }
    #[test]
    fn failed_and_pending_release_checks_never_claim_up_to_date() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.fleet.open = true;
        state.fleet.rows.push(Row {
            server: "remote".into(),
            current: "0.12.34".into(),
            probe: Some(json!({"hello":{"protocolMinor":36,"canUpdate":true},
                "status":{"runningVersion":"0.12.34","canSelfUpdate":true}})),
            ..Default::default()
        });
        state.plan_fleet_versions();
        assert_eq!(state.fleet.rows[0].state, "checking");
        assert!(state.fleet.rows[0].target.is_empty());
        state
            .fleet
            .release
            .set(json!({"state":"failed","error":"Release check unavailable."}));
        state.plan_fleet_versions();
        assert_eq!(
            state.fleet.rows[0].state,
            "skipped: Release check unavailable."
        );
        assert!(!state.fleet.rows[0].eligible);
    }
    #[test]
    fn each_service_uses_its_own_platform_release_and_missing_assets_are_not_current() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.fleet.open = true;
        state.fleet.release.set(catalog("v0.12.34"));
        state.fleet.rows.push(Row {
            server: "remote".into(),
            current: "0.12.34".into(),
            probe: Some(json!({"hello":{"protocolMinor":36,"canUpdate":true},
                "status":{"runningVersion":"0.12.34","canSelfUpdate":true,
                    "release":{"state":"checked","tag":"v0.12.39","assetAvailable":true}}})),
            ..Default::default()
        });
        state.plan_fleet_versions();
        assert_eq!(state.fleet.rows[0].target, "v0.12.39");
        assert!(state.fleet.rows[0].eligible);
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{
                "runningVersion":"0.12.34","canSelfUpdate":true,
                "release":{"state":"checked","tag":"v0.12.39","assetAvailable":false}
            }})
            .into(),
        );
        state.plan_fleet_versions();
        assert!(!state.fleet.rows[0].eligible);
        assert!(state.fleet.rows[0].state.contains("no download"));
    }
    #[test]
    fn a_current_app_still_plans_an_older_local_service() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        *client.info.lock().unwrap() = json!({"protocolMinor":36,"canUpdate":true});
        state.client = Some(client);
        state.fleet.open = true;
        state.fleet.release.set(catalog("v0.12.39"));
        state.fleet.rows.push(Row {
            server: "local".into(),
            current: "0.12.39".into(),
            ..Default::default()
        });
        state.machines.views.insert(
            "local".into(),
            json!({"updateStatus":{
                "runningVersion":"0.12.34","canSelfUpdate":true,"state":"idle"
            }})
            .into(),
        );
        state.plan_fleet_versions_for("0.12.39");
        assert!(state.fleet.rows[0].eligible);
        assert!(state.fleet.rows[0].local_server_only);
        assert!(state.updater.prepared_update().is_none());
        assert!(!state.machine_runs_version("local", "0.12.39"));
    }
    #[test]
    fn local_service_only_restart_saves_exact_handoff_and_cancellation_clears_it() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.fleet.running = true;
        state.fleet.phase = RunPhase::Servers;
        state.fleet.rows.push(Row {
            server: "local".into(),
            name: "Local service".into(),
            target: "v0.12.39".into(),
            local_server_only: true,
            attempted: true,
            update_id: Some("local-attempt".into()),
            ..Default::default()
        });
        state.prepare_fleet_service_restart().unwrap();
        let path = state.data_dir.join("updates/everything.json");
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            saved["localVersion"].is_null(),
            "an unchanged app is not counted as updated"
        );
        assert_eq!(saved["handedOff"][0]["server"], "local");
        assert_eq!(saved["handedOff"][0]["updateId"], "local-attempt");
        state.record_fleet_restart_result("Update drain cancelled. Normal operation resumed.");
        assert!(!path.exists());
        assert!(state.fleet.rows[0].handed_off.is_none());
        assert_eq!(state.fleet.rows[0].result, Some(false));
    }
    fn linked(
        state: &mut crate::model::State,
        server: &str,
        version: &str,
    ) -> tokio::sync::mpsc::Receiver<crate::protocol::Command> {
        let (mut remote, calls) = crate::client::Client::test_connected(state.data_dir.clone());
        *remote.info.lock().unwrap() = json!({
            "binaryVersion": version, "protocolMinor": 36,
            "canUpdate": true, "capabilities": [PROVIDER_UPDATES]
        });
        remote.remote = Some(crate::peer::Link {
            server_id: server.into(),
            host: server.into(),
            device_id: "fixture".into(),
            token: "not-real".into(),
            enabled: true,
        });
        state
            .client
            .as_mut()
            .expect("client")
            .linked
            .insert(server.into(), remote);
        calls
    }

    #[test]
    fn the_everything_summary_uses_oxford_commas_and_singular_forms() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.fleet.release.set(catalog("v99.0.0"));
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        state.updater = crate::update::Updater::test_ready(prepared("v99.0.0"));
        let _calls = linked(&mut state, "remote", "0.12.28");
        state.machines.views.insert(
            "remote".into(),
            json!({"providers":[
                behind("codex","Codex","0.48.0","0.50.0"),
                behind("claude","Claude","2.0.0","2.1.0")
            ],"updateStatus":{"runningVersion":"0.12.28","state":"idle","canSelfUpdate":true}})
            .into(),
        );
        assert_eq!(
            state.update_everything_summary().as_deref(),
            Some("Update this app, 1 server and 2 providers")
        );
        let _calls2 = linked(&mut state, "second", "0.12.28");
        state.machines.views.insert(
            "second".into(),
            json!({"updateStatus":{"runningVersion":"0.12.28","state":"idle","canSelfUpdate":true}}).into());
        assert_eq!(
            state.update_everything_summary().as_deref(),
            Some("Update this app, 2 servers and 2 providers")
        );
    }
    #[test]
    fn providers_settle_before_any_server_update_request() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        let mut calls = linked(&mut state, "remote", "0.1.0");
        let providers = vec![behind("codex", "Codex", "0.48.0", "0.50.0")];
        state.fleet.rows.push(Row {
            server: "local".into(),
            name: "Local".into(),
            current: env!("CARGO_PKG_VERSION").into(),
            target: "v99.0.0".into(),
            state: "will update".into(),
            eligible: false,
            ..Default::default()
        });
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            current: "0.1.0".into(),
            target: "v99.0.0".into(),
            state: "will update".into(),
            eligible: true,
            provider_capable: true,
            providers: vec!["Codex".into()],
            providers_view: Some(Value::Array(providers.clone())),
            link: Some(crate::peer::Link {
                server_id: "remote".into(),
                host: "studio".into(),
                device_id: "fixture".into(),
                token: "not-real".into(),
                enabled: true,
            }),
            ..Default::default()
        });
        let (send, receive) = mpsc::channel();
        state.fleet.sender = Some(send);
        state.fleet.events = Some(receive);
        state.confirm_fleet();
        assert_eq!(state.fleet.phase, RunPhase::Providers);
        let sent = calls.try_recv().expect("providers.update is sent first");
        assert_eq!(sent.method, "providers.update");
        assert_eq!(sent.params["driver"], "codex");
        // The server has not been asked yet; only the provider update ran.
        let server_row = state
            .fleet
            .rows
            .iter()
            .find(|row| row.server == "remote")
            .unwrap();
        assert!(!server_row.attempted);
        // The exact receipt arms the run's projection watch.
        assert!(state.fleet_receipt(
            "remote",
            &sent,
            &json!({"status":"accepted","result":{"updateAccepted":true,"updateId":"prov-1"}}),
        ));
        // The live projection carries the accepted record's terminal status.
        let request = state
            .fleet
            .rows
            .iter()
            .find(|row| row.server == "remote")
            .unwrap()
            .provider_runs[0]
            .requested_ms;
        let mut settled = providers.clone();
        settled[0]["update"] = json!({
            "updateId": "prov-1", "status": "succeeded", "startedAt": request + 1,
            "finishedAt": request + 2, "fromVersion": "0.48.0", "targetVersion": "0.50.0",
            "message": "npm install -g finished"
        });
        state
            .machines
            .views
            .insert("remote".into(), json!({"providers": settled}).into());
        state.poll_fleet();
        assert_eq!(state.fleet.phase, RunPhase::Servers);
        let server_row = state
            .fleet
            .rows
            .iter()
            .find(|row| row.server == "remote")
            .unwrap();
        assert!(
            server_row.attempted,
            "the server update starts only after providers settle"
        );
    }
    #[test]
    fn a_provider_only_machine_settles_in_the_provider_phase() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        let mut calls = linked(&mut state, "remote", "99.0.0");
        state.fleet.rows.push(Row {
            server: "local".into(),
            name: "Local".into(),
            state: "up to date".into(),
            ..Default::default()
        });
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            state: "will update".into(),
            eligible: false,
            provider_capable: true,
            providers: vec!["Codex".into()],
            providers_view: Some(Value::Array(vec![behind(
                "codex", "Codex", "0.48.0", "0.50.0",
            )])),
            ..Default::default()
        });
        state.confirm_fleet();
        let sent = calls.try_recv().expect("providers.update sent");
        assert!(state.fleet_receipt(
            "remote",
            &sent,
            &json!({"status":"accepted","result":{"updateAccepted":true,"updateId":"prov-1"}}),
        ));
        let request = state
            .fleet
            .rows
            .iter()
            .find(|row| row.server == "remote")
            .unwrap()
            .provider_runs[0]
            .requested_ms;
        let mut settled = vec![behind("codex", "Codex", "0.48.0", "0.50.0")];
        settled[0]["update"] = json!({
            "updateId": "prov-1", "status": "succeeded", "startedAt": request + 1,
            "finishedAt": request + 2, "targetVersion": "0.50.0", "message": "done"
        });
        state
            .machines
            .views
            .insert("remote".into(), json!({"providers": settled}).into());
        state.poll_fleet();
        // The run finished; the notice counts the provider, not the machine's server.
        assert!(!state.fleet.running);
        let notice = state.fleet.result_notice.as_ref().expect("result notice");
        assert!(notice.success);
        assert_eq!(notice.title, "Updated 1 provider");
    }
    #[test]
    fn a_provider_failure_marks_the_machine_and_stays_for_retry() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        let mut calls = linked(&mut state, "remote", "99.0.0");
        state.fleet.rows.push(Row {
            server: "local".into(),
            name: "Local".into(),
            state: "up to date".into(),
            ..Default::default()
        });
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            state: "will update".into(),
            provider_capable: true,
            providers_view: Some(Value::Array(vec![behind(
                "codex", "Codex", "0.48.0", "0.50.0",
            )])),
            ..Default::default()
        });
        state.confirm_fleet();
        let sent = calls.try_recv().expect("providers.update sent");
        assert!(state.fleet_receipt(
            "remote",
            &sent,
            &json!({"status":"accepted","result":{"updateAccepted":true,"updateId":"prov-1"}}),
        ));
        let request = state
            .fleet
            .rows
            .iter()
            .find(|row| row.server == "remote")
            .unwrap()
            .provider_runs[0]
            .requested_ms;
        let mut settled = vec![behind("codex", "Codex", "0.48.0", "0.50.0")];
        settled[0]["update"] = json!({
            "updateId": "prov-1", "status": "failed", "startedAt": request + 1,
            "finishedAt": request + 2, "targetVersion": "0.50.0",
            "message": "Update command exited with code 1: EACCES"
        });
        state
            .machines
            .views
            .insert("remote".into(), json!({"providers": settled}).into());
        state.poll_fleet();
        let row = state
            .fleet
            .rows
            .iter()
            .find(|row| row.server == "remote")
            .expect("a failed machine stays for retry or dismiss");
        assert_eq!(row.result, Some(false));
        assert_eq!(
            row.provider_failure.as_deref(),
            Some("studio Codex: Update command exited with code 1: EACCES")
        );
        let capsule = state.describe_capsule();
        assert_eq!(capsule.label, "studio failed");
        // Provider failures retry from Settings, so the capsule offers only Dismiss.
        assert_eq!(capsule.action, CapsuleAction::Dismiss);
        let notice = state.fleet.result_notice.expect("failure notice");
        assert!(!notice.success);
        assert_eq!(notice.title, "Update failed");
        assert_eq!(notice.lines.len(), 1);
    }
    #[test]
    fn later_cancels_the_countdown_and_restart_to_update_resumes() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.updater = crate::update::Updater::test_ready(prepared("v99.0.0"));
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        state.fleet.rows.push(Row {
            server: "local".into(),
            name: "Local".into(),
            target: "v99.0.0".into(),
            state: "will update".into(),
            eligible: true,
            ..Default::default()
        });
        state.confirm_fleet();
        state.poll_fleet();
        assert_eq!(state.fleet.phase, RunPhase::Countdown);
        let capsule = state.describe_capsule();
        assert_eq!(capsule.label, "Restart in 5s");
        assert_eq!(capsule.action, CapsuleAction::Later);
        state.postpone_update_restart();
        assert!(state.fleet.postponed_local);
        assert!(!state.restart_for_update);
        // A run whose only row was the pending local install closes; the capsule's
        // "Restart to update" remains available through the update row.
        assert!(!state.fleet.running);
        assert_eq!(state.fleet.phase, RunPhase::Idle);
        state.restart_postponed_update();
        assert!(state.restart_for_update);
        assert!(state.data_dir.join("updates/everything.json").exists());
    }
    #[test]
    fn a_refused_restart_removes_the_durable_result_and_records_the_failure() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.updater = crate::update::Updater::test_ready(prepared("v99.0.0"));
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        state.fleet.rows.push(Row {
            server: "local".into(),
            name: "Local".into(),
            target: "v99.0.0".into(),
            state: "will update".into(),
            eligible: true,
            ..Default::default()
        });
        state.confirm_fleet();
        state.poll_fleet();
        state.fleet.restart_at = Some(Instant::now());
        state.poll_fleet();
        assert_eq!(state.fleet.phase, RunPhase::Installing);
        let saved = state.data_dir.join("updates/everything.json");
        assert!(saved.exists());
        // A coordinated restart is progress, not a refusal.
        state.record_fleet_restart_result("Coordinated restart requested by the app");
        assert_eq!(state.fleet.phase, RunPhase::Installing);
        state.record_fleet_restart_result("2 threads still have work");
        assert!(!saved.exists());
        assert_eq!(state.fleet.phase, RunPhase::Servers);
        assert!(state.fleet.local_version.is_none());
        assert_eq!(state.fleet.rows[0].result, Some(false));
        let notice = state.fleet.result_notice.as_ref().expect("failure notice");
        assert!(!notice.success);
        assert_eq!(notice.lines, vec!["This app: 2 threads still have work"]);
    }

    #[test]
    fn a_failed_recovery_record_write_never_requests_a_local_restart() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        std::fs::create_dir_all(&state.data_dir).unwrap();
        std::fs::write(state.data_dir.join("updates"), b"not a directory").unwrap();
        state.fleet.local_version = Some("v99.0.0".into());
        state.fleet.running = true;
        state.fleet.rows.push(Row {
            server: "local".into(),
            ..Default::default()
        });
        state.install_local_now();
        assert!(!state.restart_for_update);
        assert_eq!(state.fleet.phase, RunPhase::Servers);
        assert!(state.fleet.local_version.is_none());
        assert_eq!(state.fleet.rows[0].result, Some(false));
        assert!(
            state.fleet.result_notice.unwrap().lines[0]
                .contains("Could not save update recovery state")
        );
    }
    fn everything_json(state: &crate::model::State, saved: Value) {
        let updates = state.data_dir.join("updates");
        std::fs::create_dir_all(&updates).unwrap();
        std::fs::write(updates.join("everything.json"), saved.to_string()).unwrap();
    }
    #[test]
    fn the_durable_result_restores_once_and_never_resends() {
        let now = crate::lifecycle::now() as i64;
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        everything_json(
            &state,
            json!({
                "savedAt": now, "localVersion": env!("CARGO_PKG_VERSION"),
                "updatedServers": 1, "updatedProviders": 2, "failures": [],
                "handedOff": [{"server":"remote","label":"studio","targetVersion":"v99.0.0",
                               "updateId":"u-1","startedAt":now}]
            }),
        );
        state.poll_fleet();
        assert_eq!(state.fleet.phase, RunPhase::Servers);
        assert_eq!(state.fleet.rows.len(), 1);
        assert_eq!(state.fleet.rows[0].state, "restarting");
        assert!(
            !state.data_dir.join("updates/everything.json").exists(),
            "the file is read once and deleted"
        );
        let notice = state.fleet.result_notice.as_ref().expect("restore notice");
        assert!(notice.success);
        assert_eq!(notice.title, "Updated this app, 1 server and 2 providers");
        assert_eq!(notice.lines, vec!["studio is still restarting."]);
        // No new request is sent during recovery: only a projection outcome settles it.
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{"runningVersion":"0.1.0","state":"idle",
                "lastOutcome":{"updateId":"other","success":true}}})
            .into(),
        );
        state.poll_fleet();
        assert_eq!(state.fleet.rows[0].result, None);
        state.machines.views.get_mut("remote").unwrap()["updateStatus"]["lastOutcome"] =
            json!({"updateId":"u-1","success":true,"tag":"v99.0.0"});
        state.machines.views.get_mut("remote").unwrap()["updateStatus"]["runningVersion"] =
            json!("99.0.0");
        state.poll_fleet();
        // The recovered handoff settled on the exact outcome and the run closed.
        assert!(state.fleet.rows.is_empty());
        assert!(!state.fleet.running);
        let closed = state.fleet.result_notice.as_ref().expect("closing notice");
        assert!(closed.success);
        assert_eq!(closed.title, "Updated 1 server");
    }
    #[test]
    fn a_handed_off_machine_fails_after_32_minutes() {
        let now = crate::lifecycle::now() as i64;
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        everything_json(
            &state,
            json!({
                "savedAt": now, "localVersion": env!("CARGO_PKG_VERSION"),
                "updatedServers": 0, "updatedProviders": 0, "failures": [],
                "handedOff": [{"server":"remote","label":"studio","targetVersion":"v99.0.0",
                               "updateId":"u-1","startedAt":now - 33 * 60_000}]
            }),
        );
        state.poll_fleet();
        assert_eq!(state.fleet.rows[0].result, Some(false));
        assert_eq!(
            state.fleet.rows[0].state,
            "failed: Not back after 32 minutes"
        );
    }
    #[test]
    fn a_stale_result_is_ignored_and_a_missed_update_reports_itself() {
        let now = crate::lifecycle::now() as i64;
        let mut stale = crate::model::State::new(PathBuf::new(), false);
        everything_json(
            &stale,
            json!({"savedAt": now - 11 * 60_000, "localVersion": env!("CARGO_PKG_VERSION"),
                   "updatedServers": 1, "updatedProviders": 0, "failures": [], "handedOff": []}),
        );
        stale.poll_fleet();
        assert!(stale.fleet.rows.is_empty());
        assert!(stale.fleet.result_notice.is_none());
        let mut missed = crate::model::State::new(PathBuf::new(), false);
        everything_json(
            &missed,
            json!({"savedAt": now, "localVersion": "v99.0.0",
                   "updatedServers": 0, "updatedProviders": 0, "failures": [], "handedOff": []}),
        );
        missed.poll_fleet();
        let notice = missed.fleet.result_notice.expect("did-not-apply notice");
        assert!(!notice.success);
        assert!(notice.lines[0].starts_with("Update did not apply."));
    }
    #[test]
    fn capsule_labels_cover_the_contract_states() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        state.fleet.running = true;
        state.fleet.phase = RunPhase::Countdown;
        state.fleet.restart_at = Some(Instant::now() + Duration::from_secs(5));
        assert_eq!(state.describe_capsule().label, "Restart in 5s");
        state.fleet.phase = RunPhase::Installing;
        assert_eq!(state.describe_capsule().label, "Restarting...");
        state.fleet.phase = RunPhase::Servers;
        state.fleet.rows.push(Row {
            server: "a".into(),
            name: "alpha".into(),
            state: "downloading".into(),
            server_target: Some("v1".into()),
            attempted: true,
            started_at: Instant::now(),
            ..Default::default()
        });
        state.fleet.rows.push(Row {
            server: "b".into(),
            name: "beta".into(),
            result: Some(true),
            ..Default::default()
        });
        assert_eq!(state.describe_capsule().label, "1 of 2 updated");
        state.fleet.rows[0].started_at = Instant::now() - Duration::from_secs(240);
        let slow = state.describe_capsule();
        assert_eq!(slow.label, "alpha");
        assert_eq!(slow.elapsed.as_deref(), Some("4m"));
    }
    #[test]
    fn retry_resends_only_failed_server_machines() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        state.fleet.running = true;
        state.fleet.phase = RunPhase::Servers;
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            result: Some(false),
            failed: true,
            server_target: Some("v99.0.0".into()),
            link: Some(crate::peer::Link {
                server_id: "remote".into(),
                host: "studio".into(),
                device_id: "fixture".into(),
                token: "not-real".into(),
                enabled: true,
            }),
            ..Default::default()
        });
        state.fleet.rows.push(Row {
            server: "other".into(),
            name: "other".into(),
            result: Some(false),
            failed: true,
            provider_runs: vec![ProviderRun {
                driver: "codex".into(),
                name: "Codex".into(),
                target: "0.50.0".into(),
                update_id: None,
                requested_ms: 0,
                outcome: Some("failed".into()),
                message: Some("nope".into()),
            }],
            ..Default::default()
        });
        assert_eq!(state.describe_capsule().action, CapsuleAction::Retry);
        let (send, _receive) = mpsc::channel();
        state.fleet.sender = Some(send);
        state.capsule_action();
        assert!(state.fleet.rows[0].attempted && state.fleet.rows[0].result.is_none());
        assert_eq!(
            state.fleet.rows[1].result,
            Some(false),
            "providers stay settled"
        );
    }
    /// Once a run is live the frozen refresh snapshot is ignored; only the
    /// projection's `machines.views[server][providers]` settles a run.
    #[test]
    fn a_frozen_refresh_snapshot_never_settles_a_provider_run() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        let mut calls = linked(&mut state, "remote", "99.0.0");
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            state: "will update".into(),
            provider_capable: true,
            providers_view: Some(Value::Array(vec![behind(
                "codex", "Codex", "0.48.0", "0.50.0",
            )])),
            ..Default::default()
        });
        state.confirm_fleet();
        let sent = calls.try_recv().expect("providers.update sent");
        assert!(state.fleet_receipt(
            "remote",
            &sent,
            &json!({"status":"accepted","result":{"updateAccepted":true,"updateId":"prov-1"}}),
        ));
        // The snapshot reports a terminal record but the projection does not.
        let mut settled = vec![behind("codex", "Codex", "0.48.0", "0.50.0")];
        settled[0]["update"] = json!({
            "updateId": "prov-1", "status": "succeeded", "startedAt": 1,
            "finishedAt": 2, "targetVersion": "0.50.0", "message": "done"
        });
        state.fleet.rows[0].providers_view = Some(Value::Array(settled));
        assert_eq!(state.fleet.phase, RunPhase::Providers);
        state.poll_fleet();
        assert!(
            !state.fleet.rows[0].provider_runs[0].settled(),
            "a frozen snapshot must not settle a live run"
        );
        // The same record in the live projection settles it.
        let mut providers = vec![behind("codex", "Codex", "0.48.0", "0.50.0")];
        providers[0]["update"] = json!({
            "updateId": "prov-1", "status": "succeeded", "startedAt": 1,
            "finishedAt": 2, "targetVersion": "0.50.0", "message": "done"
        });
        state
            .machines
            .views
            .insert("remote".into(), json!({"providers": providers}).into());
        state.poll_fleet();
        // The provider-only machine settled and the run closed.
        assert!(!state.fleet.running);
        let notice = state.fleet.result_notice.as_ref().expect("result notice");
        assert!(notice.success);
        assert_eq!(notice.title, "Updated 1 provider");
    }
    /// Later during the countdown drops a local row that only waited to install;
    /// the run settles and the postponed restart stays available.
    #[test]
    fn later_lets_the_run_settle_and_keeps_restart_to_update() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.updater = crate::update::Updater::test_ready(prepared("v99.0.0"));
        let (mut client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, _) = crate::client::Client::test_connected(state.data_dir.clone());
        client.linked.insert("remote".into(), remote);
        state.client = Some(client);
        state.fleet.rows.push(Row {
            server: "local".into(),
            name: "Local".into(),
            target: "v99.0.0".into(),
            state: "will update".into(),
            eligible: true,
            ..Default::default()
        });
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            target: "v99.0.0".into(),
            state: "will update".into(),
            eligible: true,
            ..Default::default()
        });
        state.confirm_fleet();
        // The remote handed off before the countdown started.
        let remote = state
            .fleet
            .rows
            .iter_mut()
            .find(|row| row.server == "remote")
            .unwrap();
        remote.server_target = Some("v99.0.0".into());
        remote.attempted = true;
        remote.update_id = Some("u-1".into());
        remote.state = "restarting".into();
        remote.handed_off = Some(HandedOff {
            target: "v99.0.0".into(),
            update_id: "u-1".into(),
            started_ms: crate::lifecycle::now() as i64,
        });
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{"runningVersion":"0.1.0","state":"restarting"}}).into(),
        );
        state.poll_fleet();
        assert_eq!(state.fleet.phase, RunPhase::Countdown);
        state.postpone_update_restart();
        assert!(state.fleet.postponed_local);
        assert!(
            state.fleet.rows.iter().all(|row| row.server != "local"),
            "a pending local install row leaves the run"
        );
        assert!(!state.restart_for_update);
        // The last server settles and the run completes with its notice.
        state.machines.views.get_mut("remote").unwrap()["updateStatus"] = json!({
            "runningVersion": "99.0.0", "state": "idle",
            "lastOutcome": {"updateId":"u-1","success":true,"tag":"v99.0.0"}
        });
        state.poll_fleet();
        assert!(!state.fleet.running);
        assert_eq!(state.fleet.phase, RunPhase::Idle);
        let notice = state.fleet.result_notice.as_ref().expect("closing notice");
        assert!(notice.success);
        assert_eq!(notice.title, "Updated 1 server");
        // Restart to update is still available and now goes through.
        state.restart_postponed_update();
        assert!(state.restart_for_update);
        assert_eq!(state.fleet.phase, RunPhase::Installing);
    }
    /// A machine with a provider failure and a server failure reports both.
    #[test]
    fn provider_and_server_failures_both_reach_the_notice() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            state: "failed: connection lost".into(),
            failed: true,
            result: Some(false),
            provider_failure: Some("studio Codex: exit 1".into()),
            ..Default::default()
        });
        assert_eq!(
            state.run_failures(),
            vec![
                "studio Codex: exit 1".to_owned(),
                "studio: connection lost".to_owned()
            ]
        );
        // When the provider failure is the only failure, no duplicated line appears.
        state.fleet.rows[0].state = "failed: studio Codex: exit 1".into();
        assert_eq!(
            state.run_failures(),
            vec!["studio Codex: exit 1".to_owned()]
        );
    }
    /// A definitive refusal is never erased by adopting another client's attempt;
    /// a transport loss still adopts, but never a prior attempt or outcome id.
    #[test]
    fn adoption_respects_refusals_and_prior_attempt_ids() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.demo_fleet();
        state.fleet.rows.truncate(2);
        let row = &mut state.fleet.rows[1];
        row.server = "remote".into();
        row.attempted = true;
        row.refused = true;
        row.result = Some(false);
        row.failed = true;
        row.state = "failed: Update refused.".into();
        row.prior_outcome = Some("prev-outcome".into());
        row.prior_status_id = Some("prev-attempt".into());
        // Another client's live attempt does not clear the refusal.
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{"state":"downloading","updateId":"other-attempt"}}).into(),
        );
        state.poll_fleet();
        assert_eq!(state.fleet.rows[1].update_id, None);
        assert_eq!(state.fleet.rows[1].state, "failed: Update refused.");
        assert_eq!(state.fleet.rows[1].result, Some(false));
        // A transport loss is uncertain: the prior attempt id is not adopted, but a
        // genuinely new attempt is.
        let row = &mut state.fleet.rows[1];
        row.refused = false;
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{"state":"downloading","updateId":"prev-attempt"}}).into(),
        );
        state.poll_fleet();
        assert_eq!(state.fleet.rows[1].update_id, None);
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{"state":"downloading","updateId":"new-attempt"}}).into(),
        );
        state.poll_fleet();
        assert_eq!(
            state.fleet.rows[1].update_id.as_deref(),
            Some("new-attempt")
        );
        // A prior outcome id is excluded too.
        let row = &mut state.fleet.rows[1];
        row.update_id = None;
        row.state = "failed: connection lost".into();
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{"state":"idle",
                "lastOutcome":{"updateId":"prev-outcome","success":true}}})
            .into(),
        );
        state.poll_fleet();
        assert_eq!(state.fleet.rows[1].update_id, None);
    }
    /// The footer counts only connected eligible linked machines; a cached view for
    /// a machine that is not linked does not add a server.
    #[test]
    fn a_stale_unlinked_view_is_not_a_server_update_target() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.fleet.release.set(catalog("v99.0.0"));
        state.updater = crate::update::Updater::test_ready(prepared("v99.0.0"));
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        let _calls = linked(&mut state, "remote", "0.12.28");
        state.machines.views.insert(
            "remote".into(),
            json!({"updateStatus":{"runningVersion":"0.12.28","state":"idle","canSelfUpdate":true}}).into());
        state.machines.views.insert(
            "ghost".into(),
            json!({"updateStatus":{"runningVersion":"0.0.1","state":"idle","canSelfUpdate":true}})
                .into(),
        );
        assert_eq!(
            state.update_everything_summary().as_deref(),
            Some("Update this app and 1 server"),
            "the unlinked ghost must not join the count"
        );
    }
    /// Clock skew cannot fake adoption: a projection record with a later-looking
    /// `startedAt` is ignored; the exact `command.get` receipt arms the run.
    #[test]
    fn a_skewed_record_is_ignored_until_command_get_confirms() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        let mut calls = linked(&mut state, "remote", "99.0.0");
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            state: "will update".into(),
            provider_capable: true,
            providers_view: Some(Value::Array(vec![behind(
                "codex", "Codex", "0.48.0", "0.50.0",
            )])),
            ..Default::default()
        });
        state.confirm_fleet();
        let sent = calls.try_recv().expect("providers.update sent");
        let run_request = state.fleet.rows[0].provider_runs[0].requested_ms;
        // A same-driver record that started long after our send (clock skew) is not
        // adopted while the send's receipt is still missing.
        let mut providers = vec![behind("codex", "Codex", "0.48.0", "0.50.0")];
        providers[0]["update"] = json!({
            "updateId": "old-or-foreign", "status": "running",
            "startedAt": run_request + 10 * 60_000, "targetVersion": "0.50.0"
        });
        state
            .machines
            .views
            .insert("remote".into(), json!({"providers": providers}).into());
        state.poll_fleet();
        assert_eq!(state.fleet.rows[0].provider_runs[0].update_id, None);
        assert!(!state.fleet.rows[0].provider_runs[0].settled());
        // A lookup answer for a different body does not confirm the send either.
        let id = state.fleet.provider_sends[0].0;
        let wrong = json!({"commandId":id.to_string(),"deviceId":"d","hash":"not-ours",
            "status":"accepted","result":{"updateAccepted":true,"updateId":"foreign"},"seq":1});
        assert!(state.fleet_rpc("remote", &format!("fleet:receipt:{id}"), &Ok(wrong)));
        assert_eq!(state.fleet.rows[0].provider_runs[0].update_id, None);
        assert_eq!(state.fleet.provider_sends.len(), 1);
        // The exact receipt arms the run and the terminal record settles it.
        let hash = crate::protocol::Command::new(
            "providers.update",
            json!({"driver":"codex","targetVersion":"0.50.0"}),
        )
        .hash();
        let receipt = json!({"commandId":id.to_string(),"deviceId":"d","hash":hash,
            "status":"accepted","result":{"updateAccepted":true,"updateId":"prov-9"},"seq":1});
        assert!(state.fleet_rpc(
            "another-machine",
            &format!("fleet:receipt:{id}"),
            &Ok(receipt.clone())
        ));
        assert_eq!(state.fleet.provider_sends.len(), 1);
        assert_eq!(state.fleet.rows[0].provider_runs[0].update_id, None);
        assert!(state.fleet_rpc("remote", &format!("fleet:receipt:{id}"), &Ok(receipt)));
        assert!(state.fleet.provider_sends.is_empty());
        assert_eq!(
            state.fleet.rows[0].provider_runs[0].update_id.as_deref(),
            Some("prov-9")
        );
        let mut providers = vec![behind("codex", "Codex", "0.48.0", "0.50.0")];
        providers[0]["update"] = json!({
            "updateId": "prov-9", "status": "succeeded", "startedAt": run_request - 60_000,
            "finishedAt": run_request, "targetVersion": "0.50.0", "message": "done"
        });
        state
            .machines
            .views
            .insert("remote".into(), json!({"providers": providers}).into());
        state.poll_fleet();
        assert!(!state.fleet.running);
        let notice = state.fleet.result_notice.as_ref().expect("result notice");
        assert!(notice.success);
        assert_eq!(notice.title, "Updated 1 provider");
        let _ = sent;
    }
    /// A send whose receipt never arrives and cannot be looked up expires at the
    /// confirmation deadline with an honest message; nothing is retried.
    #[test]
    fn an_unconfirmed_send_expires_at_the_deadline() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        let _calls = linked(&mut state, "remote", "99.0.0");
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            state: "will update".into(),
            provider_capable: true,
            providers_view: Some(Value::Array(vec![behind(
                "codex", "Codex", "0.48.0", "0.50.0",
            )])),
            ..Default::default()
        });
        state.confirm_fleet();
        state.fleet.provider_sends[0].1.requested_ms =
            crate::lifecycle::now() as i64 - PROVIDER_CONFIRM_MS - 1;
        state.poll_fleet();
        assert!(state.fleet.provider_sends.is_empty());
        let run = &state.fleet.rows[0].provider_runs[0];
        assert_eq!(run.outcome.as_deref(), Some("failed"));
        assert_eq!(run.message.as_deref(), Some(PROVIDER_CONFIRM_TIMEOUT));
    }
    #[test]
    fn an_accepted_provider_without_terminal_projection_expires_without_resending() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        let mut calls = linked(&mut state, "remote", "99.0.0");
        state.fleet.rows.push(Row {
            server: "remote".into(),
            name: "studio".into(),
            state: "will update".into(),
            provider_capable: true,
            providers_view: Some(Value::Array(vec![behind(
                "codex", "Codex", "0.48.0", "0.50.0",
            )])),
            ..Default::default()
        });
        state.confirm_fleet();
        let sent = calls.try_recv().expect("initial provider request");
        assert!(state.fleet_receipt(
            "remote",
            &sent,
            &json!({"status":"accepted","result":{"updateAccepted":true,"updateId":"prov-1"}}),
        ));
        assert!(state.fleet.provider_sends.is_empty());
        state.client.as_mut().unwrap().linked.remove("remote");
        state.poll_fleet();
        assert_eq!(state.fleet.phase, RunPhase::Providers);
        state.fleet.rows[0].provider_runs[0].requested_ms =
            crate::lifecycle::now() as i64 - PROVIDER_CONFIRM_MS - 1;
        state.poll_fleet();
        let run = &state.fleet.rows[0].provider_runs[0];
        assert_eq!(run.outcome.as_deref(), Some("failed"));
        assert_eq!(run.message.as_deref(), Some(PROVIDER_CONFIRM_TIMEOUT));
        assert_eq!(state.fleet.phase, RunPhase::Servers);
        assert_eq!(state.fleet.rows[0].result, Some(false));
        assert!(!state.fleet.result_notice.as_ref().unwrap().success);
        assert!(calls.try_recv().is_err(), "never resend an accepted update");
    }
    /// A restored handoff keeps up to ten minutes of elapsed time, not sixty
    /// seconds.
    #[test]
    fn a_restored_handoff_keeps_ten_minutes_of_elapsed() {
        let now = crate::lifecycle::now() as i64;
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        everything_json(
            &state,
            json!({
                "savedAt": now, "localVersion": env!("CARGO_PKG_VERSION"),
                "updatedServers": 0, "updatedProviders": 0, "failures": [],
                "handedOff": [{"server":"remote","label":"studio","targetVersion":"v99.0.0",
                               "updateId":"u-1","startedAt":now - 5 * 60_000}]
            }),
        );
        state.poll_fleet();
        assert!(
            state.fleet.rows[0].started_at.elapsed() >= Duration::from_secs(240),
            "elapsed honored up to ten minutes: {:?}",
            state.fleet.rows[0].started_at.elapsed()
        );
    }
    /// A machine that drops off never stays "Checking for more..."; neither does a
    /// refresh older than the deadline.
    #[test]
    fn refreshing_clears_on_disconnect_and_on_the_deadline() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        state
            .fleet
            .refresh_pending
            .insert("ghost".into(), Instant::now());
        state
            .fleet
            .refresh_pending
            .insert("local".into(), Instant::now());
        state.fleet.refreshing = 2;
        state.poll_fleet();
        assert_eq!(
            state.fleet.refreshing, 1,
            "an unlinked machine stops counting"
        );
        state.fleet.refresh_pending.insert(
            "local".into(),
            Instant::now() - REFRESH_DEADLINE - Duration::from_secs(1),
        );
        state.poll_fleet();
        assert_eq!(state.fleet.refreshing, 0, "the deadline ends the wait");
        assert!(state.fleet.refresh_pending.is_empty());
    }
    /// Settings sends gate duplicates while unreceipted, surface refusals inline per
    /// (server, driver) and clear on the next explicit press.
    #[test]
    fn settings_provider_sends_gate_duplicates_and_show_refusals_inline() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        let mut calls = linked(&mut state, "remote", "99.0.0");
        state.machines.views.insert(
            "remote".into(),
            json!({"providers":[behind("codex","Codex","0.48.0","0.50.0")]}).into(),
        );
        state.update_provider("remote", "codex");
        let sent = calls.try_recv().expect("providers.update sent");
        assert!(state.provider_send_pending("remote", "codex"));
        // A second press while unreceipted sends nothing.
        state.update_provider("remote", "codex");
        assert!(
            calls.try_recv().is_err(),
            "the pending send gates the duplicate"
        );
        // The refused receipt lands inline on the (server, driver) row.
        assert!(state.fleet_receipt(
            "remote",
            &sent,
            &json!({"result":{"error":{"message":"Stale version."}}}),
        ));
        assert_eq!(
            state
                .fleet
                .provider_send_errors
                .get(&("remote".to_owned(), "codex".to_owned()))
                .map(String::as_str),
            Some("Stale version.")
        );
        assert!(!state.provider_send_pending("remote", "codex"));
        // The next explicit press clears the error and sends again.
        state.update_provider("remote", "codex");
        assert!(
            state
                .fleet
                .provider_send_errors
                .get(&("remote".to_owned(), "codex".to_owned()))
                .is_none()
        );
        assert!(calls.try_recv().is_ok());
    }
    /// A send that cannot reach the machine reports inline instead of vanishing.
    #[test]
    fn a_disconnected_settings_send_reports_inline() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        // No client: the local server is not running (embedded mode).
        assert!(!state.provider_capable("local"));
        state.client = Some(crate::client::Client::new(state.data_dir.clone()));
        state.machines.views.insert(
            "local".into(),
            json!({"providers":[behind("codex","Codex","0.48.0","0.50.0")]}).into(),
        );
        state.update_provider("local", "codex");
        let error = state
            .fleet
            .provider_send_errors
            .get(&("local".to_owned(), "codex".to_owned()))
            .expect("inline error");
        assert!(error.contains("disconnected"), "{error}");
        assert!(!state.provider_send_pending("local", "codex"));
    }
    /// KSG's result is a toast: a success notice clears itself, a failure waits
    /// for Retry or Dismiss like the capsule's settled rows.
    #[test]
    fn a_success_result_notice_clears_itself_but_a_failure_stays() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.show_result_notice(ResultNotice {
            success: true,
            title: "Updated 1 provider".into(),
            lines: Vec::new(),
        });
        state.fleet.notice_shown = Some(Instant::now() - RESULT_NOTICE_DISMISS);
        state.poll_fleet();
        assert!(state.fleet.result_notice.is_none());
        state.show_result_notice(ResultNotice {
            success: false,
            title: "Update failed".into(),
            lines: vec!["studio: refused".into()],
        });
        state.fleet.notice_shown = Some(Instant::now() - RESULT_NOTICE_DISMISS * 2);
        state.poll_fleet();
        assert!(state.fleet.result_notice.is_some());
        state.dismiss_result_notice();
        assert!(state.fleet.result_notice.is_none());
    }
    /// The idle "Check for updates" press is quiet: with no client it only arms
    /// the app's own release check and sends nothing.
    #[test]
    fn check_for_updates_without_a_client_only_arms_the_app_check() {
        let mut state = crate::model::State::new(PathBuf::new(), false);
        state.updater = crate::update::Updater::test_checkable();
        state.check_for_updates();
        assert!(state.fleet.result_notice.is_none());
        assert!(state.fleet.rows.is_empty());
    }
}
