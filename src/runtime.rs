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
#[path = "provider_runtime.rs"]
mod provider_runtime;

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
    pub providers: crate::provider_update::ProviderUpdates,
    provider_starts: Vec<provider_runtime::ProviderStart>,
    validation_hold: bool,
    pub database: Option<crate::database::Database>,
    failure: Option<String>,
    pending_effects: Vec<crate::outbox::Effect>,
    committed_history: Vec<Value>,
    document_revision: u64,
    preparing_calls: Vec<PreparingCall>,
}

impl RuntimeState {
    #[cfg(test)]
    pub(crate) fn persist_for_test(
        &mut self,
        effects: Vec<crate::outbox::Effect>,
    ) -> Result<(), String> {
        self.persist(None, effects)
    }

    /// Live-fixture pump: the production tick's remote-task half, without the
    /// harness spawn or global MCP drain that server tests cannot run.
    #[cfg(test)]
    pub(crate) fn test_peer_pump(&mut self) -> bool {
        self.state.dispatch_peer_work();
        let changed = self.state.poll_remote_tasks_for_test();
        if changed {
            self.last_projection = self.state.projection();
        }
        changed
    }

    /// Live-fixture equivalent of the model poll's `server.next_request()`
    /// drain: the same `mcp_request` dispatch production runs.
    #[cfg(test)]
    pub(crate) fn test_inject_mcp(&mut self, request: crate::mcp::Request) {
        let baseline = self.state.execution_baseline();
        let initiator = self
            .state
            .execution_initiator_for_thread(request.thread)
            .unwrap_or_else(|| self.state.identity.controller_id.clone());
        self.state.mcp_request(request);
        if let Err(error) = self
            .state
            .prepare_execution_after_command(&baseline, &initiator)
        {
            self.failure = Some(format!("Execution fence failed before effects: {error}"));
        }
        self.last_projection = self.state.projection();
    }

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
        runtime
            .state
            .settle_pending_prompt_effects(&database.undispatched_prompt_runs()?);
        runtime.database = Some(database);
        runtime.state.refresh_execution(crate::lifecycle::now());
        runtime.state.mark_execution_restart_uncertain();
        runtime.reconcile_provider_intents()?;
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
            if runtime.updates.id.is_none() {
                runtime.updates.outcome = Some(outcome);
                runtime.updates.phase = "failed".into();
            }
        }
        let effects = capture.finish();
        runtime.persist(None, effects)?;
        Ok(runtime)
    }
    pub fn from_state(mut state: State) -> Self {
        let providers = crate::provider_update::ProviderUpdates::new(&state.data_dir);
        for driver in ["codex", "claude"] {
            if !providers.admits(driver) {
                state.hold_provider(driver);
            }
        }
        Self {
            updates: crate::fleet_update::ServerUpdates::new(&state.data_dir),
            providers,
            provider_starts: Vec::new(),
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
            if command.params["refresh"] == true {
                self.updates.release.refresh(true);
            }
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
        if command.method == "providers.refresh" {
            if !command
                .params
                .as_object()
                .is_some_and(|args| args.is_empty())
            {
                return Err("providers.refresh accepts no arguments.".into());
            }
            self.providers.refresh(true);
            return Ok(json!({"providers":self.provider_statuses()}));
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
        if self.database.is_none()
            && matches!(
                command.method.as_str(),
                "peer.thread.launch"
                    | "peer.thread.send"
                    | "peer.thread.stop"
                    | "peer.execution.fence"
            )
        {
            return Err(
                "This file-only profile cannot safely bind an execution lease or fence a peer run. Use a durable server profile."
                    .into(),
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
        if command.method == "task.plan" {
            // A plan read is a live view, so it neither grows receipts nor commits.
            return self
                .state
                .execute_command(&command)
                .map(|result| json!({"status":"accepted","result":result}));
        }
        if let Some(database) = &self.database {
            if let Some(receipt) = database.receipt(device, command.command_id)? {
                if receipt.hash != command.hash() {
                    return Err("commandId was already used with a different payload.".into());
                }
                return Ok(json!(receipt));
            }
            self.execution_preflight(device, &command)?;
            database.begin()?;
            let execution_baseline = self.state.execution_baseline();
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
                } else if self.providers.active() || !self.provider_starts.is_empty() {
                    Err("A provider is updating. Retry when it finishes.".into())
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
            } else if command.method == "providers.update" {
                self.validate_provider_update(&command.params)
            } else if command.method == "notifications.push.register" {
                database.stage_push_registration(device, &command.params)
            } else if command.method == "notifications.push.unregister" {
                database.stage_push_unregistration(device)
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
            let mut execution_failed = false;
            match result {
                Ok(result) => match self
                    .state
                    .prepare_execution_after_command(&execution_baseline, device)
                {
                    Ok(()) => receipt.result = result,
                    Err(error) => {
                        execution_failed = true;
                        receipt.status = ReceiptStatus::Rejected;
                        receipt.result = json!({
                            "error":format!("Execution fence failed before provider or peer effects: {error}")
                        });
                    }
                },
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
            let captured = capture.finish();
            self.persist(
                Some(&receipt),
                if execution_failed {
                    Vec::new()
                } else {
                    captured
                },
            )?;
            if command.method == "providers.update" && receipt.result["updateAccepted"] == true {
                self.start_provider_update(&receipt.result);
            }
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
        let result = if command.method == "providers.update" {
            self.validate_provider_update(&command.params)
        } else {
            self.state.execute_command(&command)
        };
        match result {
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
        if command.method == "providers.update" && receipt.result["updateAccepted"] == true {
            self.start_provider_update(&receipt.result);
        }
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
        let provider_changed = if self.failure().is_none() {
            match self.poll_provider_updates() {
                Ok(changed) => changed,
                Err(error) => {
                    self.failure = Some(error);
                    false
                }
            }
        } else {
            false
        };
        let update_changed =
            self.updates.poll(&self.state.data_dir) | prepared_changed | provider_changed;
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
            let execution_baseline = self.state.execution_baseline();
            let execution_initiator = self.state.identity.controller_id.clone();
            let changed = crate::model::poll(&mut self.state, 0.0, &mut rune::Tx::default());
            let execution_changed = self.state.refresh_execution(crate::lifecycle::now());
            if let Err(error) = self
                .state
                .prepare_execution_after_command(&execution_baseline, &execution_initiator)
            {
                let error = format!("Execution fence failed before effects: {error}");
                database.fail(error.clone());
                self.state.storage_error = Some(error.clone());
                self.failure = Some(error);
                return false;
            }
            let recovered = self.state.collect_recovered_inputs();
            if recovered || update_changed {
                self.state.persistence_scope = None;
            }
            let changed = changed | recovered | execution_changed;
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
            | self.state.collect_recovered_inputs()
            | self.state.refresh_execution(crate::lifecycle::now())
            | provider_changed;
        if !changed {
            return prepared_changed || before != self.maintenance.status();
        }
        let mut projection = self.state.projection();
        projection["providers"] = self.provider_statuses();
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
        state["providers"] = self.provider_statuses();
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
                    Err(crate::maintenance::PENDING_HARNESS_EFFECTS.into())
                } else if let Some(reason) = self.state.update_restart_blocker() {
                    Err(reason.into())
                } else if let Some(active) = self
                    .database
                    .as_ref()
                    .map(|db| db.fence_dispatch(true))
                    .filter(|active| !active.is_empty())
                {
                    // Do not silently interrupt an already claimed asynchronous effect.
                    Err(format!(
                        "{} Waiting on: {}. Drain cancelled; retry after it settles.",
                        crate::maintenance::ACTIVE_EXTERNAL_EFFECT,
                        active.join(", ")
                    ))
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
            self.state.mark_execution_prompts_dispatched(&effects);
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
            // A scoped batch may be the first commit to see a thread that a
            // tool or harness created inside this poll. The wire baseline only
            // grows thread arrays on a full projection, so promote such a
            // batch to one before stamping or diffing it.
            if index["_partial"] == true {
                let known = self.last_projection["threads"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let known_index = self.last_projection["index"]["threads"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let unknown = ["threads", "threadViews"]
                    .iter()
                    .flat_map(|field| index[*field].as_array().into_iter().flatten())
                    .any(|thread| {
                        let id = &thread["id"];
                        !known.iter().any(|old| old["id"] == *id)
                            || !known_index.iter().any(|old| old["id"] == *id)
                    });
                if unknown {
                    index = self.state.snapshot_scoped(None);
                }
            }
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
                projection["providers"] = self.provider_statuses();
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

    fn execution_preflight(&self, device: &str, command: &Command) -> Result<(), String> {
        let args = command
            .params
            .get("args")
            .filter(|_| command.method.starts_with("task."))
            .unwrap_or(&command.params);
        let thread_id = args["threadId"]
            .as_u64()
            .or_else(|| args["thread_id"].as_u64())
            .and_then(|id| usize::try_from(id).ok());
        if let Some(thread_id) = thread_id
            && let Some(thread) = self
                .state
                .threads
                .iter()
                .find(|thread| thread.id == thread_id)
        {
            let mut replacement = matches!(
                command.method.as_str(),
                "thread.resume" | "thread.retry" | "task.resume"
            ) || matches!(
                command.method.as_str(),
                "thread.send" | "captain.crew.send" | "task.send" | "queue.sendNow"
            ) && !thread.status.busy()
                && !thread.queued();
            if command.method == "task.send" && self.state.is_remote_task_thread(thread_id) {
                // The executor checks the exact lease and generation. Leaving
                // this command available lets it survive an offline target and
                // report the precise refusal when the link returns.
                replacement = false;
            }
            if replacement {
                self.state.require_execution_recovery(thread_id)?;
            }
            if command.method == "request.respond" {
                self.state.require_execution_certain(thread_id)?;
            }
        }

        if !command.method.starts_with("peer.thread.")
            && !command.method.starts_with("peer.execution.")
        {
            return Ok(());
        }
        let database = self
            .database
            .as_ref()
            .ok_or("Peer execution requires a durable server profile.")?;
        let Some(binding) = database.peer_binding(device)? else {
            return Err(
                "This peer credential predates authenticated controller binding. Re-pair with a peer code before assigning executions."
                    .into(),
            );
        };
        if command.method == "peer.thread.launch" {
            let execution = &command.params["execution"];
            if !execution.is_object() {
                return Err(format!(
                    "{} No worker or execution lease was created.",
                    crate::protocol::EXECUTION_LEASES_UPDATE_REQUIRED
                ));
            }
            let origin = &command.params["origin"];
            for field in ["workspaceId", "ownerId", "controllerId"] {
                if execution[field] != binding[field] {
                    return Err(format!(
                        "Peer assignment {field} does not match the authenticated paired controller."
                    ));
                }
            }
            if origin["serverId"] != binding["serverId"]
                || execution["controllerId"] != binding["controllerId"]
            {
                return Err(
                    "Peer assignment controller does not match the authenticated paired identity."
                        .into(),
                );
            }
        } else if matches!(
            command.method.as_str(),
            "peer.thread.send" | "peer.thread.stop" | "peer.execution.fence"
        ) && let Some(execution) = command.params.get("execution")
        {
            if execution["controllerId"] != binding["controllerId"]
                || execution["workspaceId"] != binding["workspaceId"]
                || execution["ownerId"] != binding["ownerId"]
            {
                return Err(
                    "Execution control does not match the authenticated paired controller.".into(),
                );
            }
        }
        if command.method == "peer.execution.fence"
            && command.params["controllerId"] != binding["controllerId"]
        {
            return Err("Execution fence does not match the authenticated controller.".into());
        }
        Ok(())
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
        self.state.mark_execution_restart_uncertain();
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
            self.state.refresh_execution(crate::lifecycle::now());
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
            let nested = update_index_projection(&mut current["index"], value);
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

/// Partial index deltas carry metadata only for the threads changed in this
/// batch. Merge those entries into the full index by id so neither the diff
/// nor the stored baseline shrinks to the scoped subset.
fn update_index_projection(current: &mut Value, delta: &Value) -> Value {
    let mut patches = Vec::new();
    for (key, value) in delta.as_object().unwrap() {
        match key.as_str() {
            "_partial" | "threadViews" | "runtimeInputs" => {}
            "threads" => {
                let threads = current["threads"].as_array_mut().unwrap();
                for thread in value.as_array().unwrap() {
                    let position = threads
                        .iter()
                        .position(|old| old["id"] == thread["id"])
                        .expect("dirty known index thread");
                    let change = crate::protocol::diff(&threads[position], thread);
                    for mut patch in change.as_array().unwrap().clone() {
                        let path = patch["path"].as_array_mut().unwrap();
                        path.insert(0, json!(position.to_string()));
                        path.insert(0, json!("threads"));
                        patches.push(patch);
                    }
                    threads[position] = thread.clone();
                }
            }
            _ => {
                let change = crate::protocol::diff(&current[key], value);
                for mut patch in change.as_array().unwrap().clone() {
                    patch["path"].as_array_mut().unwrap().insert(0, json!(key));
                    patches.push(patch);
                }
                current[key] = value.clone();
            }
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
    #[test]
    fn partial_projection_keeps_the_committed_index_and_wire_baseline_in_sync() {
        let mut runtime = durable();
        for _ in 0..4 {
            let occupied: Vec<usize> = runtime.state.threads.iter().map(|t| t.id).collect();
            runtime
                .command(
                    "maintainer",
                    Command::new("thread.create", json!({"occupiedDrafts":occupied})),
                )
                .unwrap();
        }
        let committed = runtime
            .database
            .as_ref()
            .unwrap()
            .committed_snapshot()
            .unwrap();
        assert_eq!(
            index_ids(&committed["state"]),
            index_ids(&runtime.snapshot()["state"])
        );
        let occupied_after: Vec<usize> = runtime.state.threads.iter().map(|t| t.id).collect();
        runtime
            .command(
                "maintainer",
                Command::new("thread.create", json!({"occupiedDrafts":occupied_after})),
            )
            .unwrap();
        let mut subscriber = runtime
            .database
            .as_ref()
            .unwrap()
            .committed_snapshot()
            .unwrap()["state"]
            .clone();
        assert_eq!(index_ids(&subscriber).len(), 6);
        let changed = runtime.state.threads[2].id;
        runtime.state.threads[2].life.latest_user_message_at = Some(1);
        runtime.state.persistence_scope = Some([changed].into_iter().collect());
        let before = runtime.seq;
        runtime.database.as_ref().unwrap().begin().unwrap();
        runtime.persist(None, vec![]).unwrap();
        assert!(runtime.seq > before);
        let events = runtime
            .database
            .as_ref()
            .unwrap()
            .replay(before, runtime.seq)
            .unwrap()
            .unwrap();
        for event in events {
            crate::protocol::apply(&mut subscriber, event["patch"].clone()).unwrap();
        }
        let committed = runtime
            .database
            .as_ref()
            .unwrap()
            .committed_snapshot()
            .unwrap()["state"]
            .clone();
        assert_eq!(index_ids(&committed), index_ids(&subscriber));
        assert_eq!(
            index_ids(&runtime.snapshot()["state"]),
            index_ids(&subscriber)
        );
        assert_eq!(subscriber["index"], committed["index"]);
        // Workspace patches use the compact wire shape; full history is
        // delivered separately. Compare through the production compactor.
        assert_eq!(
            crate::protocol::compact(&subscriber)["threads"],
            crate::protocol::compact(&committed)["threads"]
        );
        assert_eq!(runtime.snapshot()["state"]["index"], committed["index"]);
    }
    #[test]
    fn a_new_thread_discovered_by_a_scoped_poll_is_announced_with_a_full_projection() {
        let mut runtime = durable();
        for _ in 0..4 {
            let occupied: Vec<usize> = runtime.state.threads.iter().map(|t| t.id).collect();
            runtime
                .command(
                    "maintainer",
                    Command::new("thread.create", json!({"occupiedDrafts":occupied})),
                )
                .unwrap();
        }
        let mut subscriber = runtime
            .database
            .as_ref()
            .unwrap()
            .committed_snapshot()
            .unwrap()["state"]
            .clone();
        let id = runtime.state.next_id;
        runtime.state.next_id += 1;
        runtime.state.threads.push(crate::model::Thread::new(id));
        runtime.state.persistence_scope = Some([id].into_iter().collect());
        let before = runtime.seq;
        runtime.database.as_ref().unwrap().begin().unwrap();
        runtime.persist(None, vec![]).unwrap();
        assert_eq!(runtime.seq, before + 1);
        let events = runtime
            .database
            .as_ref()
            .unwrap()
            .replay(before, runtime.seq)
            .unwrap()
            .unwrap();
        for event in events {
            crate::protocol::apply(&mut subscriber, event["patch"].clone()).unwrap();
        }
        let committed = runtime
            .database
            .as_ref()
            .unwrap()
            .committed_snapshot()
            .unwrap()["state"]
            .clone();
        assert_eq!(index_ids(&committed), index_ids(&subscriber));
        assert!(index_ids(&subscriber).contains(&json!(id)));
        assert_eq!(subscriber["index"], committed["index"]);
        assert_eq!(
            crate::protocol::compact(&subscriber)["threads"],
            crate::protocol::compact(&committed)["threads"]
        );
        assert_eq!(runtime.snapshot()["state"]["index"], committed["index"]);
    }
    fn index_ids(state: &Value) -> Vec<Value> {
        state["index"]["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|thread| thread["id"].clone())
            .collect()
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
    fn crew_answers_commit_before_the_answer_prompt_dispatches() {
        let mut runtime = durable();
        let database = runtime.database.clone().unwrap();
        let channels: std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Receiver<Value>>>> =
            std::sync::Arc::default();
        {
            let state = &mut runtime.state;
            state.captains = crate::captains::Captain::presets();
            state.open_captain(0);
            let hook_channels = channels.clone();
            state.test_connect_pi = Some(Box::new(move |_tid, _path| {
                let (pi, commands, _events) = crate::harness::Session::test_connection();
                hook_channels.lock().unwrap().push(commands);
                Some(pi)
            }));
            let worker = state.next_id;
            state.next_id += 1;
            let mut thread = crate::model::Thread::new(worker);
            thread.crew_captain = Some(0);
            thread.captain = Some(0);
            thread.title = "Crew worker".into();
            thread.run_id = Some("run-q".into());
            thread.model_provider = "cpa".into();
            state.threads.push(thread);
            // The crew root claims a question; the primary home answers it.
            let report_id = state
                .mcp_call(
                    worker,
                    "captain_crew_report",
                    &json!({
                        "requestId":uuid::Uuid::new_v4().to_string(),
                        "outcome":"blocked","summary":"waiting",
                        "question":{"text":"Which branch?","recipient":"captain"}
                    }),
                )
                .unwrap()["reportId"]
                .as_str()
                .unwrap()
                .to_owned();
            let home = state.captain_home(0).unwrap();
            state
                .mcp_call(
                    home,
                    "captain_question_answer",
                    &json!({
                        "reportId":report_id,"expectedRevision":0,
                        "requestId":uuid::Uuid::new_v4().to_string(),"answer":"main"
                    }),
                )
                .unwrap();
        }
        // The batch: scope starts empty so answer delivery must promote a full
        // snapshot before any fake prompt dispatches.
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.persistence_scope = Some(Default::default());
        let changed = crate::model::poll(&mut runtime.state, 0.0, &mut rune::Tx::default());
        let effects = capture.finish();
        assert!(changed);
        assert!(runtime.state.persistence_scope.is_none());
        // Every fake backend (the Captain's home for the report, the worker for
        // the answer) has seen nothing before the commit.
        let mut guard = channels.lock().unwrap();
        let receivers: Vec<_> = guard.drain(..).collect();
        drop(guard);
        assert!(!receivers.is_empty());
        for commands in &receivers {
            assert!(commands.try_recv().is_err());
        }
        // The previous committed snapshot has no answer state.
        let before = database.read("index.json").unwrap();
        assert!(
            before
                .and_then(|index| index["captainQuestionAnswers"].as_array().cloned())
                .is_none_or(|answers| answers.is_empty())
        );
        let answer_id = runtime.state.crew_answers()[0].id.clone();
        runtime.persist(None, effects).unwrap();
        let committed = database.read("index.json").unwrap().unwrap();
        assert_eq!(
            committed["captainQuestionAnswers"][0]["id"],
            json!(answer_id)
        );
        // Only after the commit does the worker's fake backend get the answer.
        let prompt = receivers
            .iter()
            .filter_map(|commands| commands.try_recv().ok())
            .find(|command| {
                serde_json::to_string(command)
                    .unwrap_or_default()
                    .contains(&answer_id)
            })
            .expect("answer prompt after commit");
        // Its real command id acknowledges the delivery; nothing replays.
        let worker_index = runtime
            .state
            .threads
            .iter()
            .position(|t| Some(t.id) == runtime.state.crew_answers().first().map(|a| a.thread_id))
            .unwrap();
        runtime.state.threads[worker_index].apply(json!({
            "type":"response","command":"prompt","id":prompt["id"],"success":true
        }));
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.persistence_scope = Some(Default::default());
        crate::model::poll(&mut runtime.state, 0.0, &mut rune::Tx::default());
        let effects = capture.finish();
        runtime.persist(None, effects).unwrap();
        let committed = database.read("index.json").unwrap().unwrap();
        assert_eq!(
            committed["captainQuestionAnswers"][0]["delivered"],
            json!(true)
        );
        assert!(receivers.iter().all(|commands| {
            commands.try_iter().all(|command| {
                !serde_json::to_string(&command)
                    .unwrap_or_default()
                    .contains(&answer_id)
            })
        }));
    }

    fn runtime_fixture_with_crew() -> (
        RuntimeState,
        crate::database::Database,
        std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Receiver<Value>>>>,
        usize,
    ) {
        let mut runtime = durable();
        let database = runtime.database.clone().unwrap();
        let channels: std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Receiver<Value>>>> =
            std::sync::Arc::default();
        {
            let state = &mut runtime.state;
            state.captains = crate::captains::Captain::presets();
            state.open_captain(0);
            let hook_channels = channels.clone();
            state.test_connect_pi = Some(Box::new(move |_tid, _path| {
                let (pi, commands, _events) = crate::harness::Session::test_connection();
                hook_channels.lock().unwrap().push(commands);
                Some(pi)
            }));
            let worker = state.next_id;
            state.next_id += 1;
            let mut thread = crate::model::Thread::new(worker);
            thread.crew_captain = Some(0);
            thread.captain = Some(0);
            thread.title = "Crew worker".into();
            thread.run_id = Some("run-q".into());
            state.threads.push(thread);
            let home = state.captain_home(0).unwrap();
            let home_index = state.threads.iter().position(|t| t.id == home).unwrap();
            state.threads[home_index].model_provider = "cpa".into();
            (runtime, database, channels, worker)
        }
    }

    #[test]
    fn captain_reset_commits_routine_routing_with_the_home_and_rolls_back_together() {
        let mut runtime = durable();
        let database = runtime.database.clone().unwrap();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.captains = crate::captains::Captain::presets();
        runtime.state.open_captain(0);
        let old = runtime.state.captain_home(0).unwrap();
        let routine = runtime
            .state
            .background
            .routines
            .create(
                0,
                old,
                "Review".into(),
                "Read status".into(),
                crate::routines::Schedule::Every { minutes: 15 },
                crate::routines::MissedPolicy::Skip,
                chrono::Local::now(),
            )
            .unwrap();
        runtime.state.persistence_scope = None;
        runtime.persist(None, capture.finish()).unwrap();
        let receipt = runtime
            .command(
                "maintainer",
                Command::new("captain.reset", json!({"captain":0})),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted");
        let home = runtime.state.captain_home(0).unwrap();
        assert_ne!(home, old);
        assert_eq!(
            database.read("index.json").unwrap().unwrap()["captainHomes"]["0"],
            home
        );
        let ledger = database.read("routines.json").unwrap().unwrap();
        assert_eq!(ledger["routines"][0]["home"], home);
        assert_eq!(ledger["routines"][0]["id"], routine);

        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.persistence_scope = None;
        runtime.state.reset_captain_conversation(0).unwrap();
        assert_ne!(runtime.state.captain_home(0), Some(home));
        database.fail("injected reset persist failure".into());
        assert!(runtime.persist(None, capture.finish()).is_err());
        assert!(
            database.read("index.json").is_err(),
            "failed runtime stays quarantined"
        );
        database.test_connection(|connection| {
            let index = crate::projection_store::read(connection, "index.json")
                .unwrap()
                .unwrap();
            let persisted: String = connection
                .query_row(
                    "SELECT payload FROM documents WHERE name='routines.json'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(index["captainHomes"]["0"], home);
            assert_eq!(
                serde_json::from_str::<Value>(&persisted).unwrap(),
                ledger,
                "a failed reset cannot commit new routine routing without the new home"
            );
        });
    }

    #[test]
    fn captain_archive_restores_exact_markers_and_schedules_without_waking_work() {
        let (mut runtime, database, channels, worker) = runtime_fixture_with_crew();
        let home = runtime.state.captain_home(0).unwrap();
        let home_index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == home)
            .unwrap();
        runtime.state.threads[home_index].session = Some("unloaded-home-history.jsonl".into());
        // A member archived independently must stay archived after group restore.
        let worker_index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == worker)
            .unwrap();
        runtime.state.threads[worker_index].life.archived_at = Some(17);
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        let now = chrono::Local::now();
        let enabled = runtime
            .state
            .background
            .routines
            .create(
                0,
                home,
                "Enabled".into(),
                "Read status".into(),
                crate::routines::Schedule::Every { minutes: 15 },
                crate::routines::MissedPolicy::RunOnce,
                now - chrono::Duration::days(3),
            )
            .unwrap();
        let paused = runtime
            .state
            .background
            .routines
            .create(
                0,
                home,
                "User paused".into(),
                "Read status".into(),
                crate::routines::Schedule::Every { minutes: 15 },
                crate::routines::MissedPolicy::Skip,
                now,
            )
            .unwrap();
        runtime
            .state
            .background
            .routines
            .set_paused(paused, true)
            .unwrap();
        runtime
            .state
            .mcp_call(
                worker,
                "captain_crew_report",
                &json!({
                    "requestId":uuid::Uuid::new_v4(),"outcome":"completed","summary":"Saved result"
                }),
            )
            .unwrap();
        runtime.state.persistence_scope = None;
        runtime.persist(None, capture.finish()).unwrap();
        let preview = runtime
            .state
            .captain_lifecycle_preview(0, "archive")
            .unwrap();
        assert_eq!(preview["blockers"], json!([]));
        let command = Command::new(
            "captain.archive",
            json!({
                "captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"]
            }),
        );
        let receipt = runtime.command("maintainer", command.clone()).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert_eq!(
            runtime.state.captain_phase(0),
            crate::model::CaptainPhase::Archived
        );
        assert!(
            runtime
                .state
                .threads
                .iter()
                .find(|thread| thread.id == home)
                .unwrap()
                .archived()
        );
        let index = database.read("index.json").unwrap().unwrap();
        assert_eq!(
            index["captainLifecycles"]["0"]["pausedRoutines"],
            json!([enabled])
        );
        let ledger = database.read("routines.json").unwrap().unwrap();
        assert!(
            ledger["routines"]
                .as_array()
                .unwrap()
                .iter()
                .all(|routine| routine["paused"] == true)
        );
        assert!(runtime.state.crew_reports()[0].hold_delivery);
        // Same-ID replay returns the committed receipt, not a state conflict.
        assert_eq!(runtime.command("maintainer", command).unwrap(), receipt);
        let preview = runtime
            .state
            .captain_lifecycle_preview(0, "unarchive")
            .unwrap();
        let receipt = runtime.command("maintainer", Command::new("captain.unarchive", json!({
            "captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"]
        }))).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert!(
            !runtime
                .state
                .threads
                .iter()
                .find(|thread| thread.id == home)
                .unwrap()
                .archived()
        );
        assert_eq!(
            runtime.state.threads[worker_index].life.archived_at,
            Some(17)
        );
        let routines = runtime.state.background.routines.entries();
        let enabled = routines
            .iter()
            .find(|routine| routine.id == enabled)
            .unwrap();
        assert!(!enabled.paused);
        assert!(
            enabled.next_at > now.timestamp(),
            "missed run_once slots do not replay"
        );
        assert!(
            routines
                .iter()
                .find(|routine| routine.id == paused)
                .unwrap()
                .paused
        );
        assert!(runtime.state.crew_reports()[0].hold_delivery);
        assert!(!runtime.state.crew_reports()[0].delivered);
        runtime.tick();
        assert!(
            channels.lock().unwrap().is_empty(),
            "restore must not start a provider"
        );
        assert!(runtime.state.threads.iter().all(|thread| !thread.queued()));
    }

    #[test]
    fn failed_captain_archive_commits_neither_markers_nor_routine_pauses() {
        let (mut runtime, database, _, _) = runtime_fixture_with_crew();
        let home = runtime.state.captain_home(0).unwrap();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime
            .state
            .background
            .routines
            .create(
                0,
                home,
                "Review".into(),
                "Read status".into(),
                crate::routines::Schedule::Every { minutes: 15 },
                crate::routines::MissedPolicy::Skip,
                chrono::Local::now(),
            )
            .unwrap();
        runtime.state.persistence_scope = None;
        runtime.persist(None, capture.finish()).unwrap();
        let before_index = database.read("index.json").unwrap().unwrap();
        let before_ledger = database.read("routines.json").unwrap().unwrap();
        let preview = runtime
            .state
            .captain_lifecycle_preview(0, "archive")
            .unwrap();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime
            .state
            .captain_archive_command(
                0,
                "archive",
                preview["captainVersion"].as_str().unwrap(),
                &serde_json::from_value::<Vec<usize>>(preview["crewSeen"].clone()).unwrap(),
            )
            .unwrap();
        database.fail("injected archive persistence failure".into());
        assert!(runtime.persist(None, capture.finish()).is_err());
        assert!(
            database.read("index.json").is_err(),
            "a failed runtime stays quarantined"
        );
        database.test_connection(|connection| {
            assert_eq!(
                crate::projection_store::read(connection, "index.json")
                    .unwrap()
                    .unwrap(),
                before_index
            );
            let ledger: String = connection
                .query_row(
                    "SELECT payload FROM documents WHERE name='routines.json'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&ledger).unwrap(),
                before_ledger
            );
        });
    }

    #[test]
    fn captain_archive_reopens_with_holds_and_idle_sessions_without_fabricated_reports() {
        let (mut runtime, database, channels, worker) = runtime_fixture_with_crew();
        let directory = runtime.state.data_dir.clone();
        let home = runtime.state.captain_home(0).unwrap();
        let worker_index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == worker)
            .unwrap();
        let (session, commands, _events) = crate::harness::Session::test_connection();
        runtime.state.threads[worker_index].pi = Some(session);
        runtime.state.threads[worker_index].arm_crew_report();
        let preview = runtime
            .state
            .captain_lifecycle_preview(0, "archive")
            .unwrap();
        let command = Command::new(
            "captain.archive",
            json!({
                "captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"]
            }),
        );
        let receipt = runtime.command("maintainer", command.clone()).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert!(
            runtime.state.crew_reports().is_empty(),
            "disconnect must not report cancellation"
        );
        assert!(
            commands
                .try_iter()
                .all(|command| command["type"] != "prompt")
        );
        assert!(channels.lock().unwrap().is_empty());
        drop(runtime);
        drop(database);
        let mut reopened = RuntimeState::open(directory, std::sync::Arc::new(|| {})).unwrap();
        assert_eq!(
            reopened.state.captain_phase(0),
            crate::model::CaptainPhase::Archived
        );
        assert!(
            reopened
                .state
                .threads
                .iter()
                .find(|thread| thread.id == home)
                .unwrap()
                .archived()
        );
        assert!(
            reopened
                .state
                .threads
                .iter()
                .all(|thread| thread.pi.is_none())
        );
        assert!(reopened.state.crew_reports().is_empty());
        assert_eq!(reopened.command("maintainer", command).unwrap(), receipt);
    }

    #[test]
    fn captain_roles_commit_preserved_placement_and_restore_retained_private_records() {
        let mut runtime = durable();
        let database = runtime.database.clone().unwrap();
        let directory = runtime.state.data_dir.clone();
        let home = runtime.state.current().id;
        let cwd = runtime
            .state
            .thread_directory(runtime.state.active)
            .unwrap();
        runtime.state.current_mut().harness = "pi".into();
        runtime.state.current_mut().session = Some("relative-session.jsonl".into());
        runtime.state.current_mut().native.access = crate::model::AccessMode::Full;
        let profile = crate::captains::Captain::presets().remove(0);
        let preview = runtime.state.captain_promote_preview(home, None).unwrap();
        let promote = Command::new(
            "captain.promote",
            json!({
                "threadId":home,"record":profile.to_value(),"expectedVersion":preview["threadVersion"]
            }),
        );
        let receipt = runtime.command("maintainer", promote.clone()).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let captain = receipt["result"]["captain"].as_u64().unwrap() as usize;
        assert_eq!(runtime.command("maintainer", promote).unwrap(), receipt);
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "memory.put",
                    json!({
                        "captain":captain,"text":"Retain this private memory","essential":true
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let receipt = runtime.command("maintainer", Command::new("captain.document.write", json!({
            "captain":captain,"name":"NOTES.md","text":"Retain this plan","expectedRevision":0
        }))).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        let routine = runtime
            .state
            .background
            .routines
            .create(
                captain,
                home,
                "Review".into(),
                "Read status".into(),
                crate::routines::Schedule::Every { minutes: 15 },
                crate::routines::MissedPolicy::RunOnce,
                chrono::Local::now(),
            )
            .unwrap();
        runtime.state.persistence_scope = None;
        runtime.persist(None, capture.finish()).unwrap();
        let memories = runtime.state.memories(captain);
        let notes = runtime
            .state
            .captain_document_value(captain, "NOTES.md")
            .unwrap();
        let preview = runtime
            .state
            .captain_lifecycle_preview(captain, "demote")
            .unwrap();
        let demote = Command::new(
            "captain.demote",
            json!({
                "captain":captain,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"]
            }),
        );
        let receipt = runtime.command("maintainer", demote.clone()).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let index = database.read("index.json").unwrap().unwrap();
        assert_eq!(
            index["captainLifecycles"][captain.to_string()]["phase"],
            "retired"
        );
        assert_eq!(runtime.state.current().id, home);
        assert!(!runtime.state.current().captain_home);
        assert_eq!(
            runtime
                .state
                .thread_directory(runtime.state.active)
                .unwrap(),
            cwd
        );
        assert_eq!(
            runtime.state.current().native.access,
            crate::model::AccessMode::Full
        );
        assert!(
            runtime
                .state
                .background
                .routines
                .entries()
                .iter()
                .find(|entry| entry.id == routine)
                .unwrap()
                .paused
        );
        assert_eq!(runtime.command("maintainer", demote).unwrap(), receipt);
        drop(runtime);
        drop(database);
        let mut runtime = RuntimeState::open(directory, std::sync::Arc::new(|| {})).unwrap();
        assert_eq!(
            runtime.state.captain_phase(captain),
            crate::model::CaptainPhase::Retired
        );
        assert_eq!(
            serde_json::to_value(runtime.state.memories(captain)).unwrap(),
            serde_json::to_value(memories).unwrap()
        );
        assert_eq!(
            runtime
                .state
                .captain_document_value(captain, "NOTES.md")
                .unwrap(),
            notes
        );
        let preview = runtime
            .state
            .captain_promote_preview(home, Some(captain))
            .unwrap();
        let reconnects = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = reconnects.clone();
        runtime.state.test_connect_pi = Some(Box::new(move |_, _| {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (session, _, _) = crate::harness::Session::test_connection();
            Some(session)
        }));
        let receipt = runtime.command("maintainer", Command::new("captain.promote", json!({
            "threadId":home,"restoreCaptain":captain,"record":profile.to_value(),"expectedVersion":preview["threadVersion"]
        }))).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert_eq!(receipt["result"]["captain"], captain);
        assert_eq!(runtime.state.captains.len(), 1);
        assert_eq!(
            runtime
                .state
                .thread_directory(runtime.state.active)
                .unwrap(),
            cwd
        );
        assert_eq!(
            runtime.state.current().native.access,
            crate::model::AccessMode::Full
        );
        assert_eq!(
            runtime.state.current().session.as_deref(),
            cwd.join("relative-session.jsonl").to_str()
        );
        assert!(runtime.state.current().pi.is_none());
        assert!(
            runtime
                .state
                .background
                .routines
                .entries()
                .iter()
                .find(|entry| entry.id == routine)
                .unwrap()
                .paused,
            "restoring a profile never resumes its routines"
        );
        runtime.tick();
        assert_eq!(
            reconnects.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "role restoration must not auto-connect an unloaded saved session"
        );
    }

    #[test]
    fn failed_captain_demotion_cannot_commit_a_retired_profile_without_pausing_its_routines() {
        let mut runtime = durable();
        let database = runtime.database.clone().unwrap();
        let home = runtime.state.current().id;
        let profile = crate::captains::Captain::presets().remove(0);
        let preview = runtime.state.captain_promote_preview(home, None).unwrap();
        let receipt = runtime.command("maintainer", Command::new("captain.promote", json!({
            "threadId":home,"record":profile.to_value(),"expectedVersion":preview["threadVersion"]
        }))).unwrap();
        let captain = receipt["result"]["captain"].as_u64().unwrap() as usize;
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime
            .state
            .background
            .routines
            .create(
                captain,
                home,
                "Review".into(),
                "Read status".into(),
                crate::routines::Schedule::Every { minutes: 15 },
                crate::routines::MissedPolicy::Skip,
                chrono::Local::now(),
            )
            .unwrap();
        runtime.state.persistence_scope = None;
        runtime.persist(None, capture.finish()).unwrap();
        let before_index = database.read("index.json").unwrap().unwrap();
        let before_ledger = database.read("routines.json").unwrap().unwrap();
        let preview = runtime
            .state
            .captain_lifecycle_preview(captain, "demote")
            .unwrap();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime
            .state
            .captain_demote(captain, preview["captainVersion"].as_str().unwrap(), &[])
            .unwrap();
        database.fail("injected role persistence failure".into());
        assert!(runtime.persist(None, capture.finish()).is_err());
        assert!(database.read("index.json").is_err());
        database.test_connection(|connection| {
            assert_eq!(
                crate::projection_store::read(connection, "index.json")
                    .unwrap()
                    .unwrap(),
                before_index
            );
            let ledger: String = connection
                .query_row(
                    "SELECT payload FROM documents WHERE name='routines.json'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&ledger).unwrap(),
                before_ledger
            );
        });
    }

    #[test]
    fn mcp_replies_dispatch_only_after_the_commit() {
        let (mut runtime, database, channels, worker) = runtime_fixture_with_crew();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.persistence_scope = Some(Default::default());
        runtime.state.mcp_request(crate::mcp::Request {
            thread: worker,
            tool: "captain_crew_report".into(),
            args: json!({
                "requestId":uuid::Uuid::new_v4().to_string(),
                "outcome":"blocked","summary":"waiting",
                "question":{"text":"Which branch?","recipient":"captain"}
            }),
            reply: reply_tx,
        });
        // The saved result is not visible before the batch commits.
        assert!(reply_rx.try_recv().is_err());
        let changed = crate::model::poll(&mut runtime.state, 0.0, &mut rune::Tx::default());
        let effects = capture.finish();
        assert!(changed);
        // The reply is captured with only its marker, never the tool payload.
        let reply_effect = effects
            .iter()
            .find(|effect| effect.payload["type"] == "mcp.reply")
            .expect("captured mcp reply");
        assert_eq!(reply_effect.payload, json!({"type":"mcp.reply"}));
        assert!(reply_rx.try_recv().is_err());
        // The previous committed snapshot has no report.
        let before = database.read("index.json").unwrap();
        assert!(
            before
                .and_then(|index| index["captainReports"].as_array().cloned())
                .is_none_or(|reports| reports.is_empty())
        );
        runtime.persist(None, effects).unwrap();
        // The committed snapshot carries the report before the reply lands.
        let committed = database.read("index.json").unwrap().unwrap();
        let committed_id = committed["captainReports"][0]["id"].as_str().unwrap();
        let reply = reply_rx.try_recv().expect("reply after commit").unwrap();
        assert_eq!(reply["saved"], json!(true));
        assert_eq!(reply["reportId"], json!(committed_id));
        // The Captain wake prompt dispatches in the same drain.
        let mut guard = channels.lock().unwrap();
        let prompt = guard
            .iter_mut()
            .filter_map(|commands| commands.try_recv().ok())
            .find(|command| {
                serde_json::to_string(command)
                    .unwrap_or_default()
                    .contains(committed_id)
            });
        assert!(prompt.is_some());
    }

    #[test]
    fn a_failed_persist_never_dispatches_the_saved_reply() {
        let (mut runtime, database, _channels, worker) = runtime_fixture_with_crew();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.persistence_scope = Some(Default::default());
        runtime.state.mcp_request(crate::mcp::Request {
            thread: worker,
            tool: "captain_crew_report".into(),
            args: json!({
                "requestId":uuid::Uuid::new_v4().to_string(),
                "outcome":"progress","summary":"half"
            }),
            reply: reply_tx,
        });
        crate::model::poll(&mut runtime.state, 0.0, &mut rune::Tx::default());
        let effects = capture.finish();
        database.fail("injected storage failure".into());
        assert!(runtime.persist(None, effects).is_err());
        // No saved:true ever reached the caller.
        match reply_rx.try_recv() {
            Err(_) => {}
            Ok(reply) => assert!(
                reply.is_err() || reply.unwrap()["saved"] != json!(true),
                "no saved:true may reach the caller after a failed persist"
            ),
        }
    }

    #[test]
    fn uncertain_execution_is_refused_before_retry_or_resume_can_mutate_state() {
        let mut runtime = durable();
        let thread = runtime.state.current().id;
        let run = uuid::Uuid::new_v4().to_string();
        runtime.state.threads[runtime.state.active].run_id = Some(run.clone());
        let (lease, _) = runtime
            .state
            .execution
            .prepare(crate::execution::Prepare {
                scope: crate::execution::Scope {
                    workspace_id: runtime.state.identity.workspace_id.clone(),
                    owner_id: runtime.state.identity.owner_id.clone(),
                    run_as_owner_id: runtime.state.identity.owner_id.clone(),
                    controller_id: runtime.state.identity.controller_id.clone(),
                    initiating_device_id: "44444444-4444-4444-8444-444444444444".into(),
                    executor_id: runtime.state.identity.executor_id.clone(),
                },
                logical_captain_uid: None,
                logical_thread_uid: runtime
                    .state
                    .snapshot_identity()
                    .thread_uid(thread)
                    .unwrap()
                    .into(),
                executor_thread_alias: Some(thread as u64),
                executor_thread_uid: None,
                run_id: run.clone(),
                attempt_id: None,
                delivery_id: None,
                request_key: None,
                now: 1,
                queues_behind_current: false,
            })
            .unwrap();
        runtime
            .state
            .execution
            .transition(
                &lease.lease_id,
                crate::execution::Phase::Dispatched,
                2,
                "sent",
            )
            .unwrap();
        runtime
            .state
            .execution
            .uncertain(&lease.lease_id, 3, "executor disconnected")
            .unwrap();
        let before = runtime.seq;
        let error = runtime
            .command(
                "maintainer",
                Command::new("thread.retry", json!({"threadId":thread})),
            )
            .unwrap_err();
        assert!(error.contains("verified fence"), "{error}");
        assert_eq!(runtime.seq, before);
        assert_eq!(
            runtime.state.current().run_id.as_deref(),
            Some(run.as_str())
        );
        assert_eq!(runtime.state.execution.leases.len(), 1);
        runtime
            .database
            .clone()
            .unwrap()
            .fail("test complete".into());
    }

    #[test]
    fn execution_commit_failure_dispatches_zero_provider_effects() {
        let (mut runtime, database, _channels, worker) = runtime_fixture_with_crew();
        let index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == worker)
            .unwrap();
        let (pi, commands, _events) = crate::harness::Session::test_connection();
        runtime.state.threads[index].pi = Some(pi);
        runtime.state.threads[index].model_provider = "cpa".into();
        runtime.state.threads[index].model_id = "fixture".into();
        runtime.state.threads[index].status = crate::model::Status::Running;
        let run = runtime.state.threads[index].run_id.clone().unwrap();
        let (lease, _) = runtime
            .state
            .execution
            .prepare(crate::execution::Prepare {
                scope: crate::execution::Scope {
                    workspace_id: runtime.state.identity.workspace_id.clone(),
                    owner_id: runtime.state.identity.owner_id.clone(),
                    run_as_owner_id: runtime.state.identity.owner_id.clone(),
                    controller_id: runtime.state.identity.controller_id.clone(),
                    initiating_device_id: "44444444-4444-4444-8444-444444444444".into(),
                    executor_id: runtime.state.identity.executor_id.clone(),
                },
                logical_captain_uid: None,
                logical_thread_uid: runtime
                    .state
                    .snapshot_identity()
                    .thread_uid(worker)
                    .unwrap()
                    .into(),
                executor_thread_alias: Some(worker as u64),
                executor_thread_uid: None,
                run_id: run,
                attempt_id: None,
                delivery_id: None,
                request_key: None,
                now: 1,
                queues_behind_current: false,
            })
            .unwrap();
        runtime.state.threads[index].execution = Some(lease.binding());
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        assert!(
            runtime.state.threads[index]
                .command("prompt", json!({"message":"must never reach the provider"}),)
        );
        let effects = capture.finish();
        assert!(!effects.is_empty());
        database.fail("injected execution commit failure".into());
        assert!(runtime.persist(None, effects).is_err());
        assert!(
            commands.try_recv().is_err(),
            "a prompt effect must wait for the execution lease and receipt commit"
        );
        database.test_connection(|connection| {
            let index = crate::projection_store::read(connection, "index.json")
                .unwrap()
                .unwrap();
            assert!(
                index["execution"]["leases"]
                    .as_array()
                    .is_none_or(Vec::is_empty)
            );
        });
    }

    #[test]
    fn a_dropped_reply_receiver_does_not_block_later_dispatch() {
        let (mut runtime, database, channels, worker) = runtime_fixture_with_crew();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.persistence_scope = Some(Default::default());
        runtime.state.mcp_request(crate::mcp::Request {
            thread: worker,
            tool: "captain_crew_report".into(),
            args: json!({
                "requestId":uuid::Uuid::new_v4().to_string(),
                "outcome":"blocked","summary":"waiting",
                "question":{"text":"Which branch?","recipient":"captain"}
            }),
            reply: reply_tx,
        });
        crate::model::poll(&mut runtime.state, 0.0, &mut rune::Tx::default());
        let effects = capture.finish();
        // The MCP caller disconnects before the commit completes.
        drop(reply_rx);
        runtime.persist(None, effects).unwrap();
        let committed = database.read("index.json").unwrap().unwrap();
        let report_id = committed["captainReports"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(runtime.failure.is_none());
        // The harness prompt behind the reply still dispatches.
        let mut guard = channels.lock().unwrap();
        assert!(guard.iter_mut().any(|commands| {
            commands.try_iter().any(|command| {
                serde_json::to_string(&command)
                    .unwrap_or_default()
                    .contains(&report_id)
            })
        }));
    }

    #[test]
    fn report_policy_commits_before_report_capture_and_failed_change_leaves_no_saved_policy() {
        use crate::model::ReportPolicy;
        let (mut runtime, database, channels, worker) = runtime_fixture_with_crew();
        let command = Command::new(
            "captain.reportPolicy",
            json!({"captain":0,"policy":"ignore","expectedPolicy":"react"}),
        );
        let receipt = runtime.command("maintainer", command.clone()).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert_eq!(runtime.state.captain_report_policy(0), ReportPolicy::Ignore);
        assert_eq!(
            database.read("index.json").unwrap().unwrap()["captainReportPolicies"]["0"],
            "ignore"
        );
        assert_eq!(runtime.command("maintainer", command).unwrap(), receipt);
        let before_connections = channels.lock().unwrap().len();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.persistence_scope = Some(Default::default());
        let index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == worker)
            .unwrap();
        runtime.state.threads[index].apply(json!({"type":"agent_start"}));
        runtime.state.threads[index].apply(json!({
            "type":"message_end","message":{"role":"assistant","content":"A recorded claim"}
        }));
        runtime.state.threads[index].apply(json!({"type":"agent_settled","aborted":false}));
        crate::model::poll(&mut runtime.state, 0.0, &mut rune::Tx::default());
        runtime.persist(None, capture.finish()).unwrap();
        let saved = database.read("index.json").unwrap().unwrap();
        assert_eq!(saved["captainReports"][0]["policy"], "ignore");
        assert_eq!(saved["captainReports"][0]["delivered"], false);
        assert_eq!(
            channels.lock().unwrap().len(),
            before_connections,
            "ignored report cannot connect a provider"
        );
        let report_id = saved["captainReports"][0]["id"].as_str().unwrap();
        assert!(
            channels
                .lock()
                .unwrap()
                .iter_mut()
                .flat_map(|commands| commands.try_iter())
                .all(|command| !command.to_string().contains(report_id))
        );
        let before = saved;
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime
            .state
            .set_captain_report_policy(0, ReportPolicy::Context, ReportPolicy::Ignore)
            .unwrap();
        database.fail("injected policy failure".into());
        assert!(runtime.persist(None, capture.finish()).is_err());
        database.test_connection(|connection| {
            assert_eq!(
                crate::projection_store::read(connection, "index.json")
                    .unwrap()
                    .unwrap(),
                before
            );
        });
    }

    #[test]
    fn crew_reports_commit_before_the_wake_prompt_dispatches() {
        let mut runtime = durable();
        let database = runtime.database.clone().unwrap();
        let channels: std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Receiver<Value>>>> =
            std::sync::Arc::default();
        {
            let state = &mut runtime.state;
            state.captains = crate::captains::Captain::presets();
            state.open_captain(0);
            let hook_channels = channels.clone();
            state.test_connect_pi = Some(Box::new(move |_tid, _path| {
                let (pi, commands, _events) = crate::harness::Session::test_connection();
                hook_channels.lock().unwrap().push(commands);
                Some(pi)
            }));
            let worker = state.next_id;
            state.next_id += 1;
            let mut thread = crate::model::Thread::new(worker);
            thread.crew_captain = Some(0);
            thread.captain = Some(0);
            thread.title = "Crew worker".into();
            state.threads.push(thread);
            let home = state.captain_home(0).unwrap();
            let home_index = state.threads.iter().position(|t| t.id == home).unwrap();
            state.threads[home_index].model_provider = "cpa".into();
            let index = state.threads.iter().position(|t| t.id == worker).unwrap();
            // A real provider turn: start, reply, settle.
            state.threads[index].apply(json!({"type":"agent_start"}));
            state.threads[index].apply(json!({
                "type":"message_end","message":{"role":"assistant","content":"Turn reply"}}));
            state.threads[index].apply(json!({"type":"agent_settled","aborted":false}));
        }
        let report_id = runtime
            .state
            .threads
            .iter()
            .find(|t| t.crew_captain == Some(0))
            .unwrap()
            .id;
        // The batch: scope starts empty so any change must promote a full snapshot.
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.persistence_scope = Some(Default::default());
        let changed = crate::model::poll(&mut runtime.state, 0.0, &mut rune::Tx::default());
        let effects = capture.finish();
        assert!(changed);
        assert!(runtime.state.persistence_scope.is_none());
        // Nothing reached the fake backend yet: the prompt waits for the commit.
        let mut guard = channels.lock().unwrap();
        let commands = guard.remove(0);
        drop(guard);
        assert!(commands.try_recv().is_err());
        // The previous committed snapshot has no report state.
        let before = database.read("index.json").unwrap();
        assert!(
            before
                .and_then(|index| index["captainReports"].as_array().cloned())
                .is_none_or(|reports| reports.is_empty())
        );
        let report_id = runtime
            .state
            .crew_reports()
            .iter()
            .find(|report| report.thread_id == report_id)
            .map(|report| report.id.clone())
            .expect("captured crew report");
        runtime.persist(None, effects).unwrap();
        let committed = database.read("index.json").unwrap().unwrap();
        assert_eq!(committed["captainReports"][0]["id"], json!(report_id));
        // Only after the commit does the fake backend see the prompt.
        let prompt = commands.try_recv().expect("wake prompt after commit");
        assert!(serde_json::to_string(&prompt).unwrap().contains(&report_id));
        // Its real command id acknowledges the delivery; no duplicate prompt follows.
        let home = runtime.state.captain_home(0).unwrap();
        let home_index = runtime
            .state
            .threads
            .iter()
            .position(|t| t.id == home)
            .unwrap();
        runtime.state.threads[home_index].apply(json!({
            "type":"response","command":"prompt","id":prompt["id"],"success":true
        }));
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.persistence_scope = Some(Default::default());
        assert!(crate::model::poll(
            &mut runtime.state,
            0.0,
            &mut rune::Tx::default()
        ));
        let effects = capture.finish();
        runtime.persist(None, effects).unwrap();
        let committed = database.read("index.json").unwrap().unwrap();
        assert_eq!(committed["captainReports"][0]["delivered"], json!(true));
        assert!(commands.try_iter().all(|command| {
            !serde_json::to_string(&command)
                .unwrap()
                .contains(&report_id)
        }));
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
    fn startup_keeps_a_newer_active_attempt_while_closing_the_old_audit_row() {
        let directory = crate::store::directory()
            .unwrap()
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let db = crate::database::Database::open(&directory).unwrap();
        db.issue_device("interrupted-device", "not-a-real-token")
            .unwrap();
        let old_id = uuid::Uuid::new_v4().to_string();
        let connection = rusqlite::Connection::open(directory.join("state.sqlite")).unwrap();
        connection.execute("INSERT INTO update_audit(update_id,device_id,requested_at,accepted,tag,outcome) VALUES(?1,'interrupted-device',1,1,'v99.0.0',?2)",
            rusqlite::params![old_id,json!({"updateAccepted":true}).to_string()]).unwrap();
        drop(connection);
        drop(db);
        let new_id = uuid::Uuid::new_v4().to_string();
        let updates_dir = directory.join("updates");
        std::fs::create_dir_all(&updates_dir).unwrap();
        crate::store::write_atomic(
            &updates_dir.join("request.json"),
            json!({"updateId":new_id,"tag":"v0.12.19","startedAt":crate::lifecycle::now()})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        let guard = crate::install::InstallLock::acquire(&updates_dir).unwrap();
        let mut runtime =
            RuntimeState::open(directory.clone(), std::sync::Arc::new(|| {})).unwrap();
        assert_eq!(runtime.updates.phase, "restarting");
        assert_eq!(runtime.updates.id.as_deref(), Some(new_id.as_str()));
        assert_eq!(runtime.updates.tag, "v0.12.19");
        assert!(
            runtime
                .database
                .as_ref()
                .unwrap()
                .unfinished_update()
                .unwrap()
                .is_none(),
            "the old audit row was terminalized"
        );
        drop(guard);
        crate::fleet_update::save_outcome(
            &directory,
            &crate::fleet_update::Outcome {
                update_id: new_id.clone(),
                tag: "v0.12.19".into(),
                success: true,
                reason: None,
            },
        )
        .unwrap();
        runtime.updates.poll(&directory);
        assert_eq!(runtime.updates.phase, "idle");
        assert_eq!(runtime.updates.id.as_deref(), Some(new_id.as_str()));
        assert!(runtime.updates.outcome.as_ref().unwrap().success);
        runtime.shutdown().unwrap();
        drop(runtime);
        std::fs::remove_dir_all(&directory).unwrap();
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
        assert!(database.claim_dispatch("test effect").unwrap().is_none());
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        crate::outbox::send(&sender, json!({"type":"prompt", "id":"paused"})).unwrap();
        rt.persist(None, capture.finish()).unwrap();
        assert!(receiver.try_recv().is_err());
        // Metadata discovery can publish during the drain. It must not dispatch
        // the committed prompt or change the maintenance fence.
        let before = rt.maintenance.status()["phase"].clone();
        rt.tick();
        assert_eq!(rt.maintenance.status()["phase"], before);
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
        let dispatch = database.claim_dispatch("test effect").unwrap().unwrap();
        let other = database.claim_dispatch("test effect").unwrap().unwrap();
        drop(other);
        let error = rt
            .command(
                "maintainer",
                Command::new("server.prepareRestart", json!({})),
            )
            .unwrap_err();
        assert!(error.contains("Drain cancelled"));
        assert!(error.contains("Waiting on: test effect."));
        assert!(!rt.maintenance.fenced());
        drop(dispatch);
        rt.command(
            "maintainer",
            Command::new("server.prepareRestart", json!({})),
        )
        .unwrap();
        assert!(database.claim_dispatch("test effect").unwrap().is_none());
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
        assert!(database.claim_dispatch("test effect").unwrap().is_none());
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
    fn prepare_restart_allows_idle_configuration_but_refuses_active_work() {
        let mut rt = runtime();
        for kind in ["active", "approval", "queue"] {
            {
                let t = rt.state.current_mut();
                t.status = crate::model::Status::Ready;
                t.configuring = false;
                t.request = None;
                t.native.projected_queued = false;
                match kind {
                    "active" => t.status = crate::model::Status::Running,
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
        let t = rt.state.current_mut();
        t.native.projected_queued = false;
        t.configuring = true;
        rt.command(
            "maintainer",
            Command::new("server.prepareRestart", json!({})),
        )
        .unwrap();
        assert!(rt.maintenance.fenced());
        assert!(!rt.state.runtime_stop_requested);
    }
}
