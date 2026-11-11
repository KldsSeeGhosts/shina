//! State integration kept separate from the conversation reducer.
use super::*;
use crate::{
    power::{self, KeepAwake},
    routines::{MissedPolicy, RoutineView, Routines, Schedule},
};
use std::{
    net::{TcpStream, ToSocketAddrs},
    time::SystemTime,
};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackgroundSettings {
    pub keep_working: bool,
    pub keep_awake: KeepAwake,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackgroundStatus {
    pub hidden_to_tray: bool,
    pub working: usize,
    pub keep_awake_active: bool,
    pub paused_offline: bool,
    pub routines_paused_while_away: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Background {
    pub settings: BackgroundSettings,
    pub handle: Option<rune::AppHandle<State>>,
    tray: Option<crate::tray::Tray>,
    pub routines: Routines,
    awake: power::Awake,
    clock: power::SleepClock,
    gate: power::NetworkGate,
    probe: Option<mpsc::Receiver<bool>>,
    next_probe: Option<Instant>,
    endpoint: Option<(String, u16)>,
    status: BackgroundStatus,
    started: bool,
    routine_retry: Option<Instant>,
    pub(super) quitting: bool,
}

pub fn close_to_tray(enabled: bool, working: usize, supported: bool, quitting: bool) -> bool {
    enabled && working > 0 && supported && !quitting
}

fn endpoint(url: &str) -> Option<(String, u16)> {
    let (scheme, tail) = url.split_once("://")?;
    let default = match scheme {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    let authority = tail.split('/').next()?;
    if authority.contains('@') {
        return None;
    }
    if let Some(ipv6) = authority.strip_prefix('[') {
        let (host, rest) = ipv6.split_once(']')?;
        return Some((
            host.into(),
            if rest.is_empty() {
                default
            } else {
                rest.strip_prefix(':')?.parse().ok()?
            },
        ));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => Some((host.into(), port.parse().ok()?)),
        None if !authority.is_empty() => Some((authority.into(), default)),
        _ => None,
    }
}

fn discover_endpoint() -> Option<(String, u16)> {
    for name in ["CPA_BASE_URL", "CPA_ENDPOINT", "CPA_URL"] {
        if let Ok(url) = std::env::var(name)
            && let Some(endpoint) = endpoint(&url)
        {
            return Some(endpoint);
        }
    }
    let dir = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".pi/agent"))
        })?;
    let registry: Value =
        serde_json::from_slice(&std::fs::read(dir.join("models.json")).ok()?).ok()?;
    endpoint(registry["providers"]["cpa"]["baseUrl"].as_str()?)
}

impl Background {
    pub(super) fn observe_record(&mut self, record: &Value) {
        if record["command"] == "get_state"
            && record["data"]["model"]["provider"] == "cpa"
            && let Some(url) = record["data"]["model"]["baseUrl"].as_str()
            && let Some(target) = endpoint(url)
        {
            self.endpoint = Some(target);
        }
    }
}

impl State {
    pub fn background_settings(&self) -> &BackgroundSettings {
        &self.background.settings
    }
    pub fn background_status(&self) -> BackgroundStatus {
        let mut status = if self.client.is_some() {
            serde_json::from_value(self.client_views["background"].clone()).unwrap_or_default()
        } else {
            self.background.status.clone()
        };
        status.working = self.background_work_count();
        status.hidden_to_tray = self
            .background
            .handle
            .as_ref()
            .is_some_and(|h| !h.is_visible());
        status.paused_offline |= self.threads.iter().any(|t| t.status == Status::Paused);
        #[cfg(target_os = "linux")]
        if status.error.is_none() {
            status.error = Some("Idle-input detection is not supported on Linux. Routines do not pause automatically while you are away.".into());
        }
        status
    }
    pub fn set_keep_working(&mut self, enabled: bool) {
        self.background.settings.keep_working = enabled && cfg!(windows);
        if enabled && !cfg!(windows) {
            self.background.status.error =
                Some("Close-to-tray is supported on Windows only.".into());
        }
        // Never strand an invisible app when the user turns this setting off.
        if !enabled && let Some(handle) = &self.background.handle {
            handle.show();
        }
        self.checkpoint();
        (self.wake)();
    }
    pub fn set_keep_awake(&mut self, policy: KeepAwake) {
        self.forward("background.keepAwake", json!({"policy":policy}));
        self.background.settings.keep_awake = policy;
        self.checkpoint();
        (self.wake)();
    }
    pub fn routines(&self, captain: usize) -> Vec<RoutineView> {
        if self.client.is_some() {
            return serde_json::from_value(
                self.client_views["routines"][captain.to_string()].clone(),
            )
            .unwrap_or_default();
        }
        self.threads
            .iter()
            .find(|t| t.captain_home && t.captain == Some(captain))
            .map(|home| self.background.routines.views(home.id))
            .unwrap_or_default()
    }
    pub fn set_routine_paused(&mut self, id: u64, paused: bool) -> Result<(), String> {
        if let Some(result) = self.remote("routine.pause", json!({"id":id,"paused":paused})) {
            return result.map(|_| ());
        }
        self.background.routines.set_paused(id, paused)?;
        (self.wake)();
        Ok(())
    }
    pub fn delete_routine(&mut self, id: u64) -> Result<(), String> {
        if let Some(result) = self.remote("routine.remove", json!({"id":id})) {
            return result.map(|_| ());
        }
        self.background.routines.delete(id)?;
        (self.wake)();
        Ok(())
    }
    pub(super) fn background_work_count(&self) -> usize {
        self.threads
            .iter()
            .filter(|t| {
                t.status.busy()
                    || t.configuring
                    || t.queued()
                    || t.request.is_some()
                    || self.task_has_background_work(t.id)
            })
            .count()
    }
    pub fn close_in_background(&mut self) -> bool {
        let allowed = close_to_tray(
            self.background.settings.keep_working,
            if self.client.is_some() {
                self.background_work_count().max(1)
            } else {
                self.background_work_count()
            },
            self.background.tray.is_some(),
            rune::quitting(),
        );
        if allowed {
            if !self
                .background
                .tray
                .as_ref()
                .is_some_and(|tray| tray.show_for_close(self.background_work_count()))
            {
                self.background.status.error = Some(
                    "The notification-area icon is unavailable. Shina will quit instead of hiding."
                        .into(),
                );
                return false;
            }
            // A hidden host is still live. Do not mark this run as a clean quit.
            self.checkpoint();
        }
        allowed
    }
    pub(super) fn network_blocked(&self) -> bool {
        self.background.gate.blocked(Instant::now())
    }
    pub(super) fn routines_running(&self) -> bool {
        self.background
            .routines
            .entries()
            .iter()
            .any(|r| r.active_delivery.is_some())
    }
    pub(super) fn setup_background(&mut self) {
        self.background.routines = Routines::load(self.data_dir.join("routines.json"));
        self.background.endpoint = discover_endpoint();
        self.background.started = true;
        match crate::tray::Tray::start(self.wake.clone()) {
            Ok(tray) => self.background.tray = Some(tray),
            Err(error) if cfg!(windows) => self.background.status.error = Some(error),
            Err(_) => {}
        }
    }
    pub(super) fn setup_headless_background(&mut self) {
        self.background.routines = Routines::load(self.data_dir.join("routines.json"));
        self.background.endpoint = discover_endpoint();
        self.background.started = true;
    }
    pub(super) fn setup_client_background(&mut self) {
        self.background.started = true;
        match crate::tray::Tray::start(self.wake.clone()) {
            Ok(tray) => self.background.tray = Some(tray),
            Err(error) => self.background.status.error = Some(error),
        }
    }
    pub(super) fn poll_client_background(&mut self, tx: &mut Tx) -> bool {
        let Some(tray) = &self.background.tray else {
            return false;
        };
        let changed = match tray.action() {
            1 => {
                if let Some(handle) = &self.background.handle {
                    handle.open();
                }
                true
            }
            2 => {
                self.background.quitting = true;
                tx.quit();
                true
            }
            _ => false,
        };
        let hidden = self
            .background
            .handle
            .as_ref()
            .is_some_and(|h| !h.is_visible());
        tray.update(
            self.background_work_count(),
            self.background.settings.keep_working || hidden,
        );
        changed
    }
    pub(super) fn routine_mcp(
        &mut self,
        caller: usize,
        tool: &str,
        args: &Value,
    ) -> Result<Value, String> {
        let home = self
            .threads
            .iter()
            .find(|t| t.id == caller && t.captain_home && !t.delegated)
            .ok_or("Only a captain's home can manage its routines.")?;
        let captain = home.captain.ok_or("The caller is not a captain.")?;
        if self.demo || self.store.is_none() || self.ui_store.is_none() {
            return Err("Routine storage is unavailable.".into());
        }
        if tool == "list_routines" {
            return serde_json::to_value(self.background.routines.views(caller))
                .map_err(|e| e.to_string());
        }
        if tool == "create_routine" {
            let name = args["name"].as_str().ok_or("name is required.")?.to_owned();
            let prompt = args["prompt"]
                .as_str()
                .ok_or("prompt is required.")?
                .to_owned();
            let schedule: Schedule = serde_json::from_value(args["schedule"].clone())
                .map_err(|e| format!("Invalid schedule: {e}"))?;
            let policy: MissedPolicy =
                serde_json::from_value(args.get("missedPolicy").cloned().unwrap_or(json!("skip")))
                    .map_err(|_| "missedPolicy must be skip or run_once.")?;
            let id = self.background.routines.create(
                captain,
                caller,
                name,
                prompt,
                schedule,
                policy,
                chrono::Local::now(),
            )?;
            (self.wake)();
            return Ok(json!({"id": id}));
        }
        let id = args["id"].as_u64().ok_or("id must be a routine id.")?;
        if !self
            .background
            .routines
            .entries()
            .iter()
            .any(|r| r.id == id && r.home == caller)
        {
            return Err("This routine does not belong to the calling captain.".into());
        }
        match tool {
            "pause_routine" => {
                self.set_routine_paused(id, args["paused"].as_bool().unwrap_or(true))?
            }
            "delete_routine" => self.delete_routine(id)?,
            _ => return Err("Unknown routine tool.".into()),
        }
        Ok(json!({"ok": true}))
    }

    /// Called before draining agent records so reducer-triggered sends also see the gate.
    pub(super) fn poll_background_start(&mut self, tx: &mut Tx) -> bool {
        if !self.background.started {
            return false;
        }
        let before = self.background.status.clone();
        let now = Instant::now();
        let bg = &mut self.background;
        let native_resume = bg.tray.as_ref().is_some_and(|t| t.resumed());
        if bg.clock.sample(now, SystemTime::now()) || native_resume {
            bg.gate.resume(now);
            // An in-flight probe belongs to the previous resume generation.
            bg.probe = None;
            bg.next_probe = Some(now);
        }
        if let Some(tray) = &bg.tray {
            match tray.action() {
                1 => {
                    if let Some(handle) = &bg.handle {
                        handle.open();
                    }
                }
                2 => {
                    bg.quitting = true;
                    tx.quit();
                }
                _ => {}
            }
        }
        if let Some(probe) = &bg.probe
            && let Ok(success) = probe.try_recv()
        {
            bg.gate.probe(success);
            bg.probe = None;
            bg.next_probe = Some(now + Duration::from_secs(2));
        }
        if bg.gate.blocked(now) {
            if bg.probe.is_none() && bg.next_probe.is_none_or(|at| at <= now) {
                let target = bg
                    .endpoint
                    .clone()
                    .unwrap_or_else(|| ("1.1.1.1".into(), 443));
                let wake = self.wake.clone();
                let (send, receive) = mpsc::sync_channel(1);
                bg.probe = Some(receive);
                std::thread::spawn(move || {
                    let success = (target.0.as_str(), target.1)
                        .to_socket_addrs()
                        .ok()
                        .is_some_and(|mut addresses| {
                            addresses.any(|address| {
                                TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_ok()
                            })
                        });
                    let _ = send.send(success);
                    wake();
                });
            }
            tx.wake_after(
                bg.next_probe
                    .map(|at| at.saturating_duration_since(now))
                    .unwrap_or(Duration::from_secs(2))
                    .max(Duration::from_millis(100)),
            );
            // Also bound the gate when the OS resolver cannot finish.
            tx.wake_after(Duration::from_secs(2));
        }
        bg.status.paused_offline = bg.gate.blocked(now);
        let was_away = bg.status.routines_paused_while_away;
        bg.status.routines_paused_while_away = power::away(power::idle_time());
        if bg.status.routines_paused_while_away
            && !was_away
            && let Err(error) = bg.routines.note_away()
        {
            bg.status.error = Some(error);
        }
        for thread in &mut self.threads {
            if thread.offline_blocked
                && !bg.status.paused_offline
                && thread.session_started.is_some()
            {
                thread.session_started = Some(now);
            }
            thread.offline_blocked = bg.status.paused_offline;
        }
        // Power/input changes have no Rune event. This watchdog also samples wall-clock
        // gaps on platforms without WM_POWERBROADCAST. It is not a schedule polling loop.
        tx.wake_after(Duration::from_secs(30));
        before != bg.status
    }

    pub(super) fn poll_background_end(&mut self, tx: &mut Tx) -> bool {
        if !self.background.started {
            return false;
        }
        let before = self.background.status.clone();
        let now = Instant::now();
        let can_write = self.background.routine_retry.is_none_or(|at| at <= now);
        // Finish only once Pi has accepted the delivery and the home has settled. A
        // scheduled routine alone does not block update restarts or keep-awake.
        let finished: Vec<_> = self
            .background
            .routines
            .entries()
            .iter()
            .filter(|_| can_write)
            .filter_map(|r| {
                let delivery = r.active_delivery.as_ref()?;
                let home = self.threads.iter().find(|t| t.id == r.home);
                let state = match home {
                    None => Some("Captain home removed"),
                    Some(t) if matches!(t.status, Status::Failed | Status::Paused) => {
                        Some("Needs review")
                    }
                    Some(t) if !t.status.busy() && !t.queued() => {
                        Some(if t.accepted_deliveries.contains(delivery) {
                            "Finished"
                        } else {
                            "Stopped before delivery"
                        })
                    }
                    _ => None,
                }?;
                Some((r.id, state))
            })
            .collect();
        for (id, status) in finished {
            if let Err(error) = self.background.routines.finish(id, status) {
                self.background.status.error = Some(error);
                self.background.routine_retry = Some(now + Duration::from_secs(30));
            }
        }
        let waiting: HashSet<usize> = self
            .threads
            .iter()
            .filter(|t| t.request.is_some() || t.status == Status::Waiting)
            .flat_map(|t| [Some(t.id), t.parent].into_iter().flatten())
            .collect();
        let blocked: HashSet<usize> = self
            .threads
            .iter()
            .filter(|t| {
                waiting.contains(&t.id) || matches!(t.status, Status::Failed | Status::Paused)
            })
            .map(|t| t.id)
            .chain(
                self.background
                    .routines
                    .entries()
                    .iter()
                    .filter(|r| {
                        !self
                            .threads
                            .iter()
                            .any(|t| t.id == r.home && t.captain_home && !t.archived())
                    })
                    .map(|r| r.home),
            )
            .collect();
        let mut changed = false;
        if !self.background.status.paused_offline
            && !self.background.status.routines_paused_while_away
            && self.store.is_some()
            && self.ui_store.is_some()
            && self.background.routines.error.is_none()
            && self.background.routine_retry.is_none_or(|at| at <= now)
        {
            self.background.routine_retry = None;
            if let Err(error) = self
                .background
                .routines
                .hold_for_response(chrono::Local::now(), |home| waiting.contains(&home))
            {
                self.background.status.error = Some(error);
                self.background.routine_retry = Some(now + Duration::from_secs(30));
            }
            match self
                .background
                .routines
                .due(chrono::Local::now(), |home| blocked.contains(&home))
            {
                Ok(messages) => {
                    for (home, message) in messages {
                        if let Some(index) = self.threads.iter().position(|t| t.id == home) {
                            self.threads[index].pending_injections.push_back(Injection {
                                text: message,
                                shown: false,
                            });
                            if self.threads[index].pi.is_none() {
                                self.connect_thread(index);
                            }
                            self.threads[index].flush_injection();
                            changed = true;
                        }
                    }
                }
                Err(error) => {
                    self.background.status.error = Some(error);
                    self.background.routine_retry = Some(now + Duration::from_secs(30));
                }
            }
            if let Some(deadline) = self
                .background
                .routines
                .deadline(|home| blocked.contains(&home))
                && self.background.routine_retry.is_none()
            {
                tx.wake_after((deadline - chrono::Utc::now()).to_std().unwrap_or_default());
            }
        }
        if let Some(retry) = self.background.routine_retry {
            tx.wake_after(retry.saturating_duration_since(now));
        }
        let working = self.background_work_count();
        let bg = &mut self.background;
        let active = self
            .threads
            .iter()
            .any(|t| t.status == Status::Running && t.request.is_none() && !t.offline_blocked);
        bg.awake
            .update(bg.settings.keep_awake, active, power::ac_power());
        bg.status.keep_awake_active = bg.awake.active;
        bg.status.working = working;
        if let Some(error) = bg.awake.error.clone().or_else(|| bg.routines.error.clone()) {
            bg.status.error = Some(error);
        }
        if let Some(tray) = &bg.tray {
            // Keep a route back to a hidden, now-idle window.
            let hidden = bg.handle.as_ref().is_some_and(|h| !h.is_visible());
            tray.update(working, bg.settings.keep_working || hidden);
        }
        changed || before != bg.status
    }

    /// Run before task delivery, so an offline failure does not start a parent turn.
    pub(super) fn pause_offline_errors(&mut self) -> bool {
        if !self.background.gate.recent_resume(Instant::now()) {
            return false;
        }
        let mut changed = false;
        for thread in &mut self.threads {
            if thread.status == Status::Failed
                && thread
                    .error
                    .as_ref()
                    .is_some_and(|e| e.to_ascii_lowercase().contains("connection error"))
            {
                thread.status = Status::Paused;
                thread.phase = "Paused (offline)".into();
                let original = thread.error.as_deref().unwrap_or_default();
                if !original.starts_with("Paused (offline).") {
                    thread.error = Some(format!(
                        "Paused (offline). {original}\nRetry when the connection returns. Check partial work before retrying."
                    ));
                }
                // Keep a delegated task pending until the user retries or dismisses it.
                thread.task_result = None;
                thread.completion_notified = false;
                if thread.retry_prompt.is_none() {
                    thread.retry_prompt = Some(Prompt::new(
                        "The connection failed after this machine resumed. Check the current state and partial tool results, then continue the previous task. Do not repeat completed actions.".into(),
                        vec![],
                    ));
                }
                changed = true;
            }
        }
        changed
    }
}

use std::collections::HashSet;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_decision_requires_opt_in_work_tray_and_no_quit() {
        assert!(close_to_tray(true, 1, true, false));
        for (enabled, count, supported, quitting) in [
            (false, 1, true, false),
            (true, 0, true, false),
            (true, 1, false, false),
            (true, 1, true, true),
        ] {
            assert!(!close_to_tray(enabled, count, supported, quitting));
        }
    }
    #[test]
    fn endpoint_parsing_never_uses_userinfo_and_preserves_explicit_ports() {
        assert_eq!(
            endpoint("http://127.0.0.1:8317/v1"),
            Some(("127.0.0.1".into(), 8317))
        );
        assert_eq!(
            endpoint("https://proxy.example/v1"),
            Some(("proxy.example".into(), 443))
        );
        assert_eq!(endpoint("http://[::1]:8317/v1"), Some(("::1".into(), 8317)));
        assert_eq!(endpoint("https://secret@proxy.example"), None);
    }

    fn wired() -> (State, mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
        let mut state = State::new(PathBuf::new(), false);
        let (pi, commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().model_provider = "cpa".into();
        (state, commands, events)
    }

    #[test]
    fn settings_default_off_and_round_trip_in_ui_state() {
        let mut state = State::new(PathBuf::new(), false);
        assert!(!state.background_settings().keep_working);
        assert_eq!(state.background_settings().keep_awake, KeepAwake::Off);
        state.background.settings.keep_working = true;
        state.background.settings.keep_awake = KeepAwake::OnAcPower;
        let saved = state.ui_snapshot();
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_ui(&saved).unwrap();
        assert!(restored.background_settings().keep_working);
        assert_eq!(
            restored.background_settings().keep_awake,
            KeepAwake::OnAcPower
        );
        let mut legacy = saved.clone();
        legacy.as_object_mut().unwrap().remove("background");
        restored.restore_ui(&legacy).unwrap();
        assert!(!restored.background_settings().keep_working);
        let mut damaged = saved;
        damaged["background"]["keep_working"] = json!("yes");
        assert!(restored.restore_ui(&damaged).is_err());
    }

    #[test]
    fn offline_gate_preserves_user_drafts_and_blocks_every_prompt_dispatch_route() {
        let (mut state, commands, _events) = wired();
        state.background.gate.resume(Instant::now());
        state.current_mut().draft.replace_text("Keep my draft");
        state.send();
        assert_eq!(state.current().draft.text, "Keep my draft");
        assert!(commands.try_recv().is_err());
        let thread = state.current_mut();
        thread.offline_blocked = true;
        thread.enqueue_injection("Routine or task delivery".into());
        thread.native.queue.push_back(pi_session::QueuedMessage {
            id: 99,
            prompt: Prompt::new("Queued user turn".into(), vec![]),
        });
        assert!(!thread.flush_injection());
        assert!(thread.drive_native());
        assert_eq!(thread.native.queue.len(), 1);
        thread.send();
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn resume_connection_errors_pause_without_replay_and_offer_explicit_retry() {
        let (mut state, commands, _events) = wired();
        state.background.gate.resume(Instant::now());
        state.current_mut().status = Status::Failed;
        state.current_mut().error = Some("Connection error.".into());
        assert!(state.pause_offline_errors());
        assert_eq!(state.current().status, Status::Paused);
        assert!(state.background_status().paused_offline);
        assert!(state.current().retry_prompt.is_some());
        state.retry();
        assert!(
            commands.try_recv().is_err(),
            "retry is gated until connectivity returns"
        );
        assert!(!state.pause_offline_errors());
        state.background.gate.probe(true);
        state.background.gate.probe(true);
        assert_eq!(
            state.current().status,
            Status::Paused,
            "connectivity alone never replays a turn"
        );
        state.retry();
        assert_eq!(commands.try_recv().unwrap()["type"], "prompt");
    }

    #[test]
    fn normal_connection_errors_remain_failures_and_queued_work_counts_for_close() {
        let (mut state, _commands, _events) = wired();
        state.current_mut().status = Status::Failed;
        state.current_mut().error = Some("Connection error.".into());
        assert!(!state.pause_offline_errors());
        assert_eq!(state.current().status, Status::Failed);
        assert_eq!(state.background_work_count(), 0);
        state
            .current_mut()
            .enqueue_injection("Queued report".into());
        assert_eq!(state.background_work_count(), 1);
    }

    #[test]
    fn cancelled_routine_delivery_releases_the_update_guard() {
        let (mut state, _commands, _events) = wired();
        state.load_global_for_test();
        state.background.started = true;
        state.background.routines = Routines::load(state.data_dir.join("routines.json"));
        state
            .background
            .routines
            .create(
                0,
                1,
                "Review".into(),
                "Read status".into(),
                Schedule::Every { minutes: 15 },
                MissedPolicy::Skip,
                chrono::Local::now(),
            )
            .unwrap();
        let messages = state
            .background
            .routines
            .due(chrono::Local::now() + chrono::Duration::minutes(15), |_| {
                false
            })
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert!(state.routines_running());
        // The user cleared the queued message before Pi accepted it.
        state.poll_background_end(&mut Tx::default());
        assert!(!state.routines_running());
        assert_eq!(
            state.background.routines.entries()[0].last_status,
            "Stopped before delivery"
        );
    }

    #[test]
    fn routines_are_owned_by_the_calling_home_and_only_running_routines_block_updates() {
        let (mut state, commands, _events) = wired();
        state.load_global_for_test();
        state.background.routines = Routines::load(state.data_dir.join("routines.json"));
        state.captains = Captain::presets();
        state.current_mut().captain = Some(0);
        state.current_mut().captain_home = true;
        let id = state.mcp_call(1, "create_routine", &json!({
            "name":"Review", "prompt":"Read status", "schedule":{"kind":"every","minutes":15}
        })).unwrap()["id"].as_u64().unwrap();
        assert_eq!(state.routines(0).len(), 1);
        assert!(state.update_restart_blocker().is_none());
        let mut other = Thread::new(2);
        other.captain_home = true;
        other.captain = Some(1);
        state.threads.push(other);
        assert!(
            state
                .mcp_call(2, "delete_routine", &json!({"id":id}))
                .is_err()
        );
        assert!(
            state
                .mcp_call(2, "pause_routine", &json!({"id":id}))
                .is_err()
        );
        let messages = state
            .background
            .routines
            .due(chrono::Local::now() + chrono::Duration::minutes(15), |_| {
                false
            })
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert!(state.update_restart_blocker().unwrap().contains("routine"));
        state.current_mut().pending_injections.push_back(Injection {
            text: messages[0].1.clone(),
            shown: false,
        });
        assert!(state.current_mut().flush_injection());
        let prompt = commands.try_recv().unwrap();
        assert!(
            prompt["message"]
                .as_str()
                .unwrap()
                .contains("Routine: Review")
        );
        let ack = json!({"type":"response","command":"prompt","success":true,"id":prompt["id"]});
        state.current_mut().apply(ack);
        let delivery = state.background.routines.entries()[0]
            .active_delivery
            .clone()
            .unwrap();
        assert!(state.current().accepted_deliveries.contains(&delivery));
        state.delete_routine(id).unwrap();
        assert!(state.routines(0).is_empty());
    }
}
