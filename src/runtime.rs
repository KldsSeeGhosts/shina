//! One serialized owner of the existing reducer. Network tasks never borrow State.
use crate::{
    model::State,
    protocol::{Command, Receipt, ReceiptStatus},
};
use serde_json::{Value, json};
use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// UI resources are absent in the headless runtime. Only a view dereference builds them.
pub struct UiResource<T> {
    value: OnceLock<T>,
    initialize: Mutex<Option<Box<dyn FnOnce() -> T + Send>>>,
}
impl<T> UiResource<T> {
    pub fn new(initialize: impl FnOnce() -> T + Send + 'static) -> Self {
        Self {
            value: OnceLock::new(),
            initialize: Mutex::new(Some(Box::new(initialize))),
        }
    }
    #[cfg(test)]
    pub fn is_initialized(&self) -> bool {
        self.value.get().is_some()
    }
}
impl<T> Deref for UiResource<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value.get_or_init(|| {
            self.initialize
                .lock()
                .expect("UI resource")
                .take()
                .expect("initializer")()
        })
    }
}
impl<T> DerefMut for UiResource<T> {
    fn deref_mut(&mut self) -> &mut T {
        let _ = <Self as Deref>::deref(self);
        self.value.get_mut().expect("initialized")
    }
}

pub struct Call {
    pub device: String,
    pub command: Command,
    pub reply: tokio::sync::oneshot::Sender<Result<Value, String>>,
}

pub struct RuntimeState {
    pub state: State,
    pub seq: u64,
    last_projection: Value,
    pub maintenance: crate::maintenance::Maintenance,
    validation_hold: bool,
    pub database: Option<crate::database::Database>,
    failure: Option<String>,
    pending_effects: Vec<crate::outbox::Effect>,
}

impl RuntimeState {
    pub fn open(directory: PathBuf, wake: rune::Waker) -> Result<Self, String> {
        let database = crate::database::Database::open(&directory)?;
        database.fence_dispatch(directory.join("update-validation").exists());
        database.begin()?;
        let capture = crate::outbox::Capture::begin();
        let mut state = State::headless(directory);
        state.setup_runtime(wake)?;
        let mut runtime = Self::from_state(state);
        runtime.seq = runtime.seq.max(database.sequence()?);
        if let Some(previous) = database.projection()? {
            runtime.last_projection = previous;
        }
        runtime.database = Some(database);
        let effects = capture.finish();
        runtime.persist(None, effects)?;
        Ok(runtime)
    }
    pub fn from_state(state: State) -> Self {
        Self {
            validation_hold: state.data_dir.join("update-validation").exists(),
            seq: state.server_sequence(),
            last_projection: state.projection(),
            state,
            maintenance: crate::maintenance::Maintenance::default(),
            database: None,
            failure: None,
            pending_effects: Vec::new(),
        }
    }

    pub fn command(&mut self, device: &str, command: Command) -> Result<Value, String> {
        if let Some(error) = self.failure() {
            return Err(error);
        }
        if let Some(database) = &self.database
            && !database.device_active(device)?
        {
            return Err("Authentication denied. This device is unknown or revoked.".into());
        }
        if let Some(database) = &self.database {
            database.authorize(device, &command.method, &command.params)?;
        }
        if let Some(result) = self.maintenance_command(&command) {
            return result;
        }
        if self.maintenance.fenced() || self.validation_hold {
            return Err(
                "Server is draining for an update. Retry after restart or cancellation.".into(),
            );
        }
        if matches!(
            command.method.as_str(),
            "peer.capabilities" | "peer.thread.status"
        ) {
            // Read polls do not grow the durable mutation-receipt history.
            return self
                .state
                .execute_peer_command(device, &command)
                .map(|result| json!({"status":"accepted","result":result}));
        }
        if let Some(database) = &self.database {
            if let Some(receipt) = database.receipt(device, command.command_id)? {
                if receipt.hash != command.hash() {
                    return Err("commandId was already used with a different payload.".into());
                }
                return Ok(json!(receipt));
            }
            database.begin()?;
            let capture = crate::outbox::Capture::begin();
            let mut receipt = Receipt {
                command_id: command.command_id,
                device_id: device.into(),
                hash: command.hash(),
                status: ReceiptStatus::Accepted,
                result: Value::Null,
                seq: self.seq + 1,
            };
            let result = if command.method.starts_with("peer.") {
                self.state.execute_peer_command(device, &command)
            } else if command.method == "devices.revoke" {
                command.params["deviceId"]
                    .as_str()
                    .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                    .ok_or_else(|| "A UUID deviceId is required.".to_owned())
                    .and_then(|id| database.revoke(id))
                    .map(|_| json!({}))
            } else {
                self.state.execute_command(&command)
            };
            match result {
                Ok(result) => receipt.result = result,
                Err(error) => {
                    receipt.status = ReceiptStatus::Rejected;
                    receipt.result = json!({"error":error});
                }
            }
            self.state.record_command(receipt.clone(), None);
            self.persist(Some(&receipt), capture.finish())?;
            return Ok(json!(receipt));
        }
        if let Some(receipt) = self
            .state
            .command_receipt(device, command.command_id)
            .cloned()
        {
            if receipt.hash != command.hash() {
                return Err("commandId was already used with a different payload.".into());
            }
            self.state.checkpoint_and_flush()?;
            return serde_json::to_value(receipt).map_err(|e| e.to_string());
        }
        self.seq += 1;
        let mut receipt = Receipt {
            command_id: command.command_id,
            device_id: device.into(),
            hash: command.hash(),
            status: ReceiptStatus::Interrupted,
            result: json!({"error":"Dispatch outcome is uncertain after restart. This intent will not be replayed."}),
            seq: self.seq,
        };
        // Commit intent before touching a harness. A crash never causes automatic redispatch.
        self.state
            .record_command(receipt.clone(), Some(command.clone()));
        self.state.checkpoint_and_flush()?;
        match self.state.execute_command(&command) {
            Ok(result) => {
                receipt.status = ReceiptStatus::Accepted;
                receipt.result = result;
            }
            Err(error) => {
                receipt.status = ReceiptStatus::Rejected;
                receipt.result = json!({"error":error});
            }
        }
        self.state.record_command(receipt.clone(), None);
        self.state.checkpoint_and_flush()?;
        self.last_projection = self.state.projection();
        serde_json::to_value(receipt).map_err(|e| e.to_string())
    }

    pub fn tick(&mut self) -> bool {
        if self.failure.is_none() && !self.maintenance.fenced() && !self.validation_hold {
            // Only the previous committed state may cause a remote effect.
            self.state.dispatch_peer_work();
        }
        let before = self.maintenance.status();
        if self.maintenance.advance(std::time::Instant::now()) {
            self.state.runtime_stop_requested = true;
        }
        if let Some(database) = &self.database {
            database.fence_dispatch(self.maintenance.fenced() || self.validation_hold);
        }
        if self.maintenance.fenced() || self.validation_hold {
            return before != self.maintenance.status();
        }
        if self.failure.is_some() {
            return false;
        }
        if let Some(database) = self.database.clone() {
            if let Err(error) = database.begin() {
                self.failure = Some(error);
                return false;
            }
            let capture = crate::outbox::Capture::begin();
            let changed = crate::model::poll(&mut self.state, 0.0, &mut rune::Tx::default())
                | self.state.collect_recovered_inputs();
            let effects = capture.finish();
            if !changed && effects.is_empty() && self.pending_effects.is_empty() {
                match database.finish_if_unchanged() {
                    Ok(true) => return before != self.maintenance.status(),
                    Ok(false) => {}
                    Err(error) => {
                        database.fail(error.clone());
                        self.failure = Some(error);
                        return false;
                    }
                }
            }
            let previous_seq = self.seq;
            // Stores may have staged memory, routine or completion changes even without
            // a visible projection change. Commit every reducer batch before dispatch.
            if let Err(error) = self.persist(None, effects) {
                database.fail(error.clone());
                self.state.storage_error = Some(error.clone());
                self.failure = Some(error);
            }
            return self.seq != previous_seq || before != self.maintenance.status();
        }
        let changed = crate::model::poll(&mut self.state, 0.0, &mut rune::Tx::default())
            | self.state.collect_recovered_inputs();
        if !changed {
            return before != self.maintenance.status();
        }
        let projection = self.state.projection();
        if projection == self.last_projection {
            return before != self.maintenance.status();
        }
        self.last_projection = projection;
        self.seq += 1;
        self.state.advance_server_sequence(self.seq);
        self.state.checkpoint();
        true
    }

    pub fn snapshot(&self) -> Value {
        let mut maintenance = self.maintenance.status();
        if self.state.runtime_stop_requested && !self.maintenance.fenced() {
            maintenance["phase"] = json!("stopping");
        }
        json!({"epoch":self.state.server_epoch(),"seq":self.seq,"state":self.last_projection,"historyCursor":null,"maintenance":maintenance})
    }

    fn maintenance_command(&mut self, command: &Command) -> Option<Result<Value, String>> {
        let now = std::time::Instant::now();
        let result = match command.method.as_str() {
            "server.status" => {
                let mut value = self.maintenance.status();
                value["ready"] = json!(true);
                value["validationHold"] = json!(self.validation_hold);
                value["dataDir"] = json!(self.state.data_dir);
                Ok(value)
            }
            "server.validateRestart" => {
                std::fs::remove_file(self.state.data_dir.join("update-validation"))
                    .map_err(|e| e.to_string())
                    .map(|()| {
                        self.validation_hold = false;
                        json!({"ready":true})
                    })
            }
            "server.prepareRestart" => {
                let clients = command.params["clientPids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok()))
                    .collect();
                if self.validation_hold {
                    Err("Restart validation is still pending.".into())
                } else if !self.pending_effects.is_empty() {
                    Err(
                        "Waiting for committed harness effects to settle. Retry preparation."
                            .into(),
                    )
                } else if let Some(reason) = self.state.update_restart_blocker() {
                    Err(reason.into())
                } else if self
                    .database
                    .as_ref()
                    .is_some_and(|db| db.fence_dispatch(true) != 0)
                {
                    // Do not silently interrupt an already claimed asynchronous effect.
                    self.database
                        .as_ref()
                        .expect("database")
                        .fence_dispatch(false);
                    Err("An external effect is still dispatching. Drain cancelled; retry after it settles.".into())
                } else {
                    match self.state.checkpoint_and_flush() {
                        Ok(()) => self.maintenance.prepare(None, clients, now),
                        Err(error) => Err(format!("Save failed. Drain cancelled: {error}")),
                    }
                }
            }
            "server.cancelRestart" => self.maintenance.cancel(&command.params),
            "server.clientReady" => self.maintenance.client_ready(&command.params),
            "server.commitRestart" => match self.state.checkpoint_and_flush() {
                Ok(()) => self.maintenance.commit(&command.params, now),
                Err(error) => {
                    let _ = self.maintenance.cancel(&command.params);
                    Err(format!("Save failed. Drain cancelled: {error}"))
                }
            },
            "server.stop" => {
                if self.maintenance.fenced() {
                    Err("An update is draining. Cancel it before stopping the server.".into())
                } else {
                    self.state.checkpoint_and_flush().map(|()| {
                        self.state.runtime_stop_requested = true;
                        json!({"stopping":true})
                    })
                }
            }
            _ => return None,
        };
        if let Some(database) = &self.database {
            database.fence_dispatch(self.maintenance.fenced() || self.validation_hold);
        }
        Some(result)
    }

    pub fn failure(&self) -> Option<String> {
        self.failure
            .clone()
            .or_else(|| self.database.as_ref().and_then(|db| db.failure()))
    }

    fn persist(
        &mut self,
        receipt: Option<&Receipt>,
        effects: Vec<crate::outbox::Effect>,
    ) -> Result<(), String> {
        let database = self.database.clone().ok_or("No runtime database.")?;
        let result = (|| {
            self.state.checkpoint_and_flush()?;
            let projection = self.state.projection();
            if receipt.is_none()
                && effects.is_empty()
                && self.pending_effects.is_empty()
                && projection == self.last_projection
                && database.finish_if_unchanged()?
            {
                return Ok(());
            }
            self.seq = self
                .seq
                .checked_add(1)
                .filter(|seq| *seq <= i64::MAX as u64)
                .ok_or("Server sequence exhausted.")?;
            self.state.advance_server_sequence(self.seq);
            self.state.checkpoint_and_flush()?;
            let patch = crate::protocol::diff(
                &crate::protocol::compact(&self.last_projection),
                &crate::protocol::compact(&projection),
            );
            database.commit(self.seq, &projection, &patch, receipt, &effects)?;
            self.last_projection = projection;
            self.pending_effects.extend(effects);
            if self.maintenance.fenced() || self.validation_hold {
                return Ok(());
            }
            crate::outbox::drain(&database, std::mem::take(&mut self.pending_effects))
        })();
        if let Err(error) = &result {
            database.fail(error.clone());
            self.failure = Some(error.clone());
        }
        result
    }

    pub fn lookup_command(&self, device: &str, params: &Value) -> Result<Value, String> {
        if let Some(database) = &self.database {
            let id = serde_json::from_value(params["commandId"].clone())
                .map_err(|_| "Invalid commandId.")?;
            return database
                .receipt(device, id)?
                .map(|r| json!(r))
                .ok_or("No receipt for this device and commandId.".into());
        }
        self.state.lookup_command(device, params)
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if let Some(database) = self.database.clone() {
            database.begin()?;
        }
        let capture = self
            .database
            .as_ref()
            .map(|_| crate::outbox::Capture::begin());
        self.state.begin_runtime_shutdown();
        if let Some(capture) = capture {
            self.persist(None, capture.finish())?;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        loop {
            if let Some(database) = &self.database {
                database.begin()?;
            }
            let capture = self
                .database
                .as_ref()
                .map(|_| crate::outbox::Capture::begin());
            let running = self.state.drain_runtime_shutdown();
            if let Some(capture) = capture {
                self.persist(None, capture.finish())?;
            }
            if !running || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(TICK);
        }
        if let Some(database) = &self.database {
            database.begin()?;
        }
        let capture = self
            .database
            .as_ref()
            .map(|_| crate::outbox::Capture::begin());
        self.state.stop_runtime();
        let result = if let Some(capture) = capture {
            self.persist(None, capture.finish())
        } else {
            self.state.checkpoint_and_flush()
        };
        if result.is_ok() {
            self.state.mark_clean_quit();
        }
        result
    }
}

pub const TICK: Duration = Duration::from_millis(25);

#[cfg(test)]
mod tests {
    use super::*;
    fn runtime() -> RuntimeState {
        let mut state = State::headless(crate::store::directory().unwrap());
        state.load_global_for_test();
        RuntimeState::from_state(state)
    }
    fn durable() -> RuntimeState {
        let runtime = RuntimeState::open(
            crate::store::directory().unwrap(),
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
        runtime
            .database
            .as_ref()
            .unwrap()
            .issue_device("maintainer", "fixture-token")
            .unwrap();
        runtime
    }
    #[test]
    fn outbox_pauses_during_drain_and_resumes_only_after_cancellation() {
        let mut rt = durable();
        let prepared = rt
            .command(
                "maintainer",
                Command::new("server.prepareRestart", json!({})),
            )
            .unwrap();
        let database = rt.database.clone().unwrap();
        assert!(database.claim_dispatch().unwrap().is_none());
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        crate::outbox::send(&sender, json!({"type":"prompt", "id":"paused"})).unwrap();
        rt.persist(None, capture.finish()).unwrap();
        assert!(receiver.try_recv().is_err());
        assert!(!rt.tick());
        assert!(receiver.try_recv().is_err());
        rt.command("maintainer", Command::new("server.cancelRestart", prepared))
            .unwrap();
        rt.tick();
        assert_eq!(receiver.try_recv().unwrap()["id"], "paused");
        assert!(rt.pending_effects.is_empty());
    }
    #[test]
    fn claimed_effect_cancels_preparation_until_it_settles() {
        let mut rt = durable();
        let database = rt.database.clone().unwrap();
        let dispatch = database.claim_dispatch().unwrap().unwrap();
        let error = rt
            .command(
                "maintainer",
                Command::new("server.prepareRestart", json!({})),
            )
            .unwrap_err();
        assert!(error.contains("Drain cancelled"));
        assert!(!rt.maintenance.fenced());
        drop(dispatch);
        rt.command(
            "maintainer",
            Command::new("server.prepareRestart", json!({})),
        )
        .unwrap();
        assert!(database.claim_dispatch().unwrap().is_none());
    }
    #[test]
    fn unknown_and_revoked_devices_cannot_drain_or_stop() {
        let mut rt = durable();
        rt.database
            .as_ref()
            .unwrap()
            .issue_device("revoked", "old-token")
            .unwrap();
        rt.database.as_ref().unwrap().revoke("revoked").unwrap();
        for device in ["unknown", "revoked"] {
            for method in [
                "server.prepareRestart",
                "server.stop",
                "server.validateRestart",
            ] {
                assert!(
                    rt.command(device, Command::new(method, json!({})))
                        .unwrap_err()
                        .contains("Authentication denied")
                );
                assert!(!rt.maintenance.fenced());
                assert!(!rt.state.runtime_stop_requested);
            }
        }
    }
    #[test]
    fn restart_validation_holds_startup_effects_until_authenticated_validation() {
        let directory = crate::store::directory().unwrap();
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("update-validation"), b"paused").unwrap();
        let mut rt = durable();
        let database = rt.database.clone().unwrap();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        crate::outbox::send(&sender, json!({"type":"prompt", "id":"validated"})).unwrap();
        rt.persist(None, capture.finish()).unwrap();
        assert!(database.claim_dispatch().unwrap().is_none());
        assert!(receiver.try_recv().is_err());
        rt.command(
            "maintainer",
            Command::new("server.validateRestart", json!({})),
        )
        .unwrap();
        rt.tick();
        assert_eq!(receiver.try_recv().unwrap()["id"], "validated");
    }
    #[test]
    fn drain_fences_mutations_and_save_failure_resumes_without_stopping() {
        let mut rt = runtime();
        let prepared = rt
            .command(
                "maintainer",
                Command::new("server.prepareRestart", json!({})),
            )
            .unwrap();
        assert!(
            rt.command("device", Command::new("thread.create", json!({})))
                .unwrap_err()
                .contains("draining")
        );
        assert!(!rt.state.runtime_stop_requested);
        rt.state.ui_store.take();
        assert!(
            rt.command("maintainer", Command::new("server.commitRestart", prepared))
                .unwrap_err()
                .contains("Save failed")
        );
        assert!(!rt.maintenance.fenced());
        assert!(!rt.state.runtime_stop_requested);
    }
    #[test]
    fn prepare_refuses_active_configuring_approval_and_queue_without_aborting() {
        let mut rt = runtime();
        for kind in ["active", "configuring", "approval", "queue"] {
            {
                let t = rt.state.current_mut();
                t.status = crate::model::Status::Ready;
                t.configuring = false;
                t.request = None;
                t.native.projected_queued = false;
                match kind {
                    "active" => t.status = crate::model::Status::Running,
                    "configuring" => t.configuring = true,
                    "approval" => t.request = Some(json!({"id":"pending"})),
                    "queue" => t.native.projected_queued = true,
                    _ => unreachable!(),
                }
            }
            assert!(
                rt.command(
                    "maintainer",
                    Command::new("server.prepareRestart", json!({}))
                )
                .is_err(),
                "{kind}"
            );
            assert!(!rt.maintenance.fenced());
            assert!(!rt.state.runtime_stop_requested);
        }
    }
}
