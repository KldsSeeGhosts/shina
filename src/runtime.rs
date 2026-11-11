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
}

impl RuntimeState {
    pub fn open(directory: PathBuf, wake: rune::Waker) -> Result<Self, String> {
        let mut state = State::headless(directory);
        state.setup_runtime(wake)?;
        Ok(Self::from_state(state))
    }
    pub fn from_state(state: State) -> Self {
        Self {
            seq: state.server_sequence(),
            last_projection: state.projection(),
            state,
        }
    }

    pub fn command(&mut self, device: &str, command: Command) -> Result<Value, String> {
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
        let changed = crate::model::poll(&mut self.state, 0.0, &mut rune::Tx::default())
            | self.state.collect_recovered_inputs();
        if !changed {
            return false;
        }
        let projection = self.state.projection();
        if projection == self.last_projection {
            return false;
        }
        self.last_projection = projection;
        self.seq += 1;
        self.state.advance_server_sequence(self.seq);
        self.state.checkpoint();
        true
    }

    pub fn snapshot(&self) -> Value {
        json!({"epoch":self.state.server_epoch(),"seq":self.seq,"state":self.last_projection,"historyCursor":null})
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
