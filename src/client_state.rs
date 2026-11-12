//! State facade and plain projections. Execution still uses the original State/Thread methods.
use super::*;
use crate::protocol::{Command, Receipt};

#[derive(serde::Serialize, serde::Deserialize)]
struct MessageView {
    role: String,
    text: String,
    images: Vec<Attachment>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ThreadView {
    id: usize,
    #[serde(default)]
    generation: String,
    status: Status,
    messages: Vec<MessageView>,
    tools: Vec<Tool>,
    thoughts: Vec<Thought>,
    work: Vec<AssistantWork>,
    harness_state: Value,
    model: String,
    model_provider: String,
    model_id: String,
    thinking: String,
    models: Vec<AgentModel>,
    thinking_levels: Vec<String>,
    supports_images: Option<bool>,
    context_window: Option<u64>,
    context_usage: crate::context::ContextUsage,
    context_phase: crate::context::ContextPhase,
    configuring: bool,
    #[serde(default)]
    initializing: bool,
    phase: String,
    error: Option<String>,
    diagnostics: Vec<String>,
    request: Option<Value>,
    questionnaire: Value,
    elapsed: Option<u64>,
    queue: Vec<(usize, String, Vec<Attachment>)>,
    steering: Vec<String>,
    follow_ups: Vec<String>,
    commands: Vec<Value>,
    compactions: Vec<Value>,
    stats: Value,
    fork_points: Vec<Value>,
    operating: bool,
    queued: bool,
    #[serde(default)]
    harness_choice: bool,
    #[serde(default)]
    commands_loaded: bool,
    #[serde(default)]
    commands_error: Option<String>,
}

impl ThreadView {
    fn capture(thread: &Thread) -> Self {
        // JSON roundtrips these bounded, plain records, never process or presentation objects.
        Self {
            id: thread.id,
            generation: thread.request_generation.clone(),
            status: thread.status,
            messages: thread
                .messages
                .iter()
                .map(|m| MessageView {
                    role: m.role.into(),
                    text: m.text.clone(),
                    images: m.images.clone(),
                })
                .collect(),
            tools: thread
                .tools
                .iter()
                .map(|tool| {
                    let mut tool = tool.clone();
                    if !tool.done {
                        tool.duration_ms = tool
                            .started
                            .map(|at| at.elapsed().as_secs().saturating_mul(1000))
                            .or(tool.duration_ms);
                    }
                    tool
                })
                .collect(),
            thoughts: thread
                .thoughts
                .iter()
                .map(|thought| {
                    let mut thought = thought.clone();
                    if !thought.done {
                        thought.duration_ms = thought
                            .started
                            .map(|at| at.elapsed().as_secs().saturating_mul(1000))
                            .or(thought.duration_ms);
                    }
                    thought
                })
                .collect(),
            work: thread.work.clone(),
            harness_state: thread.harness_state.clone(),
            model: thread.model.clone(),
            model_provider: thread.model_provider.clone(),
            model_id: thread.model_id.clone(),
            thinking: thread.thinking.clone(),
            models: thread.models.clone(),
            thinking_levels: thread.thinking_levels.clone(),
            supports_images: thread.supports_images,
            context_window: thread.context_window,
            context_usage: thread.context.usage.clone(),
            context_phase: thread.context.phase,
            configuring: thread.configuring,
            initializing: thread.initializing,
            phase: thread.phase.clone(),
            error: thread.error.clone(),
            diagnostics: thread.diagnostics.clone(),
            request: thread.request.clone().map(|mut request| {
                request["runtimeGeneration"] = json!(thread.request_generation);
                request
            }),
            questionnaire: thread.projected_form(),
            elapsed: thread
                .started
                .map(|at| at.elapsed().as_secs())
                .or(thread.elapsed),
            queue: thread
                .native
                .queue
                .iter()
                .map(|q| (q.id, q.prompt.text.clone(), q.prompt.images.to_vec()))
                .collect(),
            steering: thread.native.steering.clone(),
            follow_ups: thread.native.follow_ups.clone(),
            commands: thread.native.commands.iter().map(|c| json!(c)).collect(),
            compactions: thread.native.compactions.clone(),
            stats: thread.native.stats.clone(),
            fork_points: thread.native.fork_points.clone(),
            operating: thread.native.operating(),
            queued: thread.queued(),
            harness_choice: thread.can_choose_harness(),
            commands_loaded: thread.native.commands_loaded,
            commands_error: thread.native.commands_error.clone(),
        }
    }

    fn apply(self, thread: &mut Thread) -> Result<(), String> {
        if thread.remote.is_some() {
            thread.request_generation = self.generation;
        }
        thread.status = self.status;
        thread.messages = self
            .messages
            .into_iter()
            .map(|message| {
                Ok(Message {
                    role: match message.role.as_str() {
                        "You" => "You",
                        "Pi" => "Pi",
                        "Shina" => "Shina",
                        _ => return Err("Invalid message role.".into()),
                    },
                    text: message.text,
                    images: message.images,
                })
            })
            .collect::<Result<_, String>>()?;
        let mut tools = self.tools;
        for tool in &mut tools {
            if !tool.done {
                tool.started = tool
                    .duration_ms
                    .and_then(|ms| Instant::now().checked_sub(Duration::from_millis(ms)));
            }
            if let Some(old) = thread.tools.iter().find(|old| old.id == tool.id) {
                tool.expanded = old.expanded;
                tool.group_expanded = old.group_expanded;
            }
        }
        thread.tools = tools;
        let mut thoughts = self.thoughts;
        for thought in &mut thoughts {
            if !thought.done {
                thought.started = thought
                    .duration_ms
                    .and_then(|ms| Instant::now().checked_sub(Duration::from_millis(ms)));
            }
            thought.expanded = thread
                .thoughts
                .iter()
                .find(|old| old.block == thought.block && old.anchor == thought.anchor)
                .is_some_and(|old| old.expanded);
        }
        thread.thoughts = thoughts;
        let mut work = self.work;
        for item in &mut work {
            item.expanded = thread
                .work
                .iter()
                .find(|old| old.start == item.start)
                .is_none_or(|old| old.expanded);
        }
        thread.work = work;
        thread.harness_state = self.harness_state;
        thread.model = self.model;
        thread.model_provider = self.model_provider;
        thread.model_id = self.model_id;
        thread.thinking = self.thinking;
        thread.models = self.models;
        thread.thinking_levels = self.thinking_levels;
        thread.supports_images = self.supports_images;
        thread.context_window = self.context_window;
        thread.context.usage = self.context_usage;
        thread.context.phase = self.context_phase;
        thread.configuring = self.configuring;
        if thread.remote.is_some() {
            thread.initializing = self.initializing;
        }
        thread.phase = self.phase;
        thread.error = self.error;
        thread.diagnostics = self.diagnostics;
        let changed_request = thread
            .request
            .as_ref()
            .map(|r| (&r["id"], &r["runtimeGeneration"]))
            != self
                .request
                .as_ref()
                .map(|r| (&r["id"], &r["runtimeGeneration"]));
        thread.request = self.request;
        if changed_request {
            thread.reply.replace_text(
                thread
                    .request
                    .as_ref()
                    .and_then(|request| request["prefill"].as_str())
                    .unwrap_or(""),
            );
            thread.prepare_remote_questionnaire();
            thread.apply_projected_form(self.questionnaire)?;
        }
        thread.elapsed = self.elapsed;
        thread.started = self
            .elapsed
            .filter(|_| thread.status.busy())
            .and_then(|secs| Instant::now().checked_sub(Duration::from_secs(secs)));
        thread.native.queue = self
            .queue
            .into_iter()
            .map(|(id, text, images)| pi_session::QueuedMessage {
                id,
                prompt: Prompt::new(text, images),
            })
            .collect();
        thread.native.steering = self.steering;
        thread.native.follow_ups = self.follow_ups;
        thread.native.commands = self
            .commands
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        thread.native.compactions = self.compactions;
        thread.native.stats = self.stats;
        thread.native.fork_points = self.fork_points;
        thread.native.projected_operating = self.operating;
        thread.native.projected_queued = self.queued;
        thread.projected_harness_choice = self.harness_choice;
        thread.native.commands_loaded = self.commands_loaded;
        thread.native.commands_error = self.commands_error;
        Ok(())
    }
}

impl State {
    pub fn validate_import(
        directory: &std::path::Path,
        documents: &std::collections::BTreeMap<String, Value>,
    ) -> Result<(), String> {
        let mut state = Self::headless(directory.to_path_buf());
        if let Some(index) = documents.get("index.json") {
            if index["version"] != 3 && index["version"] != 4 {
                return Err("Unsupported legacy index version.".into());
            }
            state.restore_global(index)?;
            if let Some(views) = index.get("threadViews") {
                state.restore_persisted_history(views)?;
            }
            if let Some(inputs) = index.get("runtimeInputs") {
                for (id, inputs) in inputs.as_object().ok_or("Invalid runtime inputs.")? {
                    let thread = state
                        .threads
                        .iter_mut()
                        .find(|thread| thread.id.to_string() == *id)
                        .ok_or("Runtime input references an unknown thread.")?;
                    thread.restore_pending_inputs(inputs)?;
                }
            }
        }
        if let Some(ui) = documents.get("ui-state.json") {
            state.restore_ui(ui)?;
        }
        if let Some(inputs) = documents.get("server-inputs.json") {
            if inputs["version"] != 1 {
                return Err("Unsupported server input version.".into());
            }
            if let Some(background) = inputs.get("background") {
                let _: BackgroundSettings = serde_json::from_value(background.clone())
                    .map_err(|e| format!("Invalid server background settings: {e}"))?;
            }
            if let Some(pending) = inputs.get("pendingMessages") {
                for (id, inputs) in pending.as_object().ok_or("Invalid pending messages.")? {
                    let thread = state
                        .threads
                        .iter_mut()
                        .find(|thread| thread.id.to_string() == *id)
                        .ok_or("Pending input references an unknown thread.")?;
                    thread.restore_pending_inputs(inputs)?;
                }
            }
        }
        if let Some(pending) = documents
            .get("ui-state.json")
            .and_then(|ui| ui.get("pendingMessages"))
        {
            for (id, inputs) in pending.as_object().ok_or("Invalid pending messages.")? {
                let thread = state
                    .threads
                    .iter_mut()
                    .find(|thread| thread.id.to_string() == *id)
                    .ok_or("Pending input references an unknown thread.")?;
                thread.restore_pending_inputs(inputs)?;
            }
        }
        state.load_memory(false)?;
        let routines = crate::routines::Routines::load(directory.join("routines.json"));
        if let Some(error) = &routines.error {
            return Err(error.clone());
        }
        for routine in routines.entries() {
            if !state.threads.iter().any(|thread| {
                thread.id == routine.home
                    && thread.captain == Some(routine.captain)
                    && thread.captain_home
                    && !thread.delegated
            }) {
                return Err("Routine references an unknown captain home.".into());
            }
        }
        if let Some(value) = documents.get("honcho-outbox.json") {
            honcho::validate_import(value)?;
        }
        if state
            .threads
            .iter()
            .any(|thread| thread.attachment_error.is_some())
        {
            return Err("Import contains missing or invalid input images. Legacy files were left unchanged.".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn write_realistic_legacy_fixture(directory: &std::path::Path) {
        let mut state = Self::headless(directory.to_path_buf());
        state.load_global_for_test();
        state.captains = Captain::presets();
        let workspace = directory.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let project = state.add_project(workspace).unwrap().unwrap();
        state.captain_project = Some(Some(project));
        state.open_captain(0);
        let home = state.current().id;
        let mut child = Thread::new(state.next_id);
        state.next_id += 1;
        child.title = "Inspect a realistic fixture".into();
        child.project = Some(project);
        child.parent = Some(home);
        child.delegated = true;
        child.captain = Some(0);
        let child_id = child.id;
        state.threads.push(child);
        state.demo_task(
            child_id,
            TaskState::Completed,
            Some(1000),
            "Fixture result",
            "",
        );
        let mut archived = Thread::new(state.next_id);
        state.next_id += 1;
        archived.title = "Archived history".into();
        archived.project = Some(project);
        archived.life.archived_at = Some(lifecycle::now());
        archived.messages.push(Message {
            role: "Pi",
            text: "Retained archived reply".into(),
            images: vec![],
        });
        state.threads.push(archived);
        state
            .add_memory(0, "The fixture uses a temporary workspace.".into(), true)
            .unwrap();
        state.background.routines =
            crate::routines::Routines::load(directory.join("routines.json"));
        state
            .background
            .routines
            .create(
                0,
                home,
                "Check fixtures".into(),
                "Inspect status".into(),
                crate::routines::Schedule::Every { minutes: 15 },
                crate::routines::MissedPolicy::Skip,
                chrono::Local::now(),
            )
            .unwrap();
        state
            .current_mut()
            .draft
            .replace_text("Unsent local draft, never server-owned.");
        state
            .current_mut()
            .native
            .queue
            .push_back(pi_session::QueuedMessage {
                id: 1,
                prompt: Prompt::new("Previously queued input".into(), vec![]),
            });
        state.checkpoint_and_flush().unwrap();
    }

    pub fn headless(directory: PathBuf) -> Self {
        let mut state = Self::construct(PathBuf::new(), false);
        state.data_dir = directory;
        state.headless = true;
        state.persist_transcripts = true;
        state
    }

    pub fn setup_runtime(&mut self, wake: Waker) -> Result<(), String> {
        self.wake = wake.clone();
        let saved = crate::store::load(&self.data_dir.join("index.json"))?;
        let views: Option<Vec<ThreadView>> = saved
            .as_ref()
            .and_then(|index| index.get("threadViews"))
            .filter(|views| !views.is_null())
            .map(|views| {
                serde_json::from_value(views.clone())
                    .map_err(|e| format!("Invalid saved server transcript: {e}"))
            })
            .transpose()?;
        self.load_global(wake);
        if let Some(error) = &self.storage_error {
            return Err(error.clone());
        }
        // Restore the server-owned transcript, especially ACP implementations with no history.
        if let Some(views) = views {
            self.install_thread_views(views)?;
        }
        self.setup_headless_background();
        if let Some(error) = &self.background.routines.error {
            return Err(error.clone());
        }
        self.runtime_enabled = true;
        self.start_task_recovery();
        self.refresh_catalog();
        self.checkpoint_and_flush()
    }

    pub(super) fn restore_persisted_history(&mut self, value: &Value) -> Result<(), String> {
        let views = serde_json::from_value(value.clone())
            .map_err(|e| format!("Invalid saved server transcript: {e}"))?;
        self.install_thread_views(views)
    }

    fn install_thread_views(&mut self, views: Vec<ThreadView>) -> Result<(), String> {
        for view in views {
            if let Some(thread) = self.threads.iter_mut().find(|t| t.id == view.id) {
                view.apply(thread)?;
                thread.server_owned = true;
                thread.history_loaded = true;
                thread.native.projected_operating = false;
                thread.native.projected_queued = false;
                // Lost process queues are recovered against history, never dispatched here.
                thread.native.queue.clear();
                thread.native.steering.clear();
                thread.native.follow_ups.clear();
                thread.native.questionnaire = None;
                if thread.status.busy() || thread.configuring || thread.request.is_some() {
                    thread.status = Status::Failed;
                    thread.configuring = false;
                    thread.request = None;
                    thread.started = None;
                    thread.phase.clear();
                    thread.error = Some("Shina restarted during this run. Check partial work before retrying. No prompt was replayed.".into());
                }
            }
        }
        Ok(())
    }

    pub fn stop_runtime(&mut self) {
        for thread in &mut self.threads {
            thread.disconnect();
            thread.recover_native_inputs();
            if thread.has_draft_content() {
                self.recovered_inputs.insert(
                    thread.id,
                    json!({"text":thread.draft.text,"attachments":thread.draft_images}),
                );
                thread.draft.replace_text("");
                thread.draft_images.clear();
            }
        }
    }
    pub fn begin_runtime_shutdown(&mut self) {
        self.runtime_enabled = false;
        self.projects.clone_job.take();
        for thread in &mut self.threads {
            if thread.status.busy()
                || thread.configuring
                || thread.queued()
                || thread.request.is_some()
            {
                thread.stop();
            }
        }
    }
    pub fn drain_runtime_shutdown(&mut self) -> bool {
        for thread in &mut self.threads {
            for _ in 0..256 {
                let Some(event) = thread.pi.as_ref().and_then(|pi| pi.events.try_recv().ok())
                else {
                    break;
                };
                match event {
                    Event::Record(record) => thread.apply(record),
                    Event::Error(error) => thread.fail_session(error),
                    Event::Diagnostic(message) => thread.diagnostic(&message),
                    Event::Exited => thread.disconnect(),
                }
            }
        }
        self.collect_recovered_inputs();
        self.threads
            .iter()
            .any(|thread| thread.pi.is_some() && (thread.status.busy() || thread.configuring))
    }
    pub fn server_epoch(&self) -> &str {
        &self.server_epoch
    }
    pub fn collect_recovered_inputs(&mut self) -> bool {
        let mut changed = false;
        for thread in &mut self.threads {
            if !thread.has_draft_content() {
                continue;
            }
            if let Some(previous) = self.recovered_inputs.get(&thread.id) {
                let newer = Prompt::new(thread.draft.text.clone(), thread.draft_images.clone());
                thread
                    .draft
                    .replace_text(previous["text"].as_str().unwrap_or(""));
                thread.draft_images =
                    serde_json::from_value(previous["attachments"].clone()).unwrap_or_default();
                thread.restore_prompt(&newer);
            }
            self.recovered_inputs.insert(thread.id,json!({"text":thread.draft.text,"attachments":thread.draft_images,"commandId":thread.recovery_command}));
            thread.draft.replace_text("");
            thread.draft_images.clear();
            changed = true;
        }
        changed
    }
    pub fn server_sequence(&self) -> u64 {
        self.command_receipts
            .iter()
            .map(|r| r.seq)
            .max()
            .unwrap_or(0)
            .max(self.server_seq)
    }
    pub fn advance_server_sequence(&mut self, seq: u64) {
        self.server_seq = self.server_seq.max(seq);
    }
    pub fn command_receipt(&self, device: &str, id: uuid::Uuid) -> Option<&Receipt> {
        self.command_receipts
            .iter()
            .find(|receipt| receipt.device_id == device && receipt.command_id == id)
    }
    pub fn lookup_command(&self, device: &str, params: &Value) -> Result<Value, String> {
        let id = serde_json::from_value(params["commandId"].clone())
            .map_err(|_| "Invalid commandId.")?;
        self.command_receipt(device, id)
            .map(|r| json!(r))
            .ok_or("No receipt for this device and commandId.".into())
    }
    pub fn record_command(&mut self, receipt: Receipt, intent: Option<Command>) {
        self.advance_server_sequence(receipt.seq);
        let key = format!("{}/{}", receipt.device_id, receipt.command_id);
        if let Some(intent) = intent {
            self.command_intents.insert(key, intent);
        } else {
            self.command_intents.remove(&key);
        }
        if let Some(current) = self
            .command_receipts
            .iter_mut()
            .find(|r| r.device_id == receipt.device_id && r.command_id == receipt.command_id)
        {
            *current = receipt;
        } else {
            self.command_receipts.push(receipt);
        }
    }

    pub(super) fn thread_views(&self) -> Value {
        json!(
            self.threads
                .iter()
                .map(ThreadView::capture)
                .collect::<Vec<_>>()
        )
    }
    pub fn projection(&self) -> Value {
        let mut index = self.snapshot();
        for key in [
            "commandReceipts",
            "commandIntents",
            "threadViews",
            "serverSeq",
            "runtimeInputs",
        ] {
            index.as_object_mut().expect("index").remove(key);
        }
        json!({
            "index":index,"threads":self.thread_views(),"recoveryNotice":self.recovery_notice,
            "directories":self.threads.iter().enumerate().filter_map(|(index,thread)| self.thread_directory(index).ok().map(|path| (thread.id.to_string(),json!(path)))).collect::<serde_json::Map<_,_>>(),
            "memories":(0..self.captains.len()).map(|id| (id.to_string(), json!(self.memories(id)))).collect::<serde_json::Map<_,_>>(),
            "profile":self.user_profile(),
            "routines":(0..self.captains.len()).map(|id| (id.to_string(), json!(self.routines(id)))).collect::<serde_json::Map<_,_>>(),
            "background":self.background_status(),"undo":self.undo,
            "memoryBackend":self.memory_backend_status(),
            "clone":{"running":self.projects.cloning(),"progress":self.projects.clone_progress,"error":self.projects.clone_error},
        })
    }

    pub(super) fn setup_client(&mut self, wake: Waker) {
        // Concurrent desktop launches get separate draft stores, never competing index writers.
        let mut slot = None;
        for index in 0..32 {
            let directory = self.data_dir.join("clients").join(index.to_string());
            if let Ok(owner) = crate::server_lock::Owner::acquire(&directory) {
                self.owner = Some(owner);
                slot = Some(directory.join("ui-state.json"));
                break;
            }
        }
        let Some(path) = slot else {
            self.storage_error = Some("All local client draft slots are in use.".into());
            return;
        };
        self.client
            .as_mut()
            .expect("client")
            .set_device_directory(path.parent().expect("client directory").to_path_buf());
        self.ui_store = Some(crate::store::Store::new(path.clone(), wake.clone()));
        // Apply after the first projection, when server thread IDs are available.
        self.client_views["localUi"] = match crate::store::load(&path) {
            Ok(Some(ui)) => ui,
            Ok(None) => crate::store::load(&self.data_dir.join("ui-state.json"))
                .ok()
                .flatten()
                .unwrap_or(Value::Null),
            Err(error) => {
                self.storage_error = Some(error);
                self.ui_store = None;
                return;
            }
        };
        let ui = &self.client_views["localUi"];
        self.machines.ui = ui
            .get("machineUi")
            .cloned()
            .unwrap_or_else(|| json!({"local":ui}));
        self.setup_client_background();
        self.updater.start(true, Instant::now(), wake.clone());
        self.client.as_mut().expect("client").start(wake);
    }

    pub(super) fn checkpoint_client(&mut self) {
        let mut ui = self.ui_snapshot();
        ui["clientDrafts"] = json!(
            self.threads
                .iter()
                .filter(|t| t.client_draft)
                .map(|t| json!({
                    "id":t.id,"project":t.project,"harness":t.harness,"model":t.selected_model,
                    "thinking":t.selected_thinking,"access":t.native.access.id()
                }))
                .collect::<Vec<_>>()
        );
        ui["pendingMessages"] = json!({});
        ui["active"] = json!(self.current().id);
        ui["tabs"] = json!(self.tabs);
        ui["dark"] = json!(self.dark);
        ui["zoom"] = json!(self.zoom);
        self.machines.ui[&self.machines.selected] = ui.clone();
        // The top-level legacy format remains the local server's namespace.
        let mut saved = self.machines.ui["local"].clone();
        if saved.is_null() {
            saved = ui.clone();
        }
        // Device preferences must survive a restart even when the last selected
        // thread belonged to a remote server. Drafts and IDs stay server-scoped.
        for field in [
            "dark",
            "zoom",
            "sidebar",
            "sidebarWidth",
            "shelves",
            "background",
            "scope",
            "scopeAdditional",
            "scopeServer",
            "scopeRemote",
        ] {
            saved[field] = ui[field].clone();
        }
        saved["machineUi"] = self.machines.ui.clone();
        if let Some(store) = &mut self.ui_store {
            store.save(saved);
        }
    }

    pub(super) fn poll_client(&mut self, tx: &mut Tx) -> bool {
        let changed = self.poll_client_background(tx)
            | self.updater.poll(Instant::now(), self.wake.clone())
            | self.refresh_machine_notice();
        if self.background.quitting {
            return true;
        }
        let projections = self
            .client
            .as_ref()
            .map(|client| client.snapshots())
            .unwrap_or_default();
        let mut changed = changed || !projections.is_empty();
        for (server, projection) in projections {
            self.machines
                .views
                .insert(server.clone(), projection.clone());
            if server == self.machines.selected
                && let Err(error) = self.apply_projection(projection)
            {
                self.storage_error = Some(error);
            }
        }
        if let Some(client) = &self.client {
            if self.machines.selected == "local" {
                client.show_history(self.current().id);
            } else if let Some(client) = client.linked.get(&self.machines.selected) {
                client.show_history(self.current().id);
            }
        }
        let mut events = self
            .machines
            .receipts
            .remove(&self.machines.selected)
            .unwrap_or_default()
            .into_iter();
        loop {
            let event = events
                .next()
                .map(|event| (self.machines.selected.clone(), event))
                .or_else(|| {
                    self.client
                        .as_ref()
                        .and_then(|client| client.machine_event())
                });
            let Some((server, event)) = event else {
                break;
            };
            changed = true;
            if let crate::client::Event::Receipt(command, value) = &event
                && command.method == "thread.create"
                && command.params["clientDraft"].is_u64()
            {
                self.complete_client_creation(&server, command, value);
                continue;
            }
            if server != self.machines.selected
                && matches!(event, crate::client::Event::Receipt(..))
            {
                if let crate::client::Event::Receipt(command, value) = &event
                    && let Some(thread) =
                        self.machines.threads.get_mut(&server).and_then(|threads| {
                            threads.iter_mut().find(|thread| {
                                thread
                                    .remote_send
                                    .as_ref()
                                    .is_some_and(|(id, _, _)| *id == command.command_id)
                            })
                        })
                    && thread
                        .remote_send
                        .as_ref()
                        .is_some_and(|(id, _, _)| *id == command.command_id)
                    && let Some((_, edit, attachments)) = thread.remote_send.take()
                    && value["result"]["status"] == "accepted"
                    && thread.draft == edit
                    && thread.draft_images == attachments
                {
                    thread.draft.replace_text("");
                    thread.draft_images.clear();
                    self.machines.ui[&server]["drafts"][thread.id.to_string()] = json!("");
                }
                self.machines
                    .receipts
                    .entry(server)
                    .or_default()
                    .push(event);
                continue;
            }
            if server != "local" && matches!(event, crate::client::Event::Maintenance(..)) {
                // A restarting machine is not ready, so the unreachable notice covers it.
                self.refresh_machine_notice();
                continue;
            }
            match event {
                crate::client::Event::Maintenance(method, mut params) => {
                    if method == "server.draining" {
                        params["pid"] = json!(std::process::id());
                        let saved = self.checkpoint_and_flush();
                        let (method, params) = match saved {
                            Ok(()) => ("server.clientReady", params),
                            Err(error) => {
                                self.restart_notice = Some(format!(
                                    "Update cancelled because drafts could not save: {error}"
                                ));
                                ("server.cancelRestart", params)
                            }
                        };
                        if let Some(client) = &self.client {
                            let _ = client.commands.send(method, params);
                        }
                    } else if method == "server.restarting" {
                        self.background.quitting = true;
                        tx.quit();
                    } else if method == "server.stopping" {
                        self.restart_notice = Some("The local server stopped. Start it with shina service start or shina serve.".into());
                    } else {
                        self.restart_notice =
                            Some("Update drain cancelled. Normal operation resumed.".into());
                    }
                }
                crate::client::Event::Error(error) => {
                    if server == "local" {
                        self.storage_error = Some(error);
                    } else {
                        self.machines.notice =
                            Some(format!("{}: {error}", self.machine_label(&server)));
                    }
                }
                crate::client::Event::Receipt(command, value) => {
                    if let Some(thread) = self.threads.iter_mut().find(|thread| {
                        thread
                            .remote_send
                            .as_ref()
                            .is_some_and(|(id, _, _)| *id == command.command_id)
                    }) && let Some((id, edit, attachments)) = thread.remote_send.take()
                    {
                        if id == command.command_id {
                            if value["result"]["status"] == "accepted"
                                && thread.draft == edit
                                && thread.draft_images == attachments
                            {
                                thread.draft.replace_text("");
                                thread.draft_images.clear();
                            }
                        } else {
                            thread.remote_send = Some((id, edit, attachments));
                        }
                    }
                    if command.method.starts_with("server.") {
                        if let Some(error) = value["error"]["message"].as_str() {
                            self.restart_notice = Some(error.into());
                        }
                    } else if value["result"]["status"] == "accepted" {
                        if matches!(
                            command.method.as_str(),
                            "thread.create"
                                | "captain.open"
                                | "captain.put"
                                | "captain.reset"
                                | "thread.fork"
                        ) && let Some(id) = value["result"]["result"]["threadId"].as_u64()
                        {
                            self.client_select = Some(id as usize);
                        }
                        if command.method == "project.put"
                            && let Some(id) = value["result"]["result"]["projectId"].as_u64()
                        {
                            self.machines.project_selection.insert(
                                server.clone(),
                                (
                                    id as usize,
                                    command.params["selectFor"]
                                        .as_str()
                                        .unwrap_or("project")
                                        .into(),
                                    command.params["captain"].as_u64().map(|id| id as usize),
                                ),
                            );
                        }
                        if matches!(command.method.as_str(), "thread.stop" | "thread.rewind")
                            && let Some(id) = command.params["threadId"].as_u64()
                        {
                            self.client_recover.insert(id as usize, command.command_id);
                        }
                        if matches!(command.method.as_str(), "queue.edit" | "input.claim")
                            && let Some(thread) = self
                                .threads
                                .iter_mut()
                                .find(|t| json!(t.id) == command.params["threadId"])
                        {
                            let images = serde_json::from_value(
                                value["result"]["result"]["attachments"].clone(),
                            )
                            .unwrap_or_default();
                            thread.restore_prompt(&Prompt::new(
                                value["result"]["result"]["text"]
                                    .as_str()
                                    .unwrap_or("")
                                    .into(),
                                images,
                            ));
                            thread.native.follow_up = true;
                        }
                    } else {
                        if let Some(id) = command.params["clientDraft"].as_u64()
                            && let Some(draft) = self
                                .threads
                                .iter_mut()
                                .find(|thread| thread.id == id as usize && thread.client_draft)
                        {
                            draft.client_creating = None;
                        }
                        self.storage_error = Some(value["error"]["message"].as_str().or(value["result"]["result"]["error"].as_str()).unwrap_or("Command outcome is uncertain. Inspect the receipt before retrying.").into());
                    }
                }
            }
        }
        if let Some(result) = self
            .machines
            .pending
            .as_ref()
            .and_then(|pending| pending.try_recv().ok())
        {
            self.machines.pending = None;
            changed = true;
            match result {
                Ok(crate::peer::MachineResult::Linked(link)) => {
                    if self
                        .client
                        .as_ref()
                        .is_some_and(|client| client.linked.contains_key(&link.server_id))
                    {
                        let threads = if self.machines.selected == link.server_id {
                            Some(&mut self.threads)
                        } else {
                            self.machines.threads.get_mut(&link.server_id)
                        };
                        if let Some(threads) = threads {
                            for thread in threads {
                                if thread.remote_send.take().is_some() {
                                    thread.error=Some("This device was paired again. Review the previous send before retrying. Your draft was kept.".into());
                                }
                            }
                        }
                    }
                    if let Some(client) = &mut self.client {
                        client.add_link(link, self.wake.clone());
                    }
                    self.machines.code.replace_text("");
                    self.machines.notice =
                        Some("Machine linked. Its workspace will appear when connected.".into());
                }
                Ok(crate::peer::MachineResult::Paired(value)) => {
                    self.machines.notice = Some(format!(
                        "Pairing code: {}. Expires in five minutes and works once.",
                        value["code"].as_str().unwrap_or("")
                    ))
                }
                Ok(crate::peer::MachineResult::Unlinked(id)) => {
                    if self.machines.selected == id {
                        self.select_machine("local");
                    }
                    if let Some(client) = &mut self.client {
                        client.linked.remove(&id);
                    }
                    self.machines.views.remove(&id);
                    self.machines.threads.remove(&id);
                    self.machines.notice = Some(
                        "The server acknowledged revocation. Local drafts were retained.".into(),
                    );
                }
                Ok(crate::peer::MachineResult::Devices(value)) => self.machines.devices = value,
                Err(error) => self.machines.notice = Some(error),
            }
        }
        self.finish_client_promotions();
        self.finish_project_selection();
        if let Some(id) = self.client_select
            && let Some(index) = self.threads.iter().position(|t| t.id == id)
        {
            self.client_select = None;
            self.select_thread(index);
        }
        for (id, command_id) in self.client_recover.clone() {
            if self
                .recovered_inputs
                .get(&id)
                .is_some_and(|input| input["commandId"] == json!(command_id))
                && self
                    .remote(
                        "input.claim",
                        json!({"threadId":id,"expectedCommandId":command_id}),
                    )
                    .is_some_and(|result| result.is_ok())
            {
                self.client_recover.remove(&id);
            }
        }
        for thread in &mut self.threads {
            changed |= thread.native_tick(Instant::now());
            if let Some(deadline) = thread.native_deadline() {
                tx.wake_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
        let at = lifecycle::now();
        for thread in &self.threads {
            if let Some(wake) = thread.life.snoozed_until {
                changed |= wake > self.last_lifecycle_clock && wake <= at;
                if wake > at {
                    tx.wake_after(Duration::from_millis(wake - at));
                }
            }
        }
        self.last_lifecycle_clock = at;
        if at.saturating_sub(self.last_sweep) >= 60_000 {
            self.last_sweep = at;
            changed = true;
        }
        tx.wake_after(Duration::from_millis(
            60_000 - at.saturating_sub(self.last_sweep).min(60_000),
        ));
        if let Some(notice) = &self.undo {
            if notice.expires <= at {
                self.undo = None;
                changed = true;
            } else {
                tx.wake_after(Duration::from_millis(notice.expires - at));
            }
        }
        if let Some(wait) = crate::ui::tick_portraits(self, Instant::now()) {
            tx.wake_after(wait);
        }
        if changed {
            self.checkpoint_client();
        }
        if self.restart_for_update {
            self.restart_for_update = false;
            if let Err(error) = self.prepared_exit(true, None) {
                self.restart_notice = Some(error);
            }
        }
        if let Some(wait) = self.updater.remaining(Instant::now()) {
            tx.wake_after(wait);
        }
        changed
    }

    fn apply_projection(&mut self, projection: Value) -> Result<(), String> {
        let active = self.current().id;
        let tabs = self.tabs.clone();
        let first = self.client_views["index"].is_null();
        let dark = if first {
            self.client_views["localUi"]["dark"]
                .as_bool()
                .unwrap_or(projection["index"]["dark"] == true)
        } else {
            self.dark
        };
        let zoom = if first {
            self.client_views["localUi"]["zoom"]
                .as_f64()
                .or_else(|| projection["index"]["zoom"].as_f64())
                .map(|zoom| snap_zoom(zoom as f32))
                .unwrap_or(self.zoom)
        } else {
            self.zoom
        };
        let scope = self.project_scope;
        let additional = self.scope_additional.clone();
        let mut old = std::mem::take(&mut self.threads);
        // restore_global validates record references; retain only its server-owned records.
        self.restore_global(&projection["index"])?;
        let metadata = std::mem::take(&mut self.threads);
        for mut thread in metadata {
            if let Some(index) = old.iter().position(|old| old.id == thread.id) {
                let mut local = old.swap_remove(index);
                local.title = thread.title;
                local.renamed = thread.renamed;
                local.life = thread.life;
                local.session = thread.session;
                local.harness = thread.harness;
                local.harness_context_sent = thread.harness_context_sent;
                local.project = thread.project;
                local.machine = thread.machine;
                local.updated = thread.updated;
                local.captain = thread.captain;
                local.captain_home = thread.captain_home;
                local.delegated = thread.delegated;
                local.parent = thread.parent;
                local.native.access = thread.native.access;
                local.selected_model = thread.selected_model;
                local.selected_thinking = thread.selected_thinking;
                thread = local;
            }
            thread.remote = self
                .client
                .as_ref()
                .and_then(|client| client.commands_for(&self.machines.selected))
                .cloned();
            self.threads.push(thread);
        }
        // Device drafts are not server records. Projections cannot overwrite or delete them.
        self.threads
            .extend(old.into_iter().filter(|thread| thread.client_draft));
        if let Some(drafts) = self.client_views["localUi"]["clientDrafts"]
            .as_array()
            .cloned()
        {
            for draft in drafts {
                let Some(id) = draft["id"]
                    .as_u64()
                    .map(|id| id as usize)
                    .filter(|id| *id > usize::MAX / 2)
                else {
                    continue;
                };
                if self.threads.iter().any(|thread| thread.id == id) {
                    continue;
                }
                let mut thread = Thread::new(id);
                thread.client_draft = true;
                thread.project = draft["project"].as_u64().map(|id| id as usize);
                thread.harness = draft["harness"].as_str().unwrap_or("pi").into();
                thread.selected_model =
                    serde_json::from_value(draft["model"].clone()).unwrap_or_default();
                thread.selected_thinking = draft["thinking"].as_str().map(str::to_owned);
                thread.native.access = draft["access"]
                    .as_str()
                    .and_then(AccessMode::parse)
                    .unwrap_or_default();
                self.threads.push(thread);
            }
        }
        let views: Vec<ThreadView> =
            serde_json::from_value(projection["threads"].clone()).map_err(|e| e.to_string())?;
        for view in views {
            if let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == view.id) {
                view.apply(thread)?;
            }
        }
        self.active = self
            .threads
            .iter()
            .position(|thread| thread.id == active)
            .unwrap_or(0);
        self.tabs = tabs
            .into_iter()
            .filter(|id| self.threads.iter().any(|t| t.id == *id))
            .collect();
        if !self.tabs.contains(&self.current().id) {
            self.tabs.push(self.current().id);
        }
        self.dark = dark;
        self.zoom = zoom;
        self.project_scope = scope;
        self.scope_additional = additional;
        self.recovery_notice = projection["recoveryNotice"].as_str().map(str::to_owned);
        if !self.undo.as_ref().is_some_and(|notice| {
            notice.action == Action::Discard && notice.expires > lifecycle::now()
        }) {
            self.undo =
                serde_json::from_value(projection["undo"].clone()).map_err(|e| e.to_string())?;
        }
        self.projects.remote_cloning = projection["clone"]["running"] == true;
        self.projects.clone_progress = projection["clone"]["progress"]
            .as_str()
            .unwrap_or("")
            .into();
        self.projects.clone_error = projection["clone"]["error"].as_str().map(str::to_owned);
        let local = self
            .client_views
            .get_mut("localUi")
            .map(Value::take)
            .unwrap_or(Value::Null);
        self.client_views = projection;
        if !local.is_null() {
            self.restore_ui(&local)?;
            if let Some(tabs) = local["tabs"].as_array() {
                self.tabs = tabs
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|id| id as usize)
                    .filter(|id| self.threads.iter().any(|t| t.id == *id))
                    .collect();
            }
            if let Some(index) = self
                .threads
                .iter()
                .position(|thread| json!(thread.id) == local["active"])
            {
                self.active = index;
            }
        }
        if !self.tabs.contains(&self.current().id) {
            self.tabs.push(self.current().id);
        }
        self.client_recover
            .retain(|id, _| self.threads.iter().any(|thread| thread.id == *id));
        self.capture_opened_draft();
        self.storage_error = None;
        if let Some(path) = self.launch_project.take()
            && !self.demo
        {
            self.forward("project.put", json!({"path":path}));
        }
        Ok(())
    }

    pub(super) fn remote(&self, method: &str, params: Value) -> Option<Result<uuid::Uuid, String>> {
        self.client.as_ref().map(|client| {
            client
                .commands_for(if method.starts_with("server.") {
                    "local"
                } else {
                    &self.machines.selected
                })
                .ok_or_else(|| "This machine is not linked.".to_string())?
                .send(method, params)
        })
    }
    pub fn machine_label(&self, server: &str) -> String {
        if server == "local" {
            "Local".into()
        } else {
            self.client
                .as_ref()
                .and_then(|client| client.linked.get(server))
                .and_then(|client| client.remote.as_ref())
                .map(|link| link.host.clone())
                .unwrap_or_else(|| server.into())
        }
    }
    /// Whether this desktop's connection to a machine's Shina server is ready. Demo data has no
    /// server, so it never shows a dropped connection.
    pub fn machine_connected(&self, server: &str) -> bool {
        if self.demo {
            return true;
        }
        self.client
            .as_ref()
            .and_then(|client| client.commands_for(server))
            .is_some_and(|commands| commands.connected())
    }
    /// The names of the linked machines whose connection is not ready.
    pub fn unreachable_machines(&self) -> Vec<String> {
        let Some(client) = &self.client else {
            return Vec::new();
        };
        client
            .linked
            .keys()
            .filter(|server| !self.machine_connected(server))
            .map(|server| self.machine_label(server))
            .collect()
    }
    /// The open thread's machine, by name, while this desktop cannot reach it.
    pub fn unreachable_selected(&self) -> Option<String> {
        (self.client.is_some()
            && (!self.machine_connected(&self.machines.selected)
                || !self.machines.views.contains_key(&self.machines.selected)))
        .then(|| self.machine_label(&self.machines.selected))
    }
    /// Raises one notice naming every linked machine that is down ("Studio and Laptop are
    /// unreachable. Reconnecting.") and clears it when all of them are ready again. It never
    /// replaces or clears another notice. Returns whether the notice changed.
    pub(super) fn refresh_machine_notice(&mut self) -> bool {
        let down = self.unreachable_machines();
        let ours =
            self.machines.notice.is_some() && self.machines.notice == self.machines.auto_notice;
        if down.is_empty() {
            if ours {
                self.machines.notice = None;
                self.machines.auto_notice = None;
                return true;
            }
            self.machines.auto_notice = None;
            return false;
        }
        if self.machines.notice.is_some() && !ours {
            return false;
        }
        let names = match down.as_slice() {
            [one] => format!("{one} is"),
            [first @ .., last] => format!("{} and {last} are", first.join(", ")),
            [] => unreachable!("checked above"),
        };
        let text = format!("{names} unreachable. Reconnecting.");
        if self.machines.notice.as_deref() == Some(text.as_str()) {
            return false;
        }
        self.machines.notice = Some(text.clone());
        self.machines.auto_notice = Some(text);
        true
    }
    /// Server/thread ids remain a pair. Only one server's records enter the State facade.
    pub fn select_machine(&mut self, server: &str) {
        self.machines.picker = false;
        if server == self.machines.selected {
            return;
        }
        let projection = self.machines.views.get(server).cloned();
        if projection.is_none()
            && server != "local"
            && !self
                .client
                .as_ref()
                .is_some_and(|client| client.linked.contains_key(server))
        {
            self.machines.notice =
                Some("This machine is unreachable and has no cached workspace yet.".into());
            return;
        }
        self.machines
            .scope_server
            .get_or_insert_with(|| self.machines.selected.clone());
        let filter = (
            self.project_scope,
            self.scope_additional.clone(),
            self.machines.scope_server.clone(),
            self.machines.scope_remote.clone(),
        );
        self.checkpoint_client();
        self.machines.threads.insert(
            self.machines.selected.clone(),
            std::mem::take(&mut self.threads),
        );
        self.machines.recover.insert(
            self.machines.selected.clone(),
            std::mem::take(&mut self.client_recover),
        );
        self.machines
            .selection
            .insert(self.machines.selected.clone(), self.client_select.take());
        self.machines.selected = server.into();
        self.threads = self
            .machines
            .threads
            .remove(server)
            .unwrap_or_else(|| vec![Thread::new(1)]);
        self.active = 0;
        self.tabs.clear();
        self.client_select = self.machines.selection.remove(server).flatten();
        self.client_recover = self.machines.recover.remove(server).unwrap_or_default();
        self.selected_threads.clear();
        self.project_scope = ProjectScope::All;
        self.scope_additional.clear();
        self.project_picker = None;
        self.captain_editor = None;
        self.row_menu = None;
        self.undo = None;
        // Theme, zoom and the sidebar's layout belong to this device, not to the machine. A
        // machine's saved UI only brings back its drafts, tabs, open thread and workspace filter.
        let device = (
            self.dark,
            self.zoom,
            self.sidebar,
            self.sidebar_width,
            self.shelves,
            self.background.settings.clone(),
        );
        self.client_views = json!({"localUi":self.machines.ui[server]});
        if let Some(projection) = projection {
            if let Err(error) = self.apply_projection(projection) {
                self.storage_error = Some(error);
            }
        } else {
            // No target records are available yet. Keep commands disabled and show the
            // existing reconnecting state, never the previous machine's folders or captains.
            self.projects.entries.clear();
            self.captains.clear();
            self.captain_scopes.clear();
            self.captain_homes.clear();
            self.tasks.clear();
        }
        (
            self.dark,
            self.zoom,
            self.sidebar,
            self.sidebar_width,
            self.shelves,
            self.background.settings,
        ) = device;
        (
            self.project_scope,
            self.scope_additional,
            self.machines.scope_server,
            self.machines.scope_remote,
        ) = filter;
    }

    /// KSG useDraftEnvironmentRouting locks promoted, started and running conversations.
    pub fn can_retarget_draft(&self) -> bool {
        let thread = self.current();
        !thread.captain_home
            && !thread.delegated
            && !thread.archived()
            && thread.messages.is_empty()
            && thread.session.is_none()
            && thread.life.latest_user_message_at.is_none()
            && thread.life.last_completed_at.is_none()
            && !thread.status.busy()
            && !thread.configuring
            && !thread.queued()
            && thread.remote_send.is_none()
            && thread.client_creating.is_none()
            && thread.client_promoted.is_none()
    }

    /// Retarget only the unsent editor. No command runs on either host until Send.
    pub fn retarget_draft(&mut self, server: &str) {
        if !self.can_retarget_draft() || !self.machine_connected(server) {
            return;
        }
        if server == self.machines.selected {
            self.machines.picker = false;
            return;
        }
        let source = (self.machines.selected.clone(), self.current().id);
        let draft = self.current().draft.clone();
        let images = self.current().draft_images.clone();
        self.select_machine(server);
        if self.machines.selected != server {
            return;
        }
        self.new_client_draft(None);
        self.current_mut().draft = draft;
        self.current_mut().draft_images = images;
        if let Some(thread) = self
            .machines
            .threads
            .get_mut(&source.0)
            .and_then(|threads| threads.iter_mut().find(|thread| thread.id == source.1))
        {
            thread.draft.replace_text("");
            thread.draft_images.clear();
        }
        // Update the old machine's saved editor too, so a later switch cannot restore a copy.
        self.machines.ui[&source.0]["drafts"][source.1.to_string()] = json!("");
        self.open_picker(ProjectPicker::NewThread);
        self.machines.draft_picker = Some(server.into());
        self.checkpoint();
    }

    pub(super) fn new_client_draft(&mut self, project: Option<usize>) {
        if project.is_some() && self.project(project).is_none() {
            return;
        }
        let index = self
            .threads
            .iter()
            .position(|thread| {
                thread.client_draft && thread.project == project && thread.blank_draft()
            })
            .unwrap_or_else(|| {
                let id = self
                    .threads
                    .iter()
                    .filter(|t| t.client_draft)
                    .map(|t| t.id)
                    .min()
                    .unwrap_or(usize::MAX)
                    .saturating_sub(1);
                let mut thread = Thread::new(id);
                thread.client_draft = true;
                thread.harness = self.default_harness.clone();
                thread.project = project;
                self.threads.push(thread);
                self.threads.len() - 1
            });
        self.project_picker = None;
        self.show_archive = false;
        self.select_thread(index);
        self.apply_catalog();
    }

    pub fn choose_draft_project(&mut self, project: Option<usize>) {
        if self.machines.draft_picker.as_ref() == Some(&self.machines.selected)
            && self.current().client_draft
        {
            self.current_mut().project = project;
            self.project_picker = None;
            self.machines.draft_picker = None;
            self.checkpoint();
        } else {
            self.new_thread_in(project);
        }
    }

    pub(super) fn promote_client_draft(&mut self) {
        if !self.current().has_draft_content()
            || self.current().client_creating.is_some()
            || self.current().remote_send.is_some()
            || self.current().client_promoted.is_some()
        {
            return;
        }
        let thread = self.current();
        let params = json!({"project":thread.project,"clientDraft":thread.id,
            "occupiedDrafts":self.threads.iter().filter(|t| !t.client_draft && t.has_draft_content()).map(|t| t.id).collect::<Vec<_>>(),
            "harness":thread.harness,"selectedModel":thread.selected_model,"thinking":thread.selected_thinking,"access":thread.native.access.id()});
        match self.remote("thread.create", params).expect("client") {
            Ok(command) => {
                let thread = self.current_mut();
                thread.client_creating = Some(command);
                thread.remote_send =
                    Some((command, thread.draft.clone(), thread.draft_images.clone()));
            }
            Err(error) => self.storage_error = Some(error),
        }
    }

    /// Follow the owning server after Send, even if the user navigated before its receipt.
    fn complete_client_creation(&mut self, server: &str, command: &Command, value: &Value) {
        let commands = self
            .client
            .as_ref()
            .and_then(|client| client.commands_for(server))
            .cloned();
        let threads = if server == self.machines.selected {
            &mut self.threads
        } else {
            let Some(threads) = self.machines.threads.get_mut(server) else {
                return;
            };
            threads
        };
        let Some(draft) = threads.iter_mut().find(|thread| {
            thread.client_draft
                && json!(thread.id) == command.params["clientDraft"]
                && thread.client_creating == Some(command.command_id)
        }) else {
            return;
        };
        draft.client_creating = None;
        let submitted = draft.remote_send.take();
        if value["result"]["status"] != "accepted" {
            let error = value["error"]["message"]
                .as_str()
                .or(value["result"]["result"]["error"].as_str())
                .unwrap_or("Thread creation was not confirmed. Your draft was kept.")
                .to_string();
            draft.error = Some(error.clone());
            self.machines.notice = Some(error);
            return;
        }
        let Some(id) = value["result"]["result"]["threadId"].as_u64() else {
            return;
        };
        draft.client_promoted = Some(id as usize);
        let Some((_, edit, images)) = submitted else {
            return;
        };
        let params = json!({"threadId":id,"text":edit.text,"attachments":images,
            "editorRevision":uuid::Uuid::new_v4(),"mode":"prompt"});
        let result = commands
            .ok_or_else(|| "This machine is not linked. Your draft was kept.".to_string())
            .and_then(|commands| commands.send("thread.send", params));
        match result {
            Ok(id) => draft.remote_send = Some((id, edit, images)),
            Err(error) => {
                draft.error = Some(error.clone());
                self.machines.notice = Some(error);
            }
        }
    }

    fn finish_client_promotions(&mut self) {
        let promotions: Vec<_> = self
            .threads
            .iter()
            .filter_map(|t| t.client_promoted.map(|id| (t.id, id)))
            .collect();
        for (draft_id, id) in promotions {
            let Some(index) = self.threads.iter().position(|t| t.id == id) else {
                continue;
            };
            let Some(draft_index) = self.threads.iter().position(|t| t.id == draft_id) else {
                continue;
            };
            let text = std::mem::take(&mut self.threads[draft_index].draft);
            let images = std::mem::take(&mut self.threads[draft_index].draft_images);
            let sending = self.threads[draft_index].remote_send.take();
            self.threads[draft_index].client_promoted = None;
            self.threads[draft_index].client_creating = None;
            self.threads[index].draft = text;
            self.threads[index].draft_images = images;
            self.threads[index].remote_send = sending;
            for tab in &mut self.tabs {
                if *tab == draft_id {
                    *tab = id;
                }
            }
            if self.current().id == draft_id {
                self.select_thread(index);
            }
            self.checkpoint();
        }
    }

    pub(super) fn project_selection_purpose(&self) -> (&'static str, Option<usize>) {
        if self.captain_editor.is_some() {
            ("captain", None)
        } else if self.projects.return_picker == Some(ProjectPicker::CaptainScope) {
            ("scope", self.current().captain)
        } else {
            ("project", None)
        }
    }

    pub(super) fn finish_project_selection(&mut self) {
        let server = self.machines.selected.clone();
        let Some((id, purpose, captain)) = self.machines.project_selection.get(&server).cloned()
        else {
            return;
        };
        if self.project(Some(id)).is_none() {
            return;
        }
        self.machines.project_selection.remove(&server);
        match purpose.as_str() {
            "captain" => self.captain_project = Some(Some(id)),
            "scope" => {
                if let Some(captain) = captain {
                    let mut scope = self.captain_scope(captain);
                    if !scope.contains(&id) {
                        scope.push(id);
                    }
                    let result = self.set_captain_scope(captain, scope);
                    self.brief.error = result.err();
                }
            }
            _ => self.set_project_scope(Some(id), false),
        }
        self.projects.return_picker = None;
        self.checkpoint();
    }

    pub fn retarget_captain(&mut self, server: &str) {
        if !self.machine_connected(server)
            || self
                .captain_editor
                .as_ref()
                .is_none_or(|editor| editor.editing.is_some())
        {
            return;
        }
        let editor = self.captain_editor.take();
        self.select_machine(server);
        self.captain_editor = editor;
        if self.machines.selected == server {
            let editor = self.captain_editor.as_mut().unwrap();
            editor.server = server.into();
            editor.machine_picker = false;
            self.captain_project = Some(None);
            self.projects.captain_target = None;
        }
    }
    /// Opens a thread on any linked machine, as one click in the unified sidebar.
    pub fn open_machine_thread(&mut self, server: &str, id: usize) {
        let exists = if server == self.machines.selected {
            self.threads.iter().any(|thread| thread.id == id)
        } else {
            self.machines.views.get(server).is_some_and(|projection| {
                projection["index"]["threads"]
                    .as_array()
                    .is_some_and(|threads| {
                        threads
                            .iter()
                            .any(|thread| thread["id"].as_u64() == Some(id as u64))
                    })
            })
        };
        if !exists {
            self.machines.notice =
                Some("This thread is no longer available on that machine.".into());
            return;
        }
        self.select_machine(server);
        if self.machines.selected == server
            && let Some(index) = self.threads.iter().position(|thread| thread.id == id)
        {
            self.select_thread(index);
        }
    }
    pub fn machine_action(&mut self, action: &str, target: &str) {
        if self.machines.pending.is_some() {
            return;
        }
        let profile = self.data_dir.clone();
        let host = self.machines.host.text.clone();
        let code = self.machines.code.text.clone();
        self.machines.code.replace_text("");
        let action = action.to_owned();
        let target = target.to_owned();
        let wake = self.wake.clone();
        let (send, receive) = std::sync::mpsc::channel();
        self.machines.pending = Some(receive);
        self.machines.notice = None;
        std::thread::spawn(move || {
            let result = tokio::runtime::Runtime::new()
                .map_err(|e| e.to_string())
                .and_then(|runtime| {
                    runtime.block_on(async {
                        use crate::peer::MachineResult;
                        match action.as_str() {
                            "link" => crate::peer::link(&profile, &host, &code)
                                .await
                                .map(MachineResult::Linked),
                            "pair" => crate::client::local_rpc(&profile, "devices.pair", json!({}))
                                .await
                                .map(MachineResult::Paired),
                            "unlink" => crate::peer::unlink(&profile, &target)
                                .await
                                .map(|()| MachineResult::Unlinked(target)),
                            "devices" => {
                                crate::client::local_rpc(&profile, "devices.list", json!({}))
                                    .await
                                    .map(MachineResult::Devices)
                            }
                            "revoke" => {
                                let receipt = crate::client::local_rpc(
                                    &profile,
                                    "devices.revoke",
                                    json!({"deviceId":target}),
                                )
                                .await?;
                                if receipt["status"] != "accepted" {
                                    return Err("Device revocation refused.".into());
                                }
                                crate::client::local_rpc(&profile, "devices.list", json!({}))
                                    .await
                                    .map(MachineResult::Devices)
                            }
                            _ => Err("Unknown machine action.".into()),
                        }
                    })
                });
            let _ = send.send(result);
            wake();
        });
    }
    pub(crate) fn forward(&mut self, method: &str, params: Value) -> bool {
        if self.client.is_some() && self.current().client_draft {
            match method {
                "thread.configure" => return false,
                "thread.visit" => return true,
                _ => {}
            }
        }
        if let Some(result) = self.remote(method, params) {
            if let Err(error) = result {
                self.storage_error = Some(error);
            }
            true
        } else {
            false
        }
    }

    pub fn execute_command(&mut self, command: &Command) -> Result<Value, String> {
        let args = &command.params;
        if let Some(id) = args.get("threadId") {
            self.active = self
                .threads
                .iter()
                .position(|thread| json!(thread.id) == *id)
                .ok_or("Unknown threadId.")?;
        }
        let text = |key: &str| {
            args[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} must be text."))
        };
        let number = |key: &str| {
            args[key]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| format!("{key} must be an id."))
        };
        if command.method.starts_with("queue.") {
            if args["generation"] != json!(self.current().request_generation) {
                return Err("The queue belongs to an earlier runtime generation.".into());
            }
            let id = number("inputId")?;
            if !self
                .current()
                .native
                .queue
                .iter()
                .any(|input| input.id == id)
            {
                return Err("This queued input was already changed or removed.".into());
            }
        }
        match command.method.as_str() {
            "thread.send" => {
                self.current_mut().recovery_command = None;
                if self.current().phase == "Stopping" {
                    return Err("Wait for the stop to finish.".into());
                }
                let input: crate::protocol::SendInput =
                    serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
                let text = input.text;
                let attachments = input
                    .attachments
                    .iter()
                    .map(|value| Attachment::from_value(value).ok_or("Invalid local attachment."))
                    .collect::<Result<Vec<_>, _>>()?;
                attachments::validate_prompt_images(&attachments)?;
                self.current_mut().draft.replace_text(text);
                self.current_mut().draft_images = attachments;
                self.current_mut().native.follow_up =
                    matches!(input.mode, crate::protocol::SendMode::Queue);
                self.send();
                if self.current().has_draft_content() {
                    let error = self
                        .current()
                        .error
                        .clone()
                        .or(self.current().attachment_error.clone())
                        .unwrap_or("The thread cannot accept input now.".into());
                    self.current_mut().draft.replace_text("");
                    self.current_mut().draft_images.clear();
                    return Err(error);
                }
            }
            "thread.stop" => {
                self.current_mut().recovery_command = Some(command.command_id);
                self.current_mut().stop();
                if self.current().has_draft_content() {
                    let input = json!({"text":self.current().draft.text,"attachments":self.current().draft_images,"commandId":command.command_id});
                    self.recovered_inputs.insert(self.current().id, input);
                    self.current_mut().draft.replace_text("");
                    self.current_mut().draft_images.clear();
                }
            }
            "thread.configure" => {
                if let Some(harness) = args["harness"].as_str() {
                    self.choose_harness(harness);
                }
                if let Some(model) = args["model"].as_str() {
                    self.current_mut()
                        .set_model(text("provider")?, model.into());
                }
                if let Some(thinking) = args["thinking"].as_str() {
                    self.current_mut().set_thinking(thinking.into());
                }
                if let Some(access) = args["access"].as_str() {
                    self.set_access_mode(AccessMode::parse(access).ok_or("Unknown access mode.")?);
                }
            }
            "thread.retry" => self.retry(),
            "thread.visit" => self.current_mut().life.last_visited_at = Some(lifecycle::now()),
            "thread.create" => {
                let project = args["project"].as_u64().map(|id| id as usize);
                if let Some(harness) = args["harness"].as_str() {
                    self.validate_harness(harness)?;
                }
                if project.is_some() && self.project(project).is_none() {
                    return Err("Unknown project.".into());
                }
                let occupied = args["occupiedDrafts"].as_array();
                let reuse_occupied = self
                    .threads
                    .iter()
                    .find(|t| t.project == project && t.blank_draft())
                    .is_some_and(|t| occupied.is_some_and(|ids| ids.contains(&json!(t.id))));
                if reuse_occupied {
                    let mut thread = Thread::new(self.next_id);
                    self.next_id += 1;
                    thread.project = project;
                    self.threads.push(thread);
                    self.select_thread(self.threads.len() - 1);
                } else {
                    self.new_thread_in(project);
                }
                if let Some(harness) = args["harness"].as_str() {
                    self.choose_harness(harness);
                }
                if let Ok(Some((provider, model))) = serde_json::from_value::<
                    Option<(String, String)>,
                >(args["selectedModel"].clone())
                {
                    self.current_mut().set_model(provider, model);
                }
                if let Some(thinking) = args["thinking"].as_str() {
                    self.current_mut().set_thinking(thinking.into());
                }
                if let Some(access) = args["access"].as_str().and_then(AccessMode::parse) {
                    self.current_mut().native.access = access;
                }
                return Ok(json!({"threadId":self.current().id}));
            }
            "thread.rename" => {
                let title = text("title")?;
                self.rename_thread = Some(self.current().id);
                self.rename_edit.replace_text(title);
                self.save_thread_name();
                if self.rename_thread.take().is_some() {
                    return Err("Invalid thread title.".into());
                }
            }
            "thread.delete" => {
                self.delete_thread = Some(self.current().id);
                self.confirm_delete();
            }
            "thread.lifecycle" => {
                let id = self.current().id;
                match args["action"].as_str() {
                    Some("pin") => self.pin_thread(id),
                    Some("archive") => self.archive_thread(id),
                    Some("settle") => self.settle_thread(id),
                    Some("unsnooze") => self.unsnooze_thread(id),
                    Some("snooze") => {
                        self.snooze_thread(id, args["until"].as_u64().ok_or("Missing until.")?)
                    }
                    Some("autoSettle") => {
                        self.current_mut().life.auto_settle_disabled_at =
                            if args["disabled"] == true {
                                Some(lifecycle::now())
                            } else {
                                None
                            };
                    }
                    _ => return Err("Unknown lifecycle action.".into()),
                }
                if let Some(error) = self.lifecycle_error.take() {
                    return Err(error);
                }
            }
            "thread.undo" => {
                if json!(self.undo) != args["expected"] {
                    return Err("The undo action changed or expired.".into());
                }
                self.undo_thread_action();
            }
            "thread.drop" => {
                let to = match args["to"].as_str() {
                    Some("Pinned") => Section::Pinned,
                    Some("Active") => Section::Active,
                    Some("Settled") => Section::Settled,
                    Some("Snoozed") => Section::Snoozed,
                    _ => return Err("Unknown drop section.".into()),
                };
                self.drop_thread(
                    self.current().id,
                    to,
                    args["before"].as_u64().map(|id| id as usize),
                );
            }
            "settings.autoSettle" => {
                self.auto_settle_days = args["days"]
                    .as_u64()
                    .filter(|days| *days <= 3650)
                    .ok_or("Invalid inactivity interval.")?
                    as u32;
                self.sweep_lifecycle(lifecycle::now());
            }
            "settings.defaultHarness" => {
                let harness = text("harness")?;
                if !self.harness_ids().contains(&harness) {
                    return Err(format!("Unknown harness {harness}."));
                }
                self.default_harness = harness;
                self.checkpoint();
            }
            "background.keepAwake" => self.set_keep_awake(
                serde_json::from_value(args["policy"].clone()).map_err(|e| e.to_string())?,
            ),
            "thread.compact" => {
                self.compact_context(args["instructions"].as_str().map(str::to_owned))
            }
            "thread.fork" => self.fork_conversation(args["entry"].as_str().map(str::to_owned)),
            "thread.rewind" => {
                self.current_mut().recovery_command = Some(command.command_id);
                self.current_mut().rewind(text("entry")?);
            }
            "thread.forkPoints" => self.conversation_points(if args["rewind"] == true {
                ConversationMode::Rewind
            } else {
                ConversationMode::Fork
            }),
            "request.respond" => {
                let input: crate::protocol::RequestResponse =
                    serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
                let request = self.current().request.as_ref().ok_or("Request expired.")?;
                if self.current().id != input.thread_id
                    || request["id"] != input.request_id
                    || input.generation != self.current().request_generation
                {
                    return Err("Request or runtime generation changed.".into());
                }
                if input.session && request["method"] != "confirm" {
                    return Err("Session approval requires a confirmation request.".into());
                }
                if let Some(answers) = input
                    .answers
                    .filter(Value::is_array)
                    .filter(|_| input.response["cancelled"] != true)
                {
                    self.current_mut().respond_projected_form(answers)?;
                } else if input.session {
                    self.current_mut().approve_for_session();
                } else {
                    self.respond_to_request(input.thread_id, input.response)?;
                }
            }
            "queue.remove" => self.current_mut().cancel_queued(number("inputId")?),
            "queue.sendNow" => self.current_mut().steer_queued(number("inputId")?),
            "queue.edit" => {
                self.current_mut().edit_queued(number("inputId")?);
                let input = json!({"text":self.current().draft.text,"attachments":self.current().draft_images});
                self.current_mut().draft.replace_text("");
                self.current_mut().draft_images.clear();
                return Ok(input);
            }
            "input.claim" => {
                let id = self.current().id;
                if let Some(expected) = args.get("expectedCommandId")
                    && self
                        .recovered_inputs
                        .get(&id)
                        .is_none_or(|input| input["commandId"] != *expected)
                {
                    return Err("This recovered input belongs to a different command.".into());
                }
                return self
                    .recovered_inputs
                    .remove(&id)
                    .ok_or("No recoverable input.".into());
            }
            "project.put" => {
                let path = crate::projects::expand_path(&text("path")?);
                if !path.is_absolute() {
                    return Err("A workspace needs an absolute path on this server.".into());
                }
                if args["createFolder"] == true {
                    std::fs::create_dir_all(&path)
                        .map_err(|_| "That folder could not be created.")?;
                }
                let id = self
                    .add_project(path)?
                    .ok_or("The owning server did not register the project.")?;
                return Ok(json!({"projectId":id}));
            }
            "project.remove" => self.remove_project(number("projectId")?),
            "project.clone" => {
                self.projects.clone_source.replace_text(text("source")?);
                self.projects.clone_parent = PathBuf::from(text("parent")?);
                self.start_clone();
                if let Some(error) = &self.projects.clone_error {
                    return Err(error.clone());
                }
            }
            "project.cancelClone" => self.close_clone_dialog(),
            "catalog.refresh" => self.refresh_catalog(),
            "commands.refresh" => self.current_mut().refresh_commands(),
            "captain.open" => {
                self.captain_project = args
                    .get("project")
                    .map(|value| serde_json::from_value::<Option<ProjectId>>(value.clone()))
                    .transpose()
                    .map_err(|e| e.to_string())?;
                self.projects.captain_target = args["assignProject"].as_u64().map(|id| id as usize);
                self.open_captain(number("captain")?);
                return Ok(json!({"threadId":self.current().id}));
            }
            "captain.put" => {
                let captain =
                    Captain::from_value(&args["record"]).ok_or("Invalid captain record.")?;
                captain.validate()?;
                let editing = args["captain"].as_u64().map(|id| id as usize);
                if editing.is_some_and(|id| id >= self.captains.len()) {
                    return Err("Unknown captain.".into());
                }
                let mut editor = CaptainEditor::new();
                editor.editing = editing;
                editor.name.replace_text(captain.name);
                editor.role.replace_text(captain.role);
                editor.instructions.replace_text(captain.instructions);
                editor.portrait = captain.portrait;
                self.captain_editor = Some(editor);
                self.captain_project = args
                    .get("project")
                    .map(|value| serde_json::from_value::<Option<ProjectId>>(value.clone()))
                    .transpose()
                    .map_err(|e| e.to_string())?;
                self.projects.captain_target = args["assignProject"].as_u64().map(|id| id as usize);
                self.save_captain();
                if let Some(editor) = self.captain_editor.take() {
                    return Err(editor.error.unwrap_or("Could not save the captain.".into()));
                }
            }
            "captain.scope" => self.set_captain_scope(
                number("captain")?,
                serde_json::from_value(args["projects"].clone()).map_err(|e| e.to_string())?,
            )?,
            "captain.reset" => self.reset_captain_conversation(number("captain")?)?,
            "memory.put" => {
                let captain = number("captain")?;
                if let Some(id) = args["id"].as_str() {
                    if args["text"].is_string() {
                        self.edit_memory(captain, id, text("text")?)?;
                    }
                    if args["essential"].is_boolean() {
                        self.set_memory_essential(captain, id, args["essential"] == true)?;
                    }
                } else {
                    return self
                        .add_memory(captain, text("text")?, args["essential"] == true)
                        .map(|id| json!({"id":id}));
                }
            }
            "memory.remove" => self.delete_memory(number("captain")?, &text("id")?)?,
            "profile.put" => self.set_user_profile(
                serde_json::from_value(args["profile"].clone()).map_err(|e| e.to_string())?,
            )?,
            "routine.pause" => self.set_routine_paused(
                args["id"].as_u64().ok_or("Missing routine id.")?,
                args["paused"] == true,
            )?,
            "routine.remove" => {
                self.delete_routine(args["id"].as_u64().ok_or("Missing routine id.")?)?
            }
            "task.resume" => self.resume_task(&text("taskId")?)?,
            "task.dismiss" => self.dismiss_task(&text("taskId")?)?,
            "task.resumeResults" => self.resume_task_results(self.current().id)?,
            "server.stop" => self.runtime_stop_requested = true,
            "task.delegate" | "task.send" | "task.cancel" => {
                let tool = match command.method.as_str() {
                    "task.delegate" => "delegate_task",
                    "task.send" => "task_send",
                    _ => "task_cancel",
                };
                return self.mcp_call(self.current().id, tool, &args["args"]);
            }
            _ => return Err(format!("Unsupported slice 1 method {}.", command.method)),
        }
        Ok(json!({"threadId":self.current().id}))
    }
}

impl Thread {
    pub(super) fn runtime_inputs_value(&self) -> Value {
        let mut inputs = self
            .pending_inputs_value()
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some((prompt, after)) = &self.queued_prompt {
            inputs.push(json!({"text":prompt.text,"images":prompt.images.iter().map(Attachment::to_value).collect::<Vec<_>>(),"after":after}));
        }
        if let Some((_, prompt, after)) = &self.pending_prompt {
            inputs.push(json!({"text":prompt.text,"images":prompt.images.iter().map(Attachment::to_value).collect::<Vec<_>>(),"after":after}));
        }
        json!(inputs)
    }
    pub(super) fn forward(&mut self, method: &str, mut params: Value) -> bool {
        let Some(remote) = &self.remote else {
            return false;
        };
        params["threadId"] = json!(self.id);
        if let Err(error) = remote.send(method, params) {
            self.error = Some(error);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeState;

    fn paired_client_fixture() -> (
        State,
        tokio::sync::mpsc::Receiver<Command>,
        tokio::sync::mpsc::Receiver<Command>,
        Value,
    ) {
        let (runtime, _) = wired();
        let projection = runtime.state.projection();
        let mut state = State::new(PathBuf::new(), false);
        let (mut client, local) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, commands) = crate::client::Client::test_connected(state.data_dir.clone());
        client.linked.insert("studio".into(), remote);
        state.client = Some(client);
        state
            .machines
            .views
            .insert("local".into(), projection.clone());
        state
            .machines
            .views
            .insert("studio".into(), projection.clone());
        state.apply_projection(projection.clone()).unwrap();
        (state, local, commands, projection)
    }

    #[test]
    fn retargeting_keeps_the_editor_and_images_and_creates_only_on_send() {
        let (mut state, mut local, mut remote, projection) = paired_client_fixture();
        let image = Attachment {
            id: "draft.png".into(),
            name: "draft.png".into(),
            mime: "image/png".into(),
            width: 1,
            height: 1,
            bytes: 4,
            path: None,
        };
        state
            .current_mut()
            .draft
            .replace_text("Keep the exact draft");
        state.current_mut().draft_images.push(image.clone());
        state.retarget_draft("studio");
        assert_eq!(state.machines.selected, "studio");
        assert!(state.current().client_draft);
        assert_eq!(state.current().draft.text, "Keep the exact draft");
        assert_eq!(state.current().draft_images, [image.clone()]);
        assert_eq!(state.machines.draft_picker.as_deref(), Some("studio"));
        assert!(local.try_recv().is_err());
        assert!(
            remote.try_recv().is_err(),
            "retargeting must not send thread.create or visit"
        );
        state.choose_draft_project(None);
        let id = state.current().id;
        state.apply_projection(projection.clone()).unwrap();
        assert_eq!(state.current().id, id);
        assert_eq!(state.current().draft.text, "Keep the exact draft");
        state.send();
        let create = remote.try_recv().unwrap();
        assert_eq!(create.method, "thread.create");
        assert_eq!(create.params["clientDraft"], json!(id));
        state.send();
        assert!(
            remote.try_recv().is_err(),
            "a second Send cannot duplicate promotion"
        );
        state.machines.receipts.insert(
            "studio".into(),
            vec![crate::client::Event::Receipt(
                create,
                json!({"result":{"status":"accepted","result":{"threadId":1}}}),
            )],
        );
        state.poll_client(&mut Tx::default());
        let send = remote.try_recv().unwrap();
        assert_eq!(send.method, "thread.send");
        let visit = remote.try_recv().unwrap();
        assert_eq!(visit.method, "thread.visit");
        assert_eq!(send.params["text"], "Keep the exact draft");
        assert_eq!(send.params["attachments"][0]["id"], image.id);
        assert_eq!(state.current().id, 1);
        assert!(!state.current().client_draft);
        assert_eq!(
            state.current().draft.text,
            "Keep the exact draft",
            "clear only on send receipt"
        );
        state.select_machine("local");
        assert!(
            state.current().draft.text.is_empty(),
            "the draft moved, it was not copied"
        );
    }

    #[test]
    fn client_draft_metadata_and_editor_restore_without_server_records() {
        let (mut state, _, _, projection) = paired_client_fixture();
        state.new_thread_in(None);
        let id = state.current().id;
        state
            .current_mut()
            .draft
            .replace_text("Persist this unsent draft");
        state.checkpoint_client();
        let saved = state.machines.ui["local"].clone();
        let mut restored = State::new(PathBuf::new(), false);
        restored.client = Some(crate::client::Client::new(restored.data_dir.clone()));
        restored.client_views["localUi"] = saved;
        restored.apply_projection(projection).unwrap();
        assert_eq!(restored.current().id, id);
        assert!(restored.current().client_draft);
        assert_eq!(restored.current().draft.text, "Persist this unsent draft");
    }

    #[test]
    fn a_promotion_receipt_submits_the_captured_revision_on_its_host_after_navigation() {
        let (mut state, mut local, mut remote, _) = paired_client_fixture();
        state.retarget_draft("studio");
        state
            .current_mut()
            .draft
            .replace_text("The submitted revision");
        state.send();
        let create = remote.try_recv().unwrap();
        state
            .current_mut()
            .draft
            .replace_text("New edits after Send");
        state.select_machine("local");
        state.complete_client_creation(
            "studio",
            &create,
            &json!({"result":{"status":"accepted","result":{"threadId":1}}}),
        );
        assert_eq!(state.machines.selected, "local");
        assert!(local.try_recv().is_err());
        let sent = remote.try_recv().unwrap();
        assert_eq!(sent.method, "thread.send");
        assert_eq!(sent.params["text"], "The submitted revision");
        state.select_machine("studio");
        state.finish_client_promotions();
        assert_eq!(state.current().id, 1);
        assert_eq!(state.current().draft.text, "New edits after Send");
        state.machines.receipts.insert(
            "studio".into(),
            vec![crate::client::Event::Receipt(
                sent,
                json!({"result":{"status":"accepted","result":{}}}),
            )],
        );
        state.poll_client(&mut Tx::default());
        assert_eq!(
            state.current().draft.text,
            "New edits after Send",
            "the receipt cannot clear a later editor revision"
        );
    }

    #[test]
    fn started_and_offline_threads_cannot_be_retargeted() {
        let (mut state, _, mut remote, _) = paired_client_fixture();
        state.current_mut().draft.replace_text("Stay here");
        state.current_mut().session = Some("started".into());
        state.retarget_draft("studio");
        assert_eq!(state.machines.selected, "local");
        state.current_mut().session = None;
        state.current_mut().life.latest_user_message_at = Some(1);
        assert!(
            !state.can_retarget_draft(),
            "history may not have loaded on an offline host"
        );
        state.current_mut().life.latest_user_message_at = None;
        state.client.as_mut().unwrap().linked.insert(
            "studio".into(),
            crate::client::Client::new(state.data_dir.clone()),
        );
        state.retarget_draft("studio");
        assert_eq!(state.machines.selected, "local");
        assert_eq!(state.current().draft.text, "Stay here");
        assert!(remote.try_recv().is_err());
    }

    #[test]
    fn project_receipts_wait_for_the_project_projection_and_never_select_threads() {
        let (mut state, _, _, mut projection) = paired_client_fixture();
        let active = state.current().id;
        let command = Command::new(
            "project.put",
            json!({"path":"/srv/new","selectFor":"project"}),
        );
        state.machines.receipts.insert(
            "local".into(),
            vec![crate::client::Event::Receipt(
                command,
                json!({"result":{"status":"accepted","result":{"projectId":9,"threadId":999}}}),
            )],
        );
        state.poll_client(&mut Tx::default());
        assert_eq!(state.current().id, active);
        assert!(state.client_select.is_none());
        assert!(state.machines.project_selection.contains_key("local"));
        projection["index"]["projects"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":9,"name":"New","path":"/srv/new","machine":"local"}));
        projection["index"]["nextProjectId"] = json!(10);
        state.apply_projection(projection).unwrap();
        state.finish_project_selection();
        assert_eq!(state.project_scope, ProjectScope::Project(9));
        assert_eq!(state.current().id, active);
        assert!(state.machines.project_selection.is_empty());
    }

    #[test]
    fn captain_target_is_one_server_and_switching_preserves_the_form() {
        let (mut state, mut local, mut remote, _) = paired_client_fixture();
        state.begin_new_captain();
        state
            .captain_editor
            .as_mut()
            .unwrap()
            .name
            .replace_text("Studio captain");
        state.retarget_captain("studio");
        assert_eq!(state.machines.selected, "studio");
        assert_eq!(state.captain_editor.as_ref().unwrap().server, "studio");
        assert_eq!(
            state.captain_editor.as_ref().unwrap().name.text,
            "Studio captain"
        );
        state.save_captain();
        let create = remote.try_recv().unwrap();
        assert_eq!(create.method, "captain.put");
        assert_eq!(create.params["record"]["name"], "Studio captain");
        assert!(create.params["threadId"].is_null());
        assert!(local.try_recv().is_err());
        state.captains = Captain::presets();
        state.edit_captain(0);
        state.retarget_captain("local");
        assert_eq!(
            state.machines.selected, "studio",
            "editing cannot change the home server"
        );
        state.captain_editor = None;
        state.open_picker(ProjectPicker::CaptainScope);
        state.open_picker(ProjectPicker::AddProject);
        state.browse_for_project();
        state.open_picker(ProjectPicker::AddProject);
        assert_eq!(
            state.projects.return_picker,
            Some(ProjectPicker::CaptainScope)
        );
        state.choose_project_machine("local");
        assert_eq!(
            state.machines.selected, "studio",
            "scope browsing stays on the captain's server after Back"
        );
    }

    #[test]
    fn selecting_a_cached_machine_restores_tabs_before_the_next_projection() {
        let (mut state, _, _, _) = paired_client_fixture();
        state.select_machine("studio");
        state.tabs = vec![1];
        state.current_mut().draft.replace_text("Studio draft");
        state.select_machine("local");
        state.select_machine("studio");
        assert_eq!(state.tabs, [1]);
        assert_eq!(state.current().draft.text, "Studio draft");
    }

    #[test]
    fn uncached_machines_show_connecting_without_the_previous_hosts_projects() {
        let (mut state, _, _, _) = paired_client_fixture();
        state.machines.views.remove("studio");
        state.select_machine("studio");
        assert_eq!(state.machines.selected, "studio");
        assert!(state.projects.entries.is_empty());
        assert_eq!(state.unreachable_selected().as_deref(), Some("studio"));
    }

    #[test]
    fn asynchronous_project_adds_have_no_fake_id_and_clone_preview_matches_submit() {
        let (mut state, mut local, mut remote, _) = paired_client_fixture();
        assert_eq!(state.add_project(PathBuf::from("/srv/new")).unwrap(), None);
        let project = local.try_recv().unwrap();
        assert_eq!(project.method, "project.put");
        state.select_machine("studio");
        state.projects.clone_source.replace_text("owner/repo");
        state.projects.clone_parent = PathBuf::from("/stale");
        state
            .projects
            .clone_path
            .replace_text(r"D:\Code With Spaces");
        assert_eq!(
            state.clone_target().unwrap().1.to_string_lossy(),
            r"D:\Code With Spaces\repo"
        );
        state.start_clone();
        let command = remote.try_recv().unwrap();
        assert_eq!(command.method, "project.clone");
        assert_eq!(command.params["parent"], r"D:\Code With Spaces");
        assert_eq!(
            state.projects.clone_parent.to_string_lossy(),
            r"D:\Code With Spaces"
        );
    }

    #[test]
    fn machine_views_isolate_equal_thread_ids_drafts_projects_and_commands() {
        let (runtime, _) = wired();
        let mut state = State::new(PathBuf::new(), false);
        let mut client = crate::client::Client::new(state.data_dir.clone());
        client.linked.insert(
            "server-b".into(),
            crate::client::Client::new(state.data_dir.clone()),
        );
        state.client = Some(client);
        let projection = runtime.state.projection();
        state
            .machines
            .views
            .insert("local".into(), projection.clone());
        state
            .machines
            .views
            .insert("server-b".into(), projection.clone());
        state.apply_projection(projection).unwrap();
        state.current_mut().draft.replace_text("Local thread 1");
        state.select_machine("server-b");
        assert_eq!(state.current().id, 1);
        assert_eq!(state.current().draft.text, "");
        state.current_mut().draft.replace_text("Server B thread 1");
        state.select_machine("local");
        assert_eq!(state.current().draft.text, "Local thread 1");
        state.select_machine("server-b");
        assert_eq!(state.current().draft.text, "Server B thread 1");
        state.checkpoint_client();
        assert_eq!(state.machines.ui["local"]["drafts"]["1"], "Local thread 1");
        assert_eq!(
            state.machines.ui["server-b"]["drafts"]["1"],
            "Server B thread 1"
        );
        assert!(
            state
                .remote("thread.stop", json!({"threadId":1}))
                .unwrap()
                .is_err()
        );
        assert!(!state.machine_connected("server-b"));
    }

    #[test]
    fn opening_remote_threads_preserves_device_preferences_and_namespaces() {
        let (runtime, _) = wired();
        let mut state = State::new(PathBuf::new(), false);
        state.client = Some(crate::client::Client::new(state.data_dir.clone()));
        let mut projection = runtime.state.projection();
        projection["index"]["dark"] = json!(true);
        projection["index"]["zoom"] = json!(1.6);
        state
            .machines
            .views
            .insert("local".into(), projection.clone());
        state
            .machines
            .views
            .insert("remote".into(), projection.clone());
        state.apply_projection(projection).unwrap();
        state.dark = false;
        state.zoom = 1.1;
        state.sidebar_width = 312.0;
        state.sidebar = false;
        state.shelves = [true, false];
        state.current_mut().draft.replace_text("local unsent text");
        state.checkpoint_client();
        let mut remote_ui = state.machines.ui["local"].clone();
        remote_ui["dark"] = json!(true);
        remote_ui["zoom"] = json!(1.8);
        remote_ui["sidebar"] = json!(true);
        remote_ui["sidebarWidth"] = json!(200.0);
        remote_ui["shelves"] = json!([false, false, true]);
        remote_ui["drafts"]["1"] = json!("remote unsent text");
        state.machines.ui["remote"] = remote_ui;
        for server in ["remote", "local", "remote", "local"] {
            state.open_machine_thread(server, 1);
            assert_eq!(state.machines.selected, server);
            assert!(!state.dark);
            assert_eq!(state.zoom, 1.1);
            assert_eq!(state.sidebar_width, 312.0);
            assert!(!state.sidebar);
            assert_eq!(state.shelves, [true, false]);
            assert_eq!(
                state.current().draft.text,
                if server == "local" {
                    "local unsent text"
                } else {
                    "remote unsent text"
                }
            );
        }
        state.open_machine_thread("remote", usize::MAX);
        assert_eq!(state.machines.selected, "local");
    }

    #[test]
    fn multi_machine_workspace_filter_toggles_without_aliasing_equal_ids() {
        let mut state = State::new(PathBuf::new(), false);
        state.client = Some(crate::client::Client::new(state.data_dir.clone()));
        state.set_machine_scope("remote".into(), Some(1), false);
        state.set_machine_scope("local".into(), Some(1), true);
        state.set_machine_scope("local".into(), None, true);
        assert!(state.scope_includes_on("remote", Some(1)));
        assert!(state.scope_includes_on("local", Some(1)));
        assert!(state.scope_includes_on("another", None));
        assert!(!state.scope_includes_on("another", Some(1)));
        state.set_machine_scope("local".into(), None, true);
        state.set_machine_scope("local".into(), Some(1), true);
        assert!(!state.scope_includes_on("local", Some(1)));
        assert!(state.scope_includes_on("remote", Some(1)));
        state.set_machine_scope("remote".into(), Some(1), true);
        assert_eq!(state.project_scope, ProjectScope::All);
        assert!(state.machines.scope_remote.is_empty());
    }

    #[test]
    fn remote_project_paths_are_never_normalized_or_inspected_on_the_client() {
        let value =
            json!({"id":1,"name":"Host project","path":"/srv/projects/shina","machine":"local"});
        let project = Project::from_record(&value, false).unwrap();
        assert_eq!(project.path, PathBuf::from("/srv/projects/shina"));
        assert!(project.icon.is_none());
        assert_eq!(project.to_value()["path"], value["path"]);
    }

    fn wired() -> (RuntimeState, mpsc::Receiver<Value>) {
        let mut state = State::headless(crate::store::directory().unwrap());
        state.load_global_for_test();
        let (pi, commands, _) = Pi::test_connection();
        let thread = state.current_mut();
        thread.pi = Some(pi);
        thread.state_loaded = true;
        thread.history_loaded = true;
        thread.startup_selection_verified = true;
        thread.model_provider = "cpa".into();
        thread.model_id = "fixture".into();
        (RuntimeState::from_state(state), commands)
    }

    #[test]
    fn command_receipt_deduplicates_an_accepted_send_and_rejects_changed_content() {
        let (mut runtime, commands) = wired();
        let command = Command::new(
            "thread.send",
            json!({"threadId":1,"text":"one send","attachments":[],"mode":"prompt"}),
        );
        let first = runtime.command("device", command.clone()).unwrap();
        assert_eq!(first["status"], "accepted");
        let index = crate::store::load(&runtime.state.data_dir.join("index.json"))
            .unwrap()
            .unwrap();
        assert_eq!(index["runtimeInputs"]["1"][0]["text"], "one send");
        assert_eq!(index["commandReceipts"][0]["status"], "accepted");
        assert_eq!(runtime.command("device", command.clone()).unwrap(), first);
        let mut changed = command;
        changed.params["text"] = json!("changed");
        assert!(runtime.command("device", changed).is_err());
        assert_eq!(
            commands
                .try_iter()
                .filter(|record| record["type"] == "prompt")
                .count(),
            1
        );
    }

    #[test]
    fn the_server_owns_the_default_harness_and_projects_memory_status() {
        let (mut runtime, _commands) = wired();
        let set = Command::new("settings.defaultHarness", json!({"harness":"codex"}));
        runtime.command("device", set).unwrap();
        assert_eq!(runtime.state.default_harness, "codex");
        let unknown = Command::new("settings.defaultHarness", json!({"harness":"nope"}));
        // A rejected command is reported in its receipt and changes nothing.
        let rejected = runtime.command("device", unknown);
        assert!(
            rejected
                .as_ref()
                .map_or(true, |r| r["status"] != "accepted"),
            "{rejected:?}"
        );
        assert_eq!(runtime.state.default_harness, "codex");
        let projection = runtime.state.projection();
        assert_eq!(projection["index"]["defaultHarness"], "codex");
        assert_eq!(projection["memoryBackend"]["state"], "disabled");
    }

    #[test]
    fn two_projected_clients_keep_their_own_editors_and_never_hold_a_harness() {
        let (runtime, _) = wired();
        let mut a = State::new(PathBuf::new(), false);
        let mut b = State::new(PathBuf::new(), false);
        for state in [&mut a, &mut b] {
            state.client = Some(crate::client::Client::new(state.data_dir.clone()));
            state.apply_projection(runtime.state.projection()).unwrap();
        }
        a.current_mut().draft.replace_text("only A");
        b.current_mut().draft.replace_text("only B");
        a.current_mut().native.queue_expanded = true;
        for state in [&mut a, &mut b] {
            state.apply_projection(runtime.state.projection()).unwrap();
            assert!(state.current().pi.is_none());
        }
        assert_eq!(a.current().draft.text, "only A");
        assert_eq!(b.current().draft.text, "only B");
        assert!(a.current().native.queue_expanded);
        assert!(!b.current().native.queue_expanded);
        assert!(a.store.is_none() && b.store.is_none());
    }

    #[test]
    fn stale_request_generation_cannot_approve_and_a_second_response_fails() {
        let (mut runtime, _commands) = wired();
        let generation = runtime.state.current().request_generation.clone();
        runtime.state.current_mut().request = Some(json!({"id":"approval","method":"confirm"}));
        let mut command = Command::new(
            "request.respond",
            json!({"threadId":1,"requestId":"approval","generation":"old","response":{"confirmed":true}}),
        );
        assert_eq!(
            runtime.command("device", command.clone()).unwrap()["status"],
            "rejected"
        );
        assert!(runtime.state.current().request.is_some());
        command.command_id = uuid::Uuid::new_v4();
        command.params["generation"] = json!(generation);
        assert_eq!(
            runtime.command("device", command.clone()).unwrap()["status"],
            "accepted"
        );
        command.command_id = uuid::Uuid::new_v4();
        assert_eq!(
            runtime.command("second-device", command).unwrap()["status"],
            "rejected"
        );
    }

    #[test]
    fn stop_keeps_recovered_queue_out_of_every_clients_editor() {
        let (mut runtime, _) = wired();
        runtime.state.current_mut().status = Status::Running;
        runtime
            .state
            .current_mut()
            .native
            .queue
            .push_back(pi_session::QueuedMessage {
                id: 7,
                prompt: Prompt::new("queued text".into(), vec![]),
            });
        let command = Command::new("thread.stop", json!({"threadId":1}));
        assert_eq!(
            runtime.command("device", command).unwrap()["status"],
            "accepted"
        );
        assert!(runtime.state.current().draft.text.is_empty());
        assert!(
            runtime.state.recovered_inputs[&1]["text"]
                .as_str()
                .unwrap()
                .contains("queued text")
        );
        let mut client = State::new(PathBuf::new(), false);
        client.client = Some(crate::client::Client::new(client.data_dir.clone()));
        client.apply_projection(runtime.state.projection()).unwrap();
        assert!(client.current().draft.text.is_empty());
    }

    #[test]
    fn headless_ticks_never_initialize_graphics_or_consume_device_drafts() {
        let (mut runtime, _commands) = wired();
        runtime.tick();
        assert!(!runtime.state.artwork.is_initialized());
        assert!(!runtime.state.portraits.is_initialized());
        assert!(!runtime.state.attachment_cache.is_initialized());
        runtime
            .state
            .current_mut()
            .draft
            .replace_text("not a projected editor");
        assert!(
            !runtime
                .state
                .projection()
                .to_string()
                .contains("not a projected editor")
        );
    }

    #[test]
    fn an_interrupted_intent_survives_restart_and_is_never_dispatched() {
        let (mut runtime, _commands) = wired();
        let command = Command::new(
            "thread.send",
            json!({"threadId":1,"text":"uncertain","attachments":[],"mode":"prompt"}),
        );
        runtime.state.record_command(
            Receipt {
                command_id: command.command_id,
                device_id: "device".into(),
                hash: command.hash(),
                status: crate::protocol::ReceiptStatus::Interrupted,
                result: json!({"uncertain":true}),
                seq: 1,
            },
            Some(command.clone()),
        );
        runtime.state.checkpoint_and_flush().unwrap();
        drop(runtime);
        let mut state = State::headless(crate::store::directory().unwrap());
        state.load_global_for_test();
        let mut resumed = RuntimeState::from_state(state);
        assert_eq!(
            resumed.command("device", command).unwrap()["status"],
            "interrupted"
        );
        assert!(resumed.state.current().pi.is_none());
        assert!(resumed.state.current().messages.is_empty());
    }

    #[test]
    fn switching_back_to_legacy_keeps_server_transcripts_and_receipts() {
        let (mut runtime, _commands) = wired();
        runtime.state.current_mut().messages.push(Message {
            role: "Pi",
            text: "durable server reply".into(),
            images: vec![],
        });
        runtime.state.checkpoint_and_flush().unwrap();
        drop(runtime);
        let mut legacy = State::new(PathBuf::new(), false);
        legacy.load_global_for_test();
        assert_eq!(legacy.current().messages[0].text, "durable server reply");
        legacy.checkpoint_and_flush().unwrap();
        let index = crate::store::load(&legacy.data_dir.join("index.json"))
            .unwrap()
            .unwrap();
        assert_eq!(
            index["threadViews"][0]["messages"][0]["text"],
            "durable server reply"
        );
    }

    #[test]
    fn legacy_mode_refuses_a_migrated_profile_without_starting_stores_or_harnesses() {
        let directory = crate::store::directory().unwrap();
        let database = crate::database::Database::open(&directory).unwrap();
        drop(database);
        let mut legacy = State::new(PathBuf::new(), false);
        legacy.load_global_for_test();
        assert!(
            legacy
                .storage_error
                .as_ref()
                .unwrap()
                .contains("--server-client")
        );
        assert!(legacy.store.is_none() && legacy.ui_store.is_none());
        assert!(!legacy.runtime_enabled && legacy.current().pi.is_none());
    }

    #[test]
    fn plain_migrated_desktop_launch_automatically_attaches_without_legacy_writes() {
        let directory = crate::store::directory().unwrap();
        drop(crate::database::Database::open(&directory).unwrap());
        // Prevent the client supervisor from spawning a test binary as a server.
        let _owner = crate::server_lock::Owner::acquire(&directory).unwrap();
        let legacy_index = b"legacy index must not be loaded or changed";
        std::fs::write(directory.join("index.json"), legacy_index).unwrap();
        let mut desktop = State::new(PathBuf::new(), false);
        assert!(desktop.client.is_none());
        desktop.setup(Arc::new(|| {}));
        assert!(desktop.client.is_some());
        assert!(desktop.store.is_none() && desktop.ui_store.is_some());
        assert!(!desktop.runtime_enabled && desktop.current().pi.is_none());
        assert!(desktop.storage_error.is_none());
        desktop.checkpoint_and_flush().unwrap();
        assert_eq!(
            std::fs::read(directory.join("index.json")).unwrap(),
            legacy_index
        );
        desktop.client.as_mut().unwrap().shutdown();
    }
    #[test]
    fn client_clocks_expire_local_undo_and_wake_snoozed_rows_without_a_server_tick() {
        let mut state = State::new(PathBuf::new(), false);
        state.client = Some(crate::client::Client::new(state.data_dir.clone()));
        state.current_mut().draft.replace_text("local");
        state.discard_draft(1);
        state.undo.as_mut().unwrap().expires = 0;
        let at = lifecycle::now();
        state.current_mut().life.snooze(at - 1000, at - 1);
        state.last_lifecycle_clock = at - 1000;
        assert!(state.poll_client(&mut Tx::default()));
        assert!(state.undo.is_none());
        assert!(state.current().pi.is_none());
    }
}
