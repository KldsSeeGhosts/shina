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

struct PreparingCall {
    call: Call,
    receiver: crate::model::Preparation,
    caller: Option<(usize, Option<u64>)>,
}

pub struct RuntimeState {
    pub state: State,
    pub seq: u64,
    last_projection: Value,
    pub maintenance: crate::maintenance::Maintenance,
    pub updates: crate::fleet_update::ServerUpdates,
    validation_hold: bool,
    pub database: Option<crate::database::Database>,
    failure: Option<String>,
    pending_effects: Vec<crate::outbox::Effect>,
    committed_history: Vec<Value>,
    document_revision: u64,
    preparing_calls: Vec<PreparingCall>,
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
        if let Some(outcome) = &runtime.updates.outcome {
            runtime
                .database
                .as_ref()
                .unwrap()
                .update_outcome(&json!(outcome))?;
        }
        if let Some((id, tag)) = runtime.database.as_ref().unwrap().unfinished_update()?
            && runtime.updates.id.as_deref() != Some(id.as_str())
        {
            // No detached helper owns this accepted attempt. Never repeat its work.
            let outcome = crate::fleet_update::Outcome {
                update_id: id,
                tag,
                success: false,
                reason: Some(
                    "Update preparation was interrupted before restart. Retry explicitly.".into(),
                ),
            };
            crate::fleet_update::save_outcome(&runtime.state.data_dir, &outcome)?;
            runtime
                .database
                .as_ref()
                .unwrap()
                .update_outcome(&json!(outcome))?;
            runtime.updates.outcome = Some(outcome);
            runtime.updates.phase = "failed".into();
        }
        let effects = capture.finish();
        runtime.persist(None, effects)?;
        Ok(runtime)
    }
    pub fn from_state(state: State) -> Self {
        Self {
            updates: crate::fleet_update::ServerUpdates::new(&state.data_dir),
            validation_hold: state.data_dir.join("update-validation").exists(),
            seq: state.server_sequence(),
            last_projection: state.projection(),
            state,
            maintenance: crate::maintenance::Maintenance::default(),
            database: None,
            failure: None,
            pending_effects: Vec::new(),
            committed_history: Vec::new(),
            document_revision: 0,
            preparing_calls: Vec::new(),
        }
    }

    /// A cold Git lookup never holds the serialized runtime. Authorization and
    /// receipts are checked again when the unchanged original call resumes.
    pub fn accept_call(&mut self, call: Call) {
        let preparation = (|| {
            if let Some(db) = &self.database {
                db.authorize(&call.device, &call.command.method, &call.command.params)?;
                if db.receipt(&call.device, call.command.command_id)?.is_some() {
                    return Ok(None);
                }
            }
            if self.maintenance.fenced() || self.validation_hold {
                return Ok(None);
            }
            Ok::<_, String>(
                self.state
                    .prepare_peer_call(&call.command.method, &call.command.params),
            )
        })();
        match preparation {
            Ok(Some(receiver)) => {
                let caller = (call.command.method == "task.delegate")
                    .then(|| {
                        call.command.params["threadId"].as_u64().map(|id| {
                            let id = id as usize;
                            (id, self.state.delegation_generation(id))
                        })
                    })
                    .flatten();
                self.preparing_calls.push(PreparingCall {
                    call,
                    receiver,
                    caller,
                });
            }
            Ok(None) => {
                let result = if call.command.method == "command.get" {
                    self.lookup_command(&call.device, &call.command.params)
                } else {
                    self.command(&call.device, call.command)
                };
                let _ = call.reply.send(result);
            }
            Err(error) => {
                let _ = call.reply.send(Err(error));
            }
        }
    }

    pub fn poll_prepared_calls(&mut self) -> bool {
        let mut changed = false;
        for pending in std::mem::take(&mut self.preparing_calls) {
            let result = match pending.receiver.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.preparing_calls.push(pending);
                    continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err("Repository identity worker stopped.".into())
                }
            };
            let PreparingCall { call, caller, .. } = pending;
            if caller
                .is_some_and(|(id, generation)| self.state.delegation_generation(id) != generation)
            {
                let _ = call.reply.send(Err("Delegation cancelled because the calling thread stopped during repository lookup.".into()));
                changed = true;
                continue;
            }
            if let Err(error) = self.state.finish_peer_preparation(result) {
                let _ = call.reply.send(Err(error));
            } else {
                self.accept_call(call);
            }
            changed = true;
        }
        changed
    }

    pub fn command(&mut self, device: &str, command: Command) -> Result<Value, String> {
        self.state.persistence_scope = None;
        if let Some(error) = self.failure() {
            return Err(error);
        }
        if let Some(database) = &self.database
            && !database.device_active(device)?
        {
            return Err("Authentication denied. This device is unknown or revoked.".into());
        }
        if let Some(database) = &self.database {
            if let Err(error) = database.authorize(device, &command.method, &command.params) {
                if command.method == "server.requestUpdate" {
                    database.audit_update_refusal(device, &error)?;
                }
                return Err(error);
            }
        }
        if command.method == "server.updateStatus" {
            if self.updates.poll(&self.state.data_dir) {
                self.record_update()?;
                if let Some(db) = &self.database {
                    db.begin()?;
                    self.persist(None, Vec::new())?;
                }
            }
            return Ok(self.updates.status(
                self.state.update_restart_blocker(),
                &self.maintenance.status(),
            ));
        }
        if let Some(result) = self.maintenance_command(&command) {
            return result;
        }
        if (self.maintenance.fenced() || self.validation_hold)
            && command.method != "server.requestUpdate"
        {
            return Err(
                "Server is draining for an update. Retry after restart or cancellation.".into(),
            );
        }
        if matches!(
            command.method.as_str(),
            "peer.capabilities" | "peer.thread.read" | "peer.thread.list"
        ) {
            // Transcript reads do not acknowledge results or grow mutation receipts.
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
            } else if matches!(
                command.method.as_str(),
                "devices.grant" | "devices.revokeGrant"
            ) {
                let id = command.params["deviceId"].as_str().unwrap_or("");
                if command.params["grant"] != "update" {
                    Err("Only the update grant can be changed here.".into())
                } else if !database.device_active(id)? {
                    Err("Unknown or revoked device.".into())
                } else {
                    Ok(json!({"grantDevice":id,"enabled":command.method == "devices.grant"}))
                }
            } else if command.method == "server.requestUpdate" {
                let now = crate::lifecycle::now();
                if self.maintenance.fenced() || self.validation_hold {
                    Err(
                        "Server is draining for an update. Retry after restart or cancellation."
                            .into(),
                    )
                } else if let Some(blocker) = self.state.update_restart_blocker() {
                    Err(blocker.into())
                } else if !self.updates.capable {
                    Err("This install cannot self-update.".into())
                } else if self.updates.active() {
                    Err("An update is already in progress.".into())
                } else if database.update_rate_limited(device, now)? {
                    Err("One accepted update per device per ten minutes. Try again later.".into())
                } else {
                    Ok(
                        json!({"updateAccepted":true,"updateId":uuid::Uuid::new_v4(),"requestedAt":now}),
                    )
                }
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
            if command.method == "server.requestUpdate" {
                receipt.result["requestedAt"] = json!(crate::lifecycle::now());
                receipt.result["updateAuditId"] = receipt
                    .result
                    .get("updateId")
                    .cloned()
                    .unwrap_or_else(|| json!(uuid::Uuid::new_v4()));
            }
            self.state.record_command(receipt.clone(), None);
            self.persist(Some(&receipt), capture.finish())?;
            if command.method == "server.requestUpdate" && receipt.result["updateAccepted"] == true
            {
                self.updates.start(
                    self.state.data_dir.clone(),
                    receipt.result["updateId"].as_str().unwrap().into(),
                );
            }
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

    fn record_update(&self) -> Result<(), String> {
        if let Some(db) = &self.database {
            if self.updates.active()
                && let Some(id) = &self.updates.id
            {
                db.update_progress(id, &self.updates.tag, &self.updates.phase)?;
            }
            if let Some(outcome) = &self.updates.outcome {
                db.update_outcome(&json!(outcome))?;
            }
        }
        Ok(())
    }
    pub fn tick(&mut self) -> bool {
        let prepared_changed = self.poll_prepared_calls();
        if self.failure.is_none() && !self.maintenance.fenced() && !self.validation_hold {
            // Only the previous committed state may cause a remote effect.
            self.state.dispatch_peer_work();
        }
        let update_changed = self.updates.poll(&self.state.data_dir) | prepared_changed;
        if update_changed && let Err(error) = self.record_update() {
            self.failure = Some(error);
        }
        let before = self.maintenance.status();
        if self.maintenance.advance(std::time::Instant::now()) {
            self.state.runtime_stop_requested = true;
        }
        if let Some(database) = &self.database {
            database.fence_dispatch(self.maintenance.fenced() || self.validation_hold);
        }
        if self.maintenance.fenced() || self.validation_hold {
            return update_changed || before != self.maintenance.status();
        }
        if self.failure.is_some() {
            return false;
        }
        if let Some(database) = self.database.clone() {
            let documents_changed = database.document_revision() != self.document_revision;
            if let Err(error) = database.begin() {
                self.failure = Some(error);
                return false;
            }
            let capture = crate::outbox::Capture::begin();
            self.state.persistence_scope = Some(Default::default());
            let changed = crate::model::poll(&mut self.state, 0.0, &mut rune::Tx::default());
            let recovered = self.state.collect_recovered_inputs();
            if recovered || update_changed {
                self.state.persistence_scope = None;
            }
            let changed = changed | recovered;
            let effects = capture.finish();
            if !changed
                && !update_changed
                && !documents_changed
                && effects.is_empty()
                && self.pending_effects.is_empty()
            {
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
            return prepared_changed
                || self.seq != previous_seq
                || before != self.maintenance.status();
        }
        let changed = crate::model::poll(&mut self.state, 0.0, &mut rune::Tx::default())
            | self.state.collect_recovered_inputs();
        if !changed {
            return prepared_changed || before != self.maintenance.status();
        }
        let projection = self.state.projection();
        if projection == self.last_projection {
            return prepared_changed || before != self.maintenance.status();
        }
        self.last_projection = projection;
        self.seq += 1;
        self.state.advance_server_sequence(self.seq);
        self.state.checkpoint();
        true
    }

    pub fn snapshot(&self) -> Value {
        let mut value = self.notification();
        let maintenance = value["maintenance"].clone();
        let mut state = self.last_projection.clone();
        state["updateStatus"] = self
            .updates
            .status(self.state.update_restart_blocker(), &maintenance);
        state["updateNotice"] = json!(self.updates.outcome);
        value["state"] = state;
        value
    }
    /// Watchers need the committed cursor, not another copy of every transcript.
    pub fn notification(&self) -> Value {
        let mut maintenance = self.maintenance.status();
        if self.state.runtime_stop_requested && !self.maintenance.fenced() {
            maintenance["phase"] = json!("stopping");
        }
        json!({"epoch":self.state.server_epoch(),"seq":self.seq,"historyCursor":null,"maintenance":maintenance})
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

    pub fn take_committed_history(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.committed_history)
    }

    fn persist(
        &mut self,
        receipt: Option<&Receipt>,
        effects: Vec<crate::outbox::Effect>,
    ) -> Result<(), String> {
        let database = self.database.clone().ok_or("No runtime database.")?;
        let result = (|| {
            let document_revision = database.document_revision();
            if effects.len() > 0
                && self
                    .state
                    .persistence_scope
                    .as_ref()
                    .is_some_and(|s| s.is_empty())
            {
                self.state.persistence_scope = None;
            }
            let mut index = self
                .state
                .snapshot_scoped(self.state.persistence_scope.as_ref());
            let next = self
                .seq
                .checked_add(1)
                .filter(|seq| *seq <= i64::MAX as u64)
                .ok_or("Server sequence exhausted.")?;
            for view in index["threadViews"].as_array_mut().unwrap() {
                let before = self.last_projection["threads"]
                    .as_array()
                    .and_then(|threads| threads.iter().find(|t| t["id"] == view["id"]))
                    .unwrap_or(&Value::Null);
                crate::history::stamp(before, view, next);
            }
            let mut projection = self.state.projection_from_snapshot(index.clone());
            if projection["_partial"] == true && document_revision != self.document_revision {
                projection.as_object_mut().unwrap().extend(
                    self.state
                        .document_projection()
                        .as_object()
                        .unwrap()
                        .clone(),
                );
            }
            {
                projection["updateStatus"] = self.updates.status(
                    self.state.update_restart_blocker(),
                    &self.maintenance.status(),
                );
                projection["updateNotice"] = json!(self.updates.outcome);
            }
            let previous_threads = if projection["_partial"] == true {
                json!(
                    projection["threads"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|t| self.last_projection["threads"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|old| old["id"] == t["id"])
                            .cloned())
                        .collect::<Vec<_>>()
                )
            } else {
                self.last_projection["threads"].clone()
            };
            let previous = json!({"seq":self.seq,"state":{"threads":previous_threads}});
            let history = crate::history::Streams::changes(
                &previous,
                &json!({"seq":next,"state":{"threads":projection["threads"]}}),
            );
            database.stage_history(history.clone());
            let patch = update_projection(&mut self.last_projection, &projection);
            if receipt.is_none()
                && effects.is_empty()
                && self.pending_effects.is_empty()
                && patch.as_array().is_some_and(Vec::is_empty)
                && database.finish_if_unchanged()?
            {
                self.document_revision = document_revision;
                return Ok(());
            }
            self.seq = next;
            self.state.advance_server_sequence(self.seq);
            index["serverSeq"] = json!(self.seq);
            database.write("index.json", index)?;
            database.write(
                "server-inputs.json",
                self.state
                    .runtime_input_snapshot(self.state.persistence_scope.as_ref()),
            )?;
            database.commit(self.seq, &projection, &patch, receipt, &effects)?;
            self.document_revision = document_revision;
            self.committed_history.extend(history);
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
        self.committed_history.clear();
        self.state.persistence_scope = None;
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
            self.committed_history.clear();
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
                self.committed_history.clear();
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
            self.committed_history.clear();
            self.state.mark_clean_quit();
        }
        result
    }
}

/// Apply only changed scopes. Paths retain their workspace indices on the wire.
fn update_projection(current: &mut Value, delta: &Value) -> Value {
    if delta["_partial"] != true {
        let patch = crate::protocol::diff(
            &crate::protocol::compact(current),
            &crate::protocol::compact(delta),
        );
        *current = delta.clone();
        return patch;
    }
    let mut patches = Vec::new();
    for (key, value) in delta.as_object().unwrap() {
        if key == "_partial" {
            continue;
        }
        if key == "index" {
            let nested = update_projection(&mut current["index"], value);
            for mut patch in nested.as_array().unwrap().clone() {
                patch["path"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, json!("index"));
                patches.push(patch);
            }
        } else if key == "threads" {
            for thread in value.as_array().unwrap() {
                let threads = current["threads"].as_array_mut().unwrap();
                let index = threads
                    .iter()
                    .position(|t| t["id"] == thread["id"])
                    .expect("dirty existing thread");
                let compact = |t: &Value| {
                    crate::protocol::compact(&json!({"threads":[t]}))["threads"][0].clone()
                };
                let change = if thread.get("messages").is_some() {
                    crate::protocol::diff(&compact(&threads[index]), &compact(thread))
                } else {
                    crate::protocol::diff(&threads[index], thread)
                };
                for mut patch in change.as_array().unwrap().clone() {
                    let path = patch["path"].as_array_mut().unwrap();
                    path.insert(0, json!(index.to_string()));
                    path.insert(0, json!("threads"));
                    patches.push(patch);
                }
                threads[index] = thread.clone();
            }
        } else {
            let change = crate::protocol::diff(&current[key], value);
            for mut patch in change.as_array().unwrap().clone() {
                patch["path"].as_array_mut().unwrap().insert(0, json!(key));
                patches.push(patch);
            }
            current[key] = value.clone();
        }
    }
    json!(patches)
}

pub const TICK: Duration = Duration::from_millis(25);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_writes_refresh_global_fields_without_capturing_thread_views() {
        for staged in [false, true] {
            let directory = crate::store::directory()
                .unwrap()
                .join(uuid::Uuid::new_v4().to_string());
            let mut runtime = RuntimeState::open(directory, std::sync::Arc::new(|| {})).unwrap();
            let database = runtime.database.clone().unwrap();
            let before = runtime.seq;
            if staged {
                database.begin().unwrap();
            }
            let mut profile = runtime.state.user_profile();
            profile.name = if staged {
                "Staged profile"
            } else {
                "Background profile"
            }
            .into();
            runtime.state.set_user_profile(profile.clone()).unwrap();
            if !staged {
                database.begin().unwrap();
            }
            runtime.state.persistence_scope = Some(Default::default());
            runtime.persist(None, vec![]).unwrap();
            assert!(runtime.seq > before);
            assert!(runtime.state.persistence_scope.as_ref().unwrap().is_empty());
            assert_eq!(runtime.snapshot()["state"]["profile"]["name"], profile.name);
            assert_eq!(
                database.projection().unwrap().unwrap()["profile"]["name"],
                profile.name
            );
            let committed = runtime.seq;
            database.begin().unwrap();
            runtime.persist(None, vec![]).unwrap();
            assert_eq!(
                runtime.seq, committed,
                "an unchanged document must not keep advancing the sequence"
            );
            runtime.shutdown().unwrap();
        }
    }
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
    fn row_storage_committed_snapshot_restores_message_draft_and_queue_context() {
        let mut runtime = durable();
        let database = runtime.database.clone().unwrap();
        let directory = runtime.state.data_dir.clone();
        let record = crate::composer_context::thread_context_record("local", 1, "Reference");
        let text = format!(
            "See [Reference]({})",
            crate::composer_context::format_composer_context_href("thread", &record.context_id)
        );
        let context = crate::composer_context::MessageContext {
            version: 1,
            records: vec![record.clone()],
        };
        let thread = runtime.state.current_mut();
        thread.messages.push(crate::model::Message {
            id: "context-message".into(),
            role: "You",
            text: text.clone(),
            images: vec![],
            context: Some(context.clone()),
        });
        thread.draft = crate::composer_rich::draft_from_canonical(&text);
        thread.draft_context = vec![record.clone()];
        let snapshot = crate::composer_rich::Snapshot::capture_with_context(
            &thread.draft,
            &[],
            &thread.draft_context,
        );
        let mut prompt = crate::attachments::Prompt::new(text.clone(), vec![]);
        prompt.context = Some(context.clone());
        prompt.draft = Some(snapshot);
        thread
            .native
            .queue
            .push_back(crate::model::pi_session::QueuedMessage { id: 7, prompt });
        database.begin().unwrap();
        database
            .write("ui.json", runtime.state.ui_snapshot())
            .unwrap();
        runtime.persist(None, vec![]).unwrap();
        let committed = database.committed_snapshot().unwrap();
        assert_eq!(
            committed["state"]["threads"][0]["messages"][0]["id"],
            "context-message"
        );
        assert_eq!(
            committed["state"]["threads"][0]["messages"][0]["context"],
            json!(context)
        );
        drop(runtime);
        drop(database);

        let database = crate::database::Database::open(&directory).unwrap();
        let committed = database.committed_snapshot().unwrap();
        assert_eq!(
            committed["state"]["threads"][0]["messages"][0]["context"],
            json!(context)
        );
        let mut restored = State::headless(directory);
        restored.setup_runtime(std::sync::Arc::new(|| {})).unwrap();
        restored
            .restore_ui(&database.read("ui.json").unwrap().unwrap())
            .unwrap();
        let thread = restored.current();
        assert_eq!(thread.messages[0].id, "context-message");
        assert_eq!(thread.messages[0].context, Some(context.clone()));
        assert_eq!(thread.draft_context, vec![record]);
        assert_eq!(crate::composer_rich::markdown(&thread.draft), text);
        assert_eq!(thread.native.queue.len(), 1);
        assert_eq!(thread.native.queue[0].prompt.context, Some(context.clone()));
        assert_eq!(
            thread.native.queue[0]
                .prompt
                .draft
                .as_ref()
                .unwrap()
                .context,
            context.records
        );
    }

    #[test]
    fn startup_reports_an_accepted_update_interrupted_before_helper_handoff() {
        let directory = crate::store::directory().unwrap();
        let db = crate::database::Database::open(&directory).unwrap();
        db.issue_device("interrupted-device", "not-a-real-token")
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let connection = rusqlite::Connection::open(directory.join("state.sqlite")).unwrap();
        connection.execute("INSERT INTO update_audit(update_id,device_id,requested_at,accepted,tag,outcome) VALUES(?1,'interrupted-device',1,1,'v99.0.0',?2)",
            rusqlite::params![id,json!({"updateAccepted":true}).to_string()]).unwrap();
        drop(connection);
        drop(db);
        let mut runtime =
            RuntimeState::open(directory.clone(), std::sync::Arc::new(|| {})).unwrap();
        let outcome = runtime.updates.outcome.as_ref().unwrap();
        assert_eq!(outcome.update_id, id);
        assert_eq!(outcome.tag, "v99.0.0");
        assert!(!outcome.success);
        assert!(outcome.reason.as_deref().unwrap().contains("interrupted"));
        assert!(
            runtime
                .database
                .as_ref()
                .unwrap()
                .unfinished_update()
                .unwrap()
                .is_none()
        );
        assert_eq!(
            crate::fleet_update::read_outcome(&directory)
                .unwrap()
                .update_id,
            id
        );
        runtime.shutdown().unwrap();
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
