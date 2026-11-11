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
}

impl RuntimeState {
    pub fn open(directory: PathBuf, wake: rune::Waker) -> Result<Self, String> {
        let mut state = State::headless(directory);
        state.setup_runtime(wake)?;
        Ok(Self::from_state(state))
    }
    pub fn from_state(state: State) -> Self {
        Self {
            validation_hold: state.data_dir.join("update-validation").exists(),
            seq: state.server_sequence(),
            last_projection: state.projection(),
            state,
            maintenance: crate::maintenance::Maintenance::default(),
        }
    }

    pub fn command(&mut self, device: &str, command: Command) -> Result<Value, String> {
        if let Some(result) = self.maintenance_command(&command) {
            return result;
        }
        if self.maintenance.fenced() || self.validation_hold {
            return Err(
                "Server is draining for an update. Retry after restart or cancellation.".into(),
            );
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
        let before = self.maintenance.status();
        if self.maintenance.advance(std::time::Instant::now()) {
            self.state.runtime_stop_requested = true;
        }
        if self.maintenance.fenced() || self.validation_hold {
            return false;
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
                if let Some(reason) = self.state.update_restart_blocker() {
                    Err(reason.into())
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
        Some(result)
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        self.state.begin_runtime_shutdown();
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while self.state.drain_runtime_shutdown() && std::time::Instant::now() < deadline {
            std::thread::sleep(TICK);
        }
        self.state.stop_runtime();
        let result = self.state.checkpoint_and_flush();
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
