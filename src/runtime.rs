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
    pub database: Option<crate::database::Database>,
    failure: Option<String>,
}

impl RuntimeState {
    pub fn open(directory: PathBuf, wake: rune::Waker) -> Result<Self, String> {
        let database = crate::database::Database::open(&directory)?;
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
            seq: state.server_sequence(),
            last_projection: state.projection(),
            state,
            database: None,
            failure: None,
        }
    }

    pub fn command(&mut self, device: &str, command: Command) -> Result<Value, String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
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
            let result = if command.method == "devices.revoke" {
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
            if !changed && effects.is_empty() {
                match database.finish_if_unchanged() {
                    Ok(true) => return false,
                    Ok(false) => {}
                    Err(error) => {
                        database.fail(error.clone());
                        self.failure = Some(error);
                        return false;
                    }
                }
            }
            let before = self.seq;
            // Stores may have staged memory, routine or completion changes even without
            // a visible projection change. Commit every reducer batch before dispatch.
            if let Err(error) = self.persist(None, effects) {
                database.fail(error.clone());
                self.state.storage_error = Some(error.clone());
                self.failure = Some(error);
            }
            return self.seq != before;
        }
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
            crate::outbox::drain(&database, effects)
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
