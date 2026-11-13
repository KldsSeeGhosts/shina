//! State facade and plain projections. Execution still uses the original State/Thread methods.
use super::*;
use crate::protocol::{Command, Receipt};

// A fixed monotonic/wall-clock pair makes a start timestamp stable across captures.
fn stable_timestamp(at: Instant) -> u64 {
    static CLOCK: std::sync::OnceLock<(Instant, u64)> = std::sync::OnceLock::new();
    let (instant, unix) = CLOCK.get_or_init(|| (Instant::now(), crate::lifecycle::now()));
    if at >= *instant {
        unix.saturating_add(at.duration_since(*instant).as_millis() as u64)
    } else {
        unix.saturating_sub(instant.duration_since(at).as_millis() as u64)
    }
}
fn timestamp_instant(unix: u64) -> Option<Instant> {
    Instant::now().checked_sub(Duration::from_millis(
        crate::lifecycle::now().saturating_sub(unix),
    ))
}
fn thought_clock_key(thought: &Thought) -> String {
    format!(
        "{}:{}:{}",
        thought.anchor.message, thought.anchor.offset, thought.block
    )
}

#[cfg(test)]
#[test]
fn scoped_thought_windows_keep_their_own_start_clock() {
    let mut thread = Thread::new(1);
    for (message, secs) in [(0, 30), (10, 3)] {
        thread.thoughts.push(Thought {
            anchor: ToolAnchor { message, offset: 0 },
            block: 0,
            order: message as u64,
            text: String::new(),
            expanded: false,
            done: false,
            started: Some(Instant::now() - Duration::from_secs(secs)),
            duration_ms: None,
        });
    }
    let mut window = ThreadView::capture(&thread);
    window.thoughts.remove(0);
    let mut client = Thread::new(1);
    window.apply(&mut client).unwrap();
    let elapsed = client.thoughts[0].started.unwrap().elapsed();
    let expected = thread.thoughts[1].started.unwrap().elapsed();
    assert!(
        elapsed.as_millis().abs_diff(expected.as_millis()) < 100,
        "scoped clock {elapsed:?}, expected {expected:?}"
    );
}

#[cfg(test)]
#[test]
fn running_projection_timestamps_do_not_advance_with_presentation_time() {
    let mut thread = Thread::new(1);
    thread.status = Status::Running;
    thread.started = Some(Instant::now());
    thread.tools.push(Tool::new(
        "clock",
        "test",
        &json!({}),
        ToolAnchor {
            message: 0,
            offset: 0,
        },
    ));
    thread.tools[0].started = Some(Instant::now());
    let first = serde_json::to_value(ThreadView::capture(&thread)).unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(
        first,
        serde_json::to_value(ThreadView::capture(&thread)).unwrap()
    );
    let mut client = Thread::new(1);
    serde_json::from_value::<ThreadView>(first)
        .unwrap()
        .apply(&mut client)
        .unwrap();
    assert!(client.started.unwrap().elapsed() >= Duration::from_secs(1));
    assert!(client.tools[0].started.unwrap().elapsed() >= Duration::from_secs(1));
}
#[derive(serde::Serialize, serde::Deserialize)]
struct MessageView {
    #[serde(default = "new_message_id")]
    id: String,
    role: String,
    text: String,
    images: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<crate::composer_context::MessageContext>,
}
fn new_message_id() -> String {
    uuid::Uuid::new_v4().to_string()
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
    #[serde(default)]
    model_options: Vec<crate::model_selection::ProviderOptionSelection>,
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
    #[serde(default)]
    started_at: Option<u64>,
    #[serde(default)]
    tool_started_at: std::collections::BTreeMap<String, u64>,
    #[serde(default, skip_serializing)]
    thought_started_at: Vec<Option<u64>>,
    #[serde(default)]
    thought_clocks: std::collections::BTreeMap<String, u64>,
    queue: Vec<(usize, String, Vec<Attachment>)>,
    #[serde(default)]
    queue_drafts: Vec<(usize, crate::composer_rich::Snapshot)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    queue_contexts: Vec<(usize, crate::composer_context::MessageContext)>,
    #[serde(default)]
    queue_edit_supported: bool,
    #[serde(default)]
    composer_context_supported: bool,
    #[serde(default)]
    queue_held: bool,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    resumable_run_id: Option<String>,
    #[serde(default)]
    continuation_of_run_id: Option<String>,
    #[serde(default)]
    run_control_supported: bool,
    #[serde(default)]
    turn_capabilities: Option<TurnCapabilities>,
    steering: Vec<String>,
    follow_ups: Vec<String>,
    commands: Vec<Value>,
    #[serde(default)]
    skills: Vec<crate::composer_skills::Skill>,
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
    #[serde(default)]
    memory_suggestions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fresh_session_messages: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    queue_fresh: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_updated_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_cache_ttl_ms: Option<u64>,
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
                    id: m.id.clone(),
                    role: m.role.into(),
                    text: m.text.clone(),
                    images: m.images.clone(),
                    context: m.context.clone(),
                })
                .collect(),
            tools: thread
                .tools
                .iter()
                .map(|tool| {
                    let mut tool = tool.clone();
                    tool.started = None;
                    tool
                })
                .collect(),
            thoughts: thread
                .thoughts
                .iter()
                .map(|thought| {
                    let mut thought = thought.clone();
                    thought.started = None;
                    thought
                })
                .collect(),
            work: thread.work.clone(),
            harness_state: thread.harness_state.clone(),
            model: thread.model.clone(),
            model_provider: thread.model_provider.clone(),
            model_id: thread.model_id.clone(),
            thinking: thread.thinking.clone(),
            model_options: thread.model_options.clone(),
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
            elapsed: thread.elapsed,
            started_at: thread.started.map(stable_timestamp),
            tool_started_at: thread
                .tools
                .iter()
                .filter(|tool| !tool.done)
                .filter_map(|t| Some((t.id.clone(), stable_timestamp(t.started?))))
                .collect(),
            thought_started_at: Vec::new(),
            thought_clocks: thread
                .thoughts
                .iter()
                .filter(|thought| !thought.done)
                .filter_map(|t| Some((thought_clock_key(t), stable_timestamp(t.started?))))
                .collect(),
            queue: thread
                .native
                .queue
                .iter()
                .map(|q| (q.id, q.prompt.text.clone(), q.prompt.images.to_vec()))
                .collect(),
            queue_drafts: thread
                .native
                .queue
                .iter()
                .filter_map(|q| q.prompt.draft.clone().map(|draft| (q.id, draft)))
                .collect(),
            queue_contexts: thread
                .native
                .queue
                .iter()
                .filter_map(|q| q.prompt.context.clone().map(|ctx| (q.id, ctx)))
                .collect(),
            queue_edit_supported: true,
            composer_context_supported: true,
            queue_held: thread.native.queue_held,
            run_id: thread.run_id.clone(),
            resumable_run_id: thread.resumable_run_id.clone(),
            continuation_of_run_id: thread.continuation_of_run_id.clone(),
            run_control_supported: true,
            turn_capabilities: Some(thread.turn_capabilities()),
            steering: thread.native.steering.clone(),
            follow_ups: thread.native.follow_ups.clone(),
            commands: thread.native.commands.iter().map(|c| json!(c)).collect(),
            skills: thread.native.skills.clone(),
            compactions: thread.native.compactions.clone(),
            stats: thread.native.stats.clone(),
            fork_points: thread.native.fork_points.clone(),
            operating: thread.native.operating(),
            queued: thread.queued(),
            harness_choice: thread.can_choose_harness(),
            commands_loaded: thread.native.commands_loaded,
            commands_error: thread.native.commands_error.clone(),
            memory_suggestions: thread.memory_suggestions.clone(),
            fresh_session_messages: thread.fresh_session_messages.clone(),
            queue_fresh: thread
                .native
                .queue
                .iter()
                .filter(|q| thread.native.fresh_queue_ids.contains(&q.id))
                .map(|q| q.id)
                .collect(),
            context_updated_at: thread.context.updated_at,
            prompt_cache_ttl_ms: thread.context.prompt_cache_ttl_ms,
        }
    }

    fn apply(self, thread: &mut Thread) -> Result<(), String> {
        thread.memory_suggestions = self.memory_suggestions;
        thread.fresh_session_messages = self.fresh_session_messages;
        if thread.remote.is_some() {
            thread.request_generation = self.generation;
        }
        thread.status = self.status;
        thread.run_id = self.run_id;
        thread.resumable_run_id = self.resumable_run_id;
        thread.continuation_of_run_id = self.continuation_of_run_id;
        thread.run_control_supported = self.run_control_supported;
        let old_messages = std::mem::take(&mut thread.messages);
        thread.messages = self
            .messages
            .into_iter()
            .map(|message| {
                Ok(Message {
                    id: message.id,
                    role: match message.role.as_str() {
                        "You" => "You",
                        "Pi" => "Pi",
                        "Shina" => "Shina",
                        _ => return Err("Invalid message role.".into()),
                    },
                    text: message.text,
                    images: message.images,
                    context: message.context,
                })
            })
            .collect::<Result<_, String>>()?;
        let mut tools = self.tools;
        for tool in &mut tools {
            if !tool.done {
                tool.started = self
                    .tool_started_at
                    .get(&tool.id)
                    .copied()
                    .and_then(timestamp_instant)
                    .or_else(|| {
                        tool.duration_ms
                            .and_then(|ms| Instant::now().checked_sub(Duration::from_millis(ms)))
                    });
            }
            if let Some(old) = thread.tools.iter().find(|old| old.id == tool.id) {
                tool.expanded = old.expanded;
                tool.raw_expanded = old.raw_expanded;
                tool.group_expanded = old.group_expanded;
            }
        }
        thread.tools = tools;
        let mut thoughts = self.thoughts;
        for (index, thought) in thoughts.iter_mut().enumerate() {
            if !thought.done {
                thought.started = self
                    .thought_clocks
                    .get(&thought_clock_key(thought))
                    .copied()
                    .or_else(|| self.thought_started_at.get(index).copied().flatten())
                    .and_then(timestamp_instant)
                    .or_else(|| {
                        thought
                            .duration_ms
                            .and_then(|ms| Instant::now().checked_sub(Duration::from_millis(ms)))
                    });
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
            let message_id = thread.messages.get(item.start).map(|m| m.id.as_str());
            let matched = message_id
                .and_then(|id| {
                    thread
                        .work
                        .iter()
                        .find(|old| old_messages.get(old.start).map(|m| m.id.as_str()) == Some(id))
                })
                .or_else(|| thread.work.iter().find(|old| old.start == item.start));
            item.expanded = matched.is_some_and(|old| old.expanded);
        }
        thread.work = work;
        thread.harness_state = self.harness_state;
        if let Some(capabilities) = self.turn_capabilities {
            thread.harness_state["turnCapabilities"] = json!(capabilities);
        }
        thread.model = self.model;
        thread.model_provider = self.model_provider;
        thread.model_id = self.model_id;
        thread.thinking = self.thinking;
        thread.model_options = self.model_options;
        thread.models = self.models;
        thread.thinking_levels = self.thinking_levels;
        thread.supports_images = self.supports_images;
        thread.context_window = self.context_window;
        thread.context.usage = self.context_usage;
        thread.context.phase = self.context_phase;
        thread.context.updated_at = self.context_updated_at;
        thread.context.prompt_cache_ttl_ms = self.prompt_cache_ttl_ms;
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
            .started_at
            .and_then(timestamp_instant)
            .filter(|_| thread.status.busy())
            .or_else(|| {
                self.elapsed
                    .filter(|_| thread.status.busy())
                    .and_then(|secs| Instant::now().checked_sub(Duration::from_secs(secs)))
            });
        thread.native.queue = self
            .queue
            .into_iter()
            .map(|(id, text, images)| {
                let mut prompt = Prompt::new(text, images);
                prompt.draft = self
                    .queue_drafts
                    .iter()
                    .find(|(key, _)| *key == id)
                    .map(|(_, draft)| draft.clone());
                prompt.context = self
                    .queue_contexts
                    .iter()
                    .find(|(key, _)| *key == id)
                    .map(|(_, ctx)| ctx.clone());
                pi_session::QueuedMessage { id, prompt }
            })
            .collect();
        thread.queue_edit_supported = self.queue_edit_supported;
        thread.composer_context_supported = self.composer_context_supported;
        thread.native.queue_held = self.queue_held;
        thread.reconcile_queued_edit();
        thread.tick_stop(Instant::now());
        thread.native.steering = self.steering;
        thread.native.follow_ups = self.follow_ups;
        thread.native.commands = self
            .commands
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        thread.native.compactions = self.compactions;
        thread.native.skills = self.skills;
        thread.native.stats = self.stats;
        thread.native.fork_points = self.fork_points;
        thread.native.projected_operating = self.operating;
        thread.native.projected_queued = self.queued;
        thread.projected_harness_choice = self.harness_choice;
        thread.native.commands_loaded = self.commands_loaded;
        thread.native.commands_error = self.commands_error;
        thread.native.fresh_queue_ids = self.queue_fresh.into_iter().collect();
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
            id: uuid::Uuid::new_v4().to_string(),
            role: "Pi",
            text: "Retained archived reply".into(),
            images: vec![],
            context: None,
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
        for thread in &mut self.threads {
            if thread
                .peer_origin
                .as_ref()
                .is_some_and(|origin| origin.result.is_none())
            {
                thread.recovering_task = true;
            }
        }
        self.setup_headless_background();
        if let Some(error) = &self.background.routines.error {
            return Err(error.clone());
        }
        self.runtime_enabled = true;
        self.warm_repository_identities();
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
                // Application-owned queue records survive restart, held until a person resumes.
                // Legacy process-only queues still recover against history, never auto-dispatch.
                if thread.run_control_supported && !thread.native.queue.is_empty() {
                    thread.native.queue_held = true;
                    thread.serial = thread
                        .serial
                        .max(thread.native.queue.iter().map(|q| q.id).max().unwrap_or(0));
                } else {
                    thread.native.queue.clear();
                }
                thread.native.steering.clear();
                thread.native.follow_ups.clear();
                thread.native.questionnaire = None;
                if thread.status.busy() || thread.configuring || thread.request.is_some() {
                    thread.resumable_run_id = thread.run_id.clone();
                    thread.native.queue_held = true;
                    thread.status = Status::Failed;
                    for work in &mut thread.work {
                        if work.outcome == Some(Status::Running) {
                            work.outcome = Some(Status::Failed);
                        }
                    }
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
        }
        self.collect_recovered_inputs();
    }
    pub fn begin_runtime_shutdown(&mut self) {
        self.runtime_enabled = false;
        self.projects.clone_job.take();
        let remote_children: std::collections::HashSet<_> = self
            .tasks
            .iter()
            .filter(|task| task.remote.is_some())
            .map(|task| task.child_thread_id)
            .collect();
        for thread in &mut self.threads {
            if remote_children.contains(&thread.id) {
                continue;
            }
            if thread.status.busy()
                || thread.configuring
                || thread.queued()
                || thread.request.is_some()
            {
                let _ = thread.stop_task();
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
                let images: Vec<Attachment> =
                    serde_json::from_value(previous["attachments"].clone()).unwrap_or_default();
                let snapshot = serde_json::from_value::<crate::composer_rich::Snapshot>(
                    previous["draft"].clone(),
                )
                .unwrap_or_else(|_| {
                    crate::composer_rich::Snapshot::capture(
                        &crate::composer_rich::draft(previous["text"].as_str().unwrap_or("")),
                        &images,
                    )
                });
                thread.restore_snapshot(&snapshot);
            }
            let snapshot = crate::composer_rich::Snapshot::capture_with_context(
                &thread.draft,
                &thread.draft_images,
                &thread.draft_context,
            );
            self.recovered_inputs.insert(
                thread.id,
                json!({"text":thread.draft.text,"attachments":thread.draft_images,
                "draft":snapshot,"commandId":thread.recovery_command}),
            );
            thread.draft.replace_text("");
            thread.draft_images.clear();
            thread.draft_context.clear();
            thread.retained_context = Default::default();
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
        self.thread_views_scoped(None)
    }
    pub(super) fn thread_views_scoped(
        &self,
        scope: Option<&std::collections::HashSet<usize>>,
    ) -> Value {
        json!(
            self.threads
                .iter()
                .filter(|t| scope.is_none_or(|scope| scope.contains(&t.id)))
                .map(ThreadView::capture)
                .collect::<Vec<_>>()
        )
    }
    pub fn projection(&self) -> Value {
        self.projection_from_snapshot(self.snapshot())
    }
    pub(crate) fn projection_from_snapshot(&self, mut index: Value) -> Value {
        let views = index
            .as_object_mut()
            .unwrap()
            .remove("threadViews")
            .unwrap_or_else(|| self.thread_views());
        if index["_partial"] == true {
            index.as_object_mut().unwrap().remove("runtimeInputs");
            return json!({"_partial":true,"index":index,"threads":views});
        }
        for key in [
            "commandReceipts",
            "commandIntents",
            "threadViews",
            "serverSeq",
            "runtimeInputs",
        ] {
            index.as_object_mut().expect("index").remove(key);
        }
        let mut projection = json!({
            "index":index,"threads":views,"recoveryNotice":self.recovery_notice,
            "catalog":{"loading":self.catalog.loading(),"error":self.catalog.error},
            "nativeCatalogs":{"codex":self.codex_catalog.snapshot(),"claude":self.claude_catalog.snapshot()},
            "peers":self.peers,
            "directories":self.threads.iter().enumerate().filter_map(|(index,thread)| self.thread_directory(index).ok().map(|path| (thread.id.to_string(),json!(path)))).collect::<serde_json::Map<_,_>>(),
            "undo":self.undo,
            "harnessIds":self.harness_ids(),
            "clone":{"running":self.projects.cloning(),"progress":self.projects.clone_progress,"error":self.projects.clone_error},
        });
        projection
            .as_object_mut()
            .unwrap()
            .extend(self.document_projection().as_object().unwrap().clone());
        projection
    }

    pub(crate) fn document_projection(&self) -> Value {
        json!({
            "memories":(0..self.captains.len()).map(|id| (id.to_string(), json!(self.memories(id)))).collect::<serde_json::Map<_,_>>(),
            "profile":self.user_profile(),
            "routines":(0..self.captains.len()).map(|id| (id.to_string(), json!(self.routines(id)))).collect::<serde_json::Map<_,_>>(),
            "background":self.background_status(),
            "memoryBackend":self.memory_backend_status(),
            "keepAwake":self.background_settings().keep_awake
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
                    "thinking":t.selected_thinking,"access":t.native.access.id(),"life":t.life,
                    "serviceTier":t.harness_state["serviceTier"]
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
            "preferences",
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
            | self.refresh_machine_notice()
            | self.tick_composer_notices_at(Instant::now(), Some(tx));
        if self.background.quitting {
            return true;
        }
        let projections = self
            .client
            .as_ref()
            .map(|client| client.snapshots())
            .unwrap_or_default();
        let mut changed = changed || !projections.is_empty() || self.poll_context_imports();
        for (server, projection) in projections {
            if !self.machine_enabled(&server) {
                continue;
            }
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
                && command.method == "thread.send"
            {
                let accepted = value["result"]["status"] == "accepted";
                let mut bg_started: Option<(String, usize)> = None;
                let mut bg_failed: Option<(String, usize, String)> = None;
                let mut open_new_after: Option<(usize, String)> = None;
                let mut handled_background = false;
                let mut sync_fresh = false;
                let mut cleared_keep_id = None;
                {
                    let threads = if server == self.machines.selected {
                        Some(&mut self.threads)
                    } else {
                        self.machines.threads.get_mut(&server)
                    };
                    if let Some(thread) = threads.and_then(|threads| {
                        threads.iter_mut().find(|thread| {
                            thread
                                .remote_send
                                .as_ref()
                                .is_some_and(|(id, _)| *id == command.command_id)
                        })
                    }) {
                        thread.finish_remote_send(command.command_id, accepted);
                        if accepted {
                            cleared_keep_id = Some(thread.id);
                        }
                        sync_fresh = accepted && server == self.machines.selected;
                        let bg = thread.background_send.take();
                        let send_new = thread.pending_send_and_new_thread.take();
                        handled_background = bg.is_some() || send_new.is_some();
                        if !accepted {
                            let error = value["error"]["message"]
                                .as_str()
                                .or(value["result"]["result"]["error"].as_str())
                                .unwrap_or("Failed to send message.")
                                .to_string();
                            thread.error = Some(error.clone());
                            if let Some(bg_server) = bg {
                                bg_failed = Some((bg_server, thread.id, error));
                            }
                        } else if let Some(bg_server) = bg {
                            bg_started = Some((bg_server, thread.id));
                        } else if let Some((pending_cmd, pending_server)) = send_new
                            && pending_cmd == command.command_id
                        {
                            open_new_after = Some((thread.id, pending_server));
                        }
                    }
                }
                if let Some(thread_id) = cleared_keep_id {
                    self.set_keep_full_history(&server, thread_id, false);
                }
                if sync_fresh {
                    self.sync_fresh_session_notice();
                }
                if let Some((bg_server, thread_id)) = bg_started {
                    self.notify_background_started(&bg_server, thread_id);
                }
                if let Some((bg_server, thread_id, error)) = bg_failed {
                    self.notify_background_failure(&bg_server, thread_id, error);
                }
                if let Some((thread_id, pending_server)) = open_new_after
                    && self.machines.selected == pending_server
                    && self.current().id == thread_id
                {
                    let project = self.current().project;
                    let seed = self.composer_seed();
                    self.new_client_draft_with_seed(project, &seed);
                    if !self.headless {
                        self.restore_composer_focus(
                            crate::ui::composer_ui::FocusReason::ThreadOpened,
                        );
                    }
                    self.checkpoint_client();
                }
                if handled_background {
                    continue;
                }
            }
            if let crate::client::Event::Receipt(command, value) = &event
                && command.method == "thread.resume"
            {
                let threads = if server == self.machines.selected {
                    Some(&mut self.threads)
                } else {
                    self.machines.threads.get_mut(&server)
                };
                if let Some(thread) = threads.and_then(|threads| {
                    threads
                        .iter_mut()
                        .find(|thread| thread.resume_command == Some(command.command_id))
                }) {
                    thread.resume_command = None;
                    if value["result"]["status"] != "accepted" {
                        thread.error = Some("Could not resume thread.".into());
                    }
                }
            }
            if let crate::client::Event::Receipt(command, value) = &event
                && matches!(
                    command.method.as_str(),
                    "queue.update" | "queue.returnDraft"
                )
            {
                self.apply_queue_receipt(&server, command, value);
            }
            if let crate::client::Event::Receipt(command, value) = &event
                && (command.method.starts_with("settings.")
                    || matches!(
                        command.method.as_str(),
                        "profile.put" | "background.keepAwake"
                    ))
                && server == self.settings.machine
            {
                self.settings.notice = if value["status"] == "accepted" {
                    None
                } else {
                    Some(
                        value["result"]["error"]
                            .as_str()
                            .unwrap_or("Setting was not saved.")
                            .into(),
                    )
                };
                continue;
            }
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
                crate::client::Event::Rpc { tag, result } => {
                    if let Some(req_id) = tag
                        .strip_prefix("find:")
                        .and_then(|s| s.parse::<u64>().ok())
                        && server == self.thread_find.server
                    {
                        let parsed = result.and_then(|val| {
                            serde_json::from_value::<crate::thread_find::ThreadFindResult>(val)
                                .map_err(|e| e.to_string())
                        });
                        self.apply_thread_find_result(req_id, parsed);
                    } else if let Some(req_id) = tag
                        .strip_prefix("write-plan:")
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        let parsed = result.and_then(|val| {
                            serde_json::from_value::<crate::protocol::ProjectWriteFileResult>(val)
                                .map_err(|e| e.to_string())
                        });
                        self.apply_plan_save_result(req_id, parsed);
                    } else if let Some(rest) = tag.strip_prefix("search:")
                        && let Some((seq_str, target_srv)) = rest.split_once(':')
                        && let Ok(seq) = seq_str.parse::<u64>()
                    {
                        let parsed = result.and_then(|val| {
                            serde_json::from_value::<Vec<crate::thread_search::ThreadSearchHit>>(
                                val["matches"].clone(),
                            )
                            .map_err(|e| e.to_string())
                        });
                        self.apply_sidebar_search_result(seq, target_srv, parsed);
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
                    self.apply_tab_receipt(&server, &command, &value);
                    if command.method.starts_with("server.") {
                        if let Some(error) = value["error"]["message"].as_str() {
                            self.restart_notice = Some(error.into());
                        }
                    } else if value["result"]["status"] == "accepted" {
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
                        if matches!(
                            command.method.as_str(),
                            "thread.stop" | "thread.rewind" | "queue.returnDraft"
                        ) && let Some(id) = command.params["threadId"].as_u64()
                        {
                            self.client_recover.insert(id as usize, command.command_id);
                        }
                        if command.method == "input.claim"
                            && let Some(thread) = self
                                .threads
                                .iter_mut()
                                .find(|t| json!(t.id) == command.params["threadId"])
                        {
                            let images: Vec<Attachment> = serde_json::from_value(
                                value["result"]["result"]["attachments"].clone(),
                            )
                            .unwrap_or_default();
                            if let Ok(snapshot) =
                                serde_json::from_value::<crate::composer_rich::Snapshot>(
                                    value["result"]["result"]["draft"].clone(),
                                )
                            {
                                thread.restore_snapshot(&snapshot);
                            } else {
                                thread.restore_prompt(&Prompt::new(
                                    value["result"]["result"]["text"]
                                        .as_str()
                                        .unwrap_or("")
                                        .into(),
                                    images,
                                ));
                            }
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
                                if let Some((id, _)) = &thread.remote_send {
                                    thread.finish_remote_send(*id, false);
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
                Ok(crate::peer::MachineResult::Peers) => {
                    self.machines.notice = Some("Peer links updated.".into())
                }
                Err(error) => self.machines.notice = Some(error),
            }
        }
        self.finish_client_promotions();
        self.finish_project_selection();
        self.open_requested_tab();
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
                self.deleted_threads.clear();
                self.deleted_records = None;
                changed = true;
            } else {
                tx.wake_after(Duration::from_millis(notice.expires - at));
            }
        }
        changed |= self.expire_toasts();
        for toast in &self.toasts {
            if let Some(expires) = toast.expires_at
                && expires > at
            {
                tx.wake_after(Duration::from_millis(expires - at));
            }
        }
        if let Some(wait) = crate::ui::tick_portraits(self, Instant::now()) {
            tx.wake_after(wait);
        }
        if changed {
            self.checkpoint_client();
        }
        // Fleet decisions must see this poll's projections, and precede the local
        // restart check. Otherwise a final remote outcome can wait for an unrelated wake.
        changed |= self.poll_fleet();
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
        let preserve_cached_editors = self.client_views["cachedEditors"] == true;
        let active = self.current().id;
        let tabs = self.tabs.clone();
        let first = self.client_views["index"].is_null();
        let restore_navigation = first && (self.tab_epoch == 0 || tabs.is_empty());
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
        // Validate server records without session restoration's navigation or draft creation.
        self.restore_records(&projection["index"], false)?;
        self.peers = serde_json::from_value(projection["peers"].clone()).unwrap_or_default();
        let metadata = std::mem::take(&mut self.threads);
        for mut thread in metadata {
            if let Some(index) = old.iter().position(|old| old.id == thread.id) {
                let mut local = old.swap_remove(index);
                local.title = thread.title;
                local.renamed = thread.renamed;
                local.life = thread.life;
                local.session = thread.session;
                local.never_sent = thread.never_sent;
                local.harness = thread.harness;
                local.harness_context_sent = thread.harness_context_sent;
                local.project = thread.project;
                local.machine = thread.machine;
                local.updated = thread.updated;
                local.captain = thread.captain;
                local.captain_home = thread.captain_home;
                local.delegated = thread.delegated;
                local.parent = thread.parent;
                local.peer_origin = thread.peer_origin;
                local.stop_serial = thread.stop_serial;
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
        // Keep device drafts and open tabs, even after another device deletes a server record.
        self.threads.extend(
            old.into_iter()
                .filter(|t| t.client_draft || tabs.contains(&t.id)),
        );
        if first
            && let Some(drafts) = self.client_views["localUi"]["clientDrafts"]
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
                thread.harness_state["serviceTier"] = draft["serviceTier"].clone();
                thread.native.access = draft["access"]
                    .as_str()
                    .map_or(AccessMode::default(), |access| {
                        AccessMode::parse(access).unwrap_or(AccessMode::Supervised)
                    });
                if let Some(life) = draft.get("life") {
                    thread.life = serde_json::from_value(life.clone())
                        .map_err(|_| "Invalid saved client draft lifecycle.")?;
                }
                self.threads.push(thread);
            }
        }
        let views: Vec<ThreadView> =
            serde_json::from_value(projection["threads"].clone()).map_err(|e| e.to_string())?;
        for (view, raw) in views
            .into_iter()
            .zip(projection["threads"].as_array().into_iter().flatten())
        {
            if let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == view.id) {
                view.apply(thread)?;
                if let Some(meta) = raw.get("historyMeta").filter(|m| m.is_object()) {
                    thread.history_has_more = meta["hasMoreHistory"] == true;
                    thread.history_loading = meta["loading"] == true;
                    thread.history_error = meta["error"].as_str().map(str::to_owned);
                    thread.deferred_work =
                        meta["deferredWork"].as_array().cloned().unwrap_or_default();
                    thread.deferred_work_supported = meta["workLoading"] == "deferred";
                }
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
        self.dark = dark;
        self.zoom = zoom;
        self.project_scope = scope;
        self.scope_additional = additional;
        self.recovery_notice = projection["recoveryNotice"].as_str().map(str::to_owned);
        if !self
            .undo
            .as_ref()
            .is_some_and(|notice| notice.expires > lifecycle::now() && self.client_undo_is_local())
        {
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
        self.apply_native_catalogs();
        if first && !local.is_null() {
            // Live cached editors also own pending sends and isolated queue edits. Restoring the
            // disk UI over them would lose receipts or replace an edit with its ordinary draft.
            let editors: Vec<_> = if preserve_cached_editors {
                self.threads
                    .iter()
                    .map(|thread| {
                        (
                            thread.id,
                            thread.draft.clone(),
                            thread.draft_images.clone(),
                            thread.draft_context.clone(),
                            thread.retained_context.clone(),
                            thread.attachment_error.clone(),
                        )
                    })
                    .collect()
            } else {
                vec![]
            };
            self.restore_ui(&local)?;
            for (id, draft, images, context, retained, error) in editors {
                if let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == id) {
                    thread.draft = draft;
                    thread.draft_images = images;
                    thread.draft_context = context;
                    thread.retained_context = retained;
                    thread.attachment_error = error;
                }
            }
            if restore_navigation && let Some(tabs) = local["tabs"].as_array() {
                self.tabs = tabs
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|id| id as usize)
                    .filter(|id| self.threads.iter().any(|t| t.id == *id))
                    .collect();
            }
            if restore_navigation
                && let Some(index) = self
                    .threads
                    .iter()
                    .position(|thread| json!(thread.id) == local["active"])
            {
                self.active = index;
            }
        }
        if self.threads.is_empty() {
            self.new_client_draft(None);
        } else if first {
            let id = self.current().id;
            insert_tab(&mut self.tabs, id, id);
        }
        self.client_recover
            .retain(|id, _| self.threads.iter().any(|thread| thread.id == *id));
        if !self.demo
            && self.supports_model_favorites()
            && !self.model_favorites_migrated
            && self.model_favorites.is_none()
            && !self.preferences.model_favorites.is_empty()
        {
            self.model_favorites_migrated = true;
            self.forward(
                "settings.modelFavorites",
                json!({"migrateFavorites": self.preferences.model_favorites}),
            );
        }
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

    pub fn load_earlier_history(&mut self, thread_id: usize, through_entry_id: Option<String>) {
        let Some(thread) = self.threads.iter_mut().find(|t| t.id == thread_id) else {
            return;
        };
        if !thread.history_has_more || thread.history_loading {
            return;
        }
        thread.history_loading = true;
        thread.history_error = None;
        if let Some(client) = &self.client {
            if self.machines.selected == "local" {
                client.load_earlier_history(thread_id, through_entry_id);
            } else if let Some(linked) = client.linked.get(&self.machines.selected) {
                linked.load_earlier_history(thread_id, through_entry_id);
            }
        }
    }
    pub fn load_deferred_work(&mut self, start: usize, retry: bool) {
        let Some(client) = &self.client else { return };
        let target = if self.machines.selected == "local" {
            Some(client)
        } else {
            client.linked.get(&self.machines.selected)
        };
        let Some(target) = target else { return };
        if !target.has_capability(crate::protocol::DEFERRED_WORK) {
            return;
        }
        let thread = &mut self.threads[self.active];
        for marker in &mut thread.deferred_work {
            if marker["start"] != start
                || marker["loading"] == true
                || (!retry && !marker["error"].is_null())
            {
                continue;
            }
            if let Some(anchor) = marker["sourceItemId"].as_str().map(str::to_owned) {
                marker["loading"] = json!(true);
                marker["error"] = Value::Null;
                target.load_work_history(thread.id, anchor);
            }
        }
    }
    pub fn machine_label(&self, server: &str) -> String {
        if server == "local"
            || self
                .machines
                .views
                .get("local")
                .and_then(|v| v["serverId"].as_str())
                == Some(server)
        {
            "Local".into()
        } else {
            self.client
                .as_ref()
                .and_then(|client| client.linked.get(server))
                .and_then(|client| client.remote.as_ref())
                .map(|link| link.host.clone())
                .or_else(|| {
                    self.peers
                        .iter()
                        .find(|p| p.server_id == server)
                        .map(|p| p.label.clone())
                })
                .or_else(|| {
                    self.threads
                        .iter()
                        .filter_map(|t| t.peer_origin.as_ref())
                        .find(|o| o.server_id == server)
                        .map(|o| o.server_label.clone())
                })
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
    /// Whether a machine is enabled (local is always enabled; linked machines can be
    /// disconnected and re-enabled in settings or on retry).
    pub fn machine_enabled(&self, server: &str) -> bool {
        server == "local"
            || self
                .client
                .as_ref()
                .and_then(|client| client.linked.get(server))
                .is_none_or(crate::client::Client::enabled)
    }
    /// The names of the linked machines whose connection is not ready.
    pub fn unreachable_machines(&self) -> Vec<String> {
        let Some(client) = &self.client else {
            return Vec::new();
        };
        client
            .linked
            .keys()
            .filter(|server| self.machine_enabled(server) && !self.machine_connected(server))
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
    pub(crate) fn tick_composer_notices_at(&mut self, now: Instant, tx: Option<&mut Tx>) -> bool {
        let active = if self.demo || self.client.is_none() {
            None
        } else {
            let server = &self.machines.selected;
            let has_view = self.machines.views.contains_key(server);
            let connected = self.machine_connected(server);
            if !connected || !has_view {
                let connection = self
                    .client
                    .as_ref()
                    .and_then(|client| client.connection_state_for(server))
                    .unwrap_or_default();
                Some(crate::composer_notices::EnvironmentStatus {
                    server_id: server.clone(),
                    label: self.machine_label(server),
                    connection,
                    can_disconnect: server != "local",
                })
            } else {
                None
            }
        };
        let (changed, wake) = self.composer_notices.sync_environment(active, now);
        if let (Some(tx), Some(delay)) = (tx, wake) {
            tx.wake_after(delay);
        }
        changed | self.sync_fresh_session_notice()
    }
    pub fn retry_machine(&mut self, server: &str) {
        let result = if server == "local" {
            self.client
                .as_ref()
                .ok_or_else(|| "This machine is not linked.".to_owned())
                .and_then(|client| client.retry_now())
        } else if !self.machine_enabled(server) {
            self.set_machine_enabled(server, true)
        } else {
            self.client
                .as_ref()
                .and_then(|client| client.linked.get(server))
                .ok_or_else(|| "This machine is not linked.".to_owned())
                .and_then(|remote| remote.retry_now())
        };
        if let Err(error) = result {
            self.push_toast(
                ToastKind::Error,
                "Could not reconnect environment",
                Some(error),
                None,
                None,
            );
        }
        self.tick_composer_notices_at(Instant::now(), None);
    }
    pub fn set_machine_enabled(&mut self, server: &str, enabled: bool) -> Result<(), String> {
        if server == "local" {
            return Ok(());
        }
        let wake = self.wake.clone();
        let remote = self
            .client
            .as_mut()
            .and_then(|client| client.linked.get_mut(server))
            .ok_or_else(|| "This machine is not linked.".to_owned())?;
        remote.set_enabled(enabled, wake)?;
        if !enabled {
            self.machines.views.remove(server);
            self.machines.threads.remove(server);
            self.machines.recover.remove(server);
            self.refresh_machine_notice();
            if self.machines.selected == server {
                self.select_machine("local");
                self.new_client_draft(None);
            }
        }
        self.tick_composer_notices_at(Instant::now(), None);
        Ok(())
    }
    pub fn disconnect_server(&mut self, server: &str) {
        if server == "local" {
            return;
        }
        if let Err(error) = self.set_machine_enabled(server, false) {
            self.push_toast(
                ToastKind::Error,
                "Could not disconnect server",
                Some(error),
                None,
                None,
            );
        }
    }
    pub fn toggle_machine_enabled(&mut self, server: &str) {
        let next = !self.machine_enabled(server);
        if let Err(error) = self.set_machine_enabled(server, next) {
            self.push_toast(
                ToastKind::Error,
                if next {
                    "Could not reconnect environment"
                } else {
                    "Could not disconnect server"
                },
                Some(error),
                None,
                None,
            );
        }
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
        self.tab_epoch += 1;
        self.requested_tab = None;
        self.requested_tab_restore = None;
        self.close_thread_find();
        self.machines.selected = server.into();
        let cached = self.machines.threads.remove(server);
        let cached_editors = cached.is_some();
        self.threads = cached.unwrap_or_else(|| vec![Thread::new(1)]);
        self.active = 0;
        self.tabs.clear();
        self.client_recover = self.machines.recover.remove(server).unwrap_or_default();
        self.selected_threads.clear();
        self.project_scope = ProjectScope::All;
        self.scope_additional.clear();
        self.project_picker = None;
        self.captain_editor = None;
        self.row_menu = None;
        self.undo = None;
        self.deleted_threads.clear();
        self.deleted_records = None;
        // Theme, zoom and the sidebar's layout belong to this device, not to the machine. A
        // machine's saved UI only brings back its drafts, tabs, open thread and workspace filter.
        let device = (
            self.dark,
            self.zoom,
            self.sidebar,
            self.sidebar_width,
            self.shelves,
            self.background.settings.clone(),
            self.preferences.clone(),
        );
        self.client_views =
            json!({"localUi":self.machines.ui[server],"cachedEditors":cached_editors});
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
            self.captain_remote_scopes.clear();
            self.captain_homes.clear();
            self.tasks.clear();
        }
        let restored_tabs: Vec<_> = self.machines.ui[server]["tabs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|id| id as usize)
            .filter(|id| self.threads.iter().any(|t| t.id == *id))
            .collect();
        if restored_tabs.is_empty() {
            self.tabs.clear();
            self.new_client_draft(None);
        } else {
            self.tabs = restored_tabs;
            let active = self.machines.ui[server]["active"]
                .as_u64()
                .map(|id| id as usize)
                .filter(|id| self.tabs.contains(id))
                .unwrap_or(self.tabs[0]);
            self.active = self
                .threads
                .iter()
                .position(|t| t.id == active)
                .expect("restored active tab");
            self.capture_opened_draft();
        }
        (
            self.dark,
            self.zoom,
            self.sidebar,
            self.sidebar_width,
            self.shelves,
            self.background.settings,
            self.preferences,
        ) = device;
        (
            self.project_scope,
            self.scope_additional,
            self.machines.scope_server,
            self.machines.scope_remote,
        ) = filter;
        self.tick_composer_notices_at(Instant::now(), None);
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
        let context = self.current().draft_context.clone();
        let retained = self.current().retained_context.clone();
        self.select_machine(server);
        if self.machines.selected != server {
            return;
        }
        self.new_client_draft(None);
        self.current_mut().draft = draft;
        self.current_mut().draft_images = images;
        self.current_mut().draft_context = context;
        self.current_mut().retained_context = retained;
        if let Some(thread) = self
            .machines
            .threads
            .get_mut(&source.0)
            .and_then(|threads| threads.iter_mut().find(|thread| thread.id == source.1))
        {
            thread.draft.replace_text("");
            thread.draft_images.clear();
            thread.draft_context.clear();
            thread.retained_context = Default::default();
        }
        // Update the old machine's saved editor too, so a later switch cannot restore a copy.
        self.machines.ui[&source.0]["drafts"][source.1.to_string()] = json!("");
        self.open_picker(ProjectPicker::NewThread);
        self.machines.draft_picker = Some(server.into());
        self.checkpoint();
    }

    pub(super) fn new_client_draft(&mut self, project: Option<usize>) {
        let seed = self.composer_seed();
        self.new_client_draft_with_seed(project, &seed);
    }

    pub(super) fn new_client_draft_with_seed(&mut self, project: Option<usize>, seed: &Value) {
        if project.is_some() && self.project(project).is_none() {
            return;
        }
        let reuse = self.threads.iter().position(|thread| {
            thread.client_draft
                && thread.project == project
                && thread.blank_draft()
                && thread.client_creating.is_none()
                && thread.client_promoted.is_none()
        });
        let index = reuse.unwrap_or_else(|| {
            let id = self
                .threads
                .iter()
                .chain(self.deleted_threads.iter())
                .filter(|t| t.client_draft)
                .map(|t| t.id)
                .min()
                .unwrap_or(usize::MAX)
                .min(self.next_client_draft)
                .saturating_sub(1);
            self.next_client_draft = id;
            let mut thread = Thread::new(id);
            thread.client_draft = true;
            thread.harness = self.default_harness.clone();
            thread.project = project;
            self.threads.push(thread);
            self.threads.len() - 1
        });
        self.project_picker = None;
        self.show_archive = false;
        self.apply_catalog();
        if reuse.is_none() || self.threads[index].selected_model.is_none() {
            self.inherit_composer(index, seed);
        }
        self.apply_native_catalogs();
        self.select_thread(index);
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
            "occupiedDrafts":self.threads.iter().filter(|t| !t.client_draft && (t.has_draft_content() || self.tabs.contains(&t.id))).map(|t| t.id).collect::<Vec<_>>(),
            "harness":thread.harness,"selectedModel":thread.selected_model,"thinking":thread.selected_thinking,"access":thread.native.access.id(),
            "serviceTier":thread.harness_state["serviceTier"]});
        match self.remote("thread.create", params).expect("client") {
            Ok(command) => {
                let snapshot = crate::composer_rich::Snapshot::capture_with_context(
                    &self.current().draft,
                    &self.current().draft_images,
                    &self.current().draft_context,
                );
                let thread = self.current_mut();
                thread.client_creating = Some(command);
                thread.remote_send = Some((command, snapshot));
                thread.life.last_requested_at = Some(lifecycle::now());
                thread.draft.replace_text("");
                thread.draft_images.clear();
                thread.draft_context.clear();
                thread.retained_context = Default::default();
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
        let target_server_id = self.machine_identity(server);
        let target_peers: Vec<crate::peer::Peer> = if server == self.machines.selected {
            self.peers.clone()
        } else {
            self.machines
                .views
                .get(server)
                .and_then(|v| serde_json::from_value(v["peers"].clone()).ok())
                .unwrap_or_default()
        };
        let peer_labels: Vec<(String, String)> = target_peers
            .iter()
            .map(|p| (p.server_id.clone(), p.label.clone()))
            .collect();
        let linked_hosts: Vec<(String, String)> = self
            .client
            .as_ref()
            .map(|client| {
                client
                    .linked
                    .iter()
                    .filter_map(|(k, c)| c.remote.as_ref().map(|l| (k.clone(), l.host.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let peer_label = |machine: &str| -> String {
            if machine == "local" {
                "Local".into()
            } else {
                linked_hosts
                    .iter()
                    .find(|(k, _)| k == machine)
                    .map(|(_, h)| h.clone())
                    .or_else(|| {
                        peer_labels
                            .iter()
                            .find(|(id, _)| id == machine)
                            .map(|(_, l)| l.clone())
                    })
                    .unwrap_or_else(|| machine.into())
            }
        };
        enum Outcome {
            None,
            Failed(Option<String>, usize, String),
        }
        let outcome = {
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
                if let Some((_, snapshot)) = submitted {
                    draft.restore_snapshot(&snapshot);
                }
                let error = value["error"]["message"]
                    .as_str()
                    .or(value["result"]["result"]["error"].as_str())
                    .unwrap_or("Thread creation was not confirmed. Your draft was kept.")
                    .to_string();
                draft.error = Some(error.clone());
                Outcome::Failed(draft.background_send.take(), draft.id, error)
            } else if let Some(id) = value["result"]["result"]["threadId"].as_u64() {
                draft.client_promoted = Some(id as usize);
                if let Some((_, snapshot)) = submitted {
                    match crate::composer_tokens::prompt_for_target(
                        &snapshot.editor(),
                        &snapshot.attachments,
                        &snapshot.context,
                        Some(server),
                        Some(&target_server_id),
                        &target_peers,
                        &peer_label,
                    ) {
                        Ok(prompt) => {
                            let params = json!({"threadId":id,"text":prompt.text,"attachments":prompt.images.as_ref(),
                                "context":prompt.context,"draft":prompt.draft,"editorRevision":uuid::Uuid::new_v4(),"mode":"prompt"});
                            let result = commands
                                .ok_or_else(|| {
                                    "This machine is not linked. Your draft was kept.".to_string()
                                })
                                .and_then(|commands| commands.send("thread.send", params));
                            match result {
                                Ok(cmd_id) => {
                                    draft.remote_send = Some((cmd_id, snapshot));
                                    Outcome::None
                                }
                                Err(error) => {
                                    draft.restore_snapshot(&snapshot);
                                    draft.error = Some(error.clone());
                                    Outcome::Failed(draft.background_send.take(), draft.id, error)
                                }
                            }
                        }
                        Err(error) => {
                            draft.restore_snapshot(&snapshot);
                            draft.attachment_error = Some(error.clone());
                            Outcome::Failed(draft.background_send.take(), draft.id, error)
                        }
                    }
                } else {
                    Outcome::None
                }
            } else {
                Outcome::None
            }
        };
        if let Outcome::Failed(bg, draft_id, error) = outcome {
            if let Some(bg_server) = bg {
                self.notify_background_failure(&bg_server, draft_id, error);
            } else {
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
            let context = std::mem::take(&mut self.threads[draft_index].draft_context);
            let retained = std::mem::take(&mut self.threads[draft_index].retained_context);
            let sending = self.threads[draft_index].remote_send.take();
            let background_send = self.threads[draft_index].background_send.take();
            self.threads[index].draft = text;
            self.threads[index].draft_images = images;
            self.threads[index].draft_context = context;
            self.threads[index].retained_context = retained;
            self.threads[index].remote_send = sending;
            if background_send.is_some() {
                self.threads[index].background_send = background_send;
            }
            let active = self.current().id;
            if self.tabs.contains(&draft_id) {
                self.tabs.retain(|tab| *tab != id);
            }
            for tab in &mut self.tabs {
                if *tab == draft_id {
                    *tab = id;
                }
            }
            self.threads.remove(draft_index);
            self.active = self
                .threads
                .iter()
                .position(|t| t.id == if active == draft_id { id } else { active })
                .expect("active promotion thread");
            if active == draft_id {
                self.focus_thread(self.active);
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
        if self.demo {
            self.machines.notice =
                Some("Demo machines never pair, revoke devices or request updates.".into());
            return;
        }
        if self.client.is_none() {
            self.machines.notice = Some(
                "Linking requires server-client mode. Start Shina with --server-client.".into(),
            );
            return;
        }
        if self.machines.pending.is_some() {
            return;
        }
        let profile = self.data_dir.clone();
        let peer_host = self.machines.peer_host.text.clone();
        let peer_code = self.machines.peer_code.text.clone();
        self.machines.peer_code.replace_text("");
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
                            "peer-link" => crate::client::local_rpc(
                                &profile,
                                "peer.link",
                                json!({"host":peer_host,"code":peer_code}),
                            )
                            .await
                            .map(|_| MachineResult::Peers),
                            "peer-unlink" => crate::client::local_rpc(
                                &profile,
                                "peer.unlink",
                                json!({"server":target}),
                            )
                            .await
                            .map(|_| MachineResult::Peers),
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
                            "revoke" | "grant-update" | "revoke-update" => {
                                let receipt = crate::client::local_rpc(
                                    &profile,
                                    match action.as_str() {
                                        "grant-update" => "devices.grant",
                                        "revoke-update" => "devices.revokeGrant",
                                        _ => "devices.revoke",
                                    },
                                    json!({"deviceId":target,"grant":"update"}),
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
        if self.client.is_some()
            && matches!(method, "thread.lifecycle" | "thread.drop" | "thread.rename")
            && self
                .threads
                .iter()
                .any(|t| t.client_draft && json!(t.id) == params["threadId"])
        {
            return false;
        }
        if self.client.is_some() && self.current().client_draft {
            match method {
                "thread.configure" => return false,
                "thread.visit" => return true,
                _ => {}
            }
        }
        let intent = match method {
            "thread.create" | "thread.fork" | "captain.open" | "captain.put" => {
                Some(TabIntent::Open(None))
            }
            "thread.delete" => params["threadId"].as_u64().map(|id| {
                let roots: Vec<usize> = serde_json::from_value(params["threadIds"].clone())
                    .unwrap_or_else(|_| vec![id as usize]);
                TabIntent::Delete(
                    roots
                        .into_iter()
                        .flat_map(|id| self.deletion_ids(id))
                        .collect(),
                )
            }),
            "thread.lifecycle" => params["threadId"].as_u64().and_then(|id| {
                let thread = self.threads.iter().find(|t| t.id == id as usize)?;
                match params["action"].as_str() {
                    Some("settle") if !thread.life.settled() => {
                        Some(TabIntent::Park(thread.id, true))
                    }
                    Some("archive") if !thread.archived() => {
                        Some(TabIntent::Park(thread.id, false))
                    }
                    Some("snooze") => Some(TabIntent::Park(thread.id, true)),
                    Some("discard") => Some(TabIntent::Park(thread.id, false)),
                    _ => None,
                }
            }),
            "thread.drop" if params["to"] == "Settled" => {
                params["threadId"].as_u64().and_then(|id| {
                    self.threads
                        .iter()
                        .find(|t| t.id == id as usize && !t.life.settled())
                        .map(|t| TabIntent::Park(t.id, true))
                })
            }
            "thread.undo" => {
                let entries = self.undo.as_ref().map_or(&[][..], |n| &n.entries);
                Some(TabIntent::Undo(
                    entries
                        .iter()
                        .filter_map(|e| {
                            self.undo_tabs
                                .get(&(self.machines.selected.clone(), e.id))
                                .map(|(p, _)| (e.id, *p))
                        })
                        .collect(),
                    entries.iter().find_map(|e| {
                        self.undo_tabs
                            .get(&(self.machines.selected.clone(), e.id))
                            .and_then(|(_, active)| active.then_some(e.id))
                    }),
                ))
            }
            _ => None,
        };
        if let Some(intent) = intent
            && let Some(result) = self.request_tab_command(method, params.clone(), intent)
        {
            if let Err(error) = result {
                self.storage_error = Some(error);
            }
            return true;
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
    pub(super) fn request_tab_command(
        &mut self,
        method: &str,
        params: Value,
        intent: TabIntent,
    ) -> Option<Result<uuid::Uuid, String>> {
        let result = self.remote(method, params)?;
        if let Ok(id) = result {
            if matches!(&intent, TabIntent::Open(_) | TabIntent::Undo(..))
                || matches!(&intent, TabIntent::Park(thread, _) if *thread == self.current().id)
                || matches!(&intent, TabIntent::Delete(ids) if ids.contains(&self.current().id))
            {
                self.tab_epoch += 1;
                self.requested_tab = None;
            }
            if let TabIntent::Park(thread, _) = &intent
                && let Some(position) = self.tabs.iter().position(|id| id == thread)
            {
                self.undo_tabs.insert(
                    (self.machines.selected.clone(), *thread),
                    (position, self.current().id == *thread),
                );
            }
            if let TabIntent::Delete(ids) = &intent {
                for (position, thread) in self.tabs.iter().enumerate() {
                    if ids.contains(thread) {
                        self.undo_tabs.insert(
                            (self.machines.selected.clone(), *thread),
                            (position, self.current().id == *thread),
                        );
                    }
                }
            }
            self.tab_intents.insert(
                (self.machines.selected.clone(), id),
                (self.tab_epoch, intent),
            );
        }
        Some(result)
    }
    fn apply_tab_receipt(&mut self, server: &str, command: &Command, value: &Value) {
        let Some((epoch, intent)) = self
            .tab_intents
            .remove(&(server.into(), command.command_id))
        else {
            return;
        };
        if value["result"]["status"] != "accepted" {
            return;
        }
        match intent {
            TabIntent::Open(replace) if epoch == self.tab_epoch => {
                if let Some(id) = value["result"]["result"]["threadId"].as_u64() {
                    self.requested_tab = Some((server.into(), id as usize, epoch, replace));
                }
            }
            TabIntent::Park(id, settle) if epoch == self.tab_epoch || self.current().id != id => {
                self.remove_tab(id, settle)
            }
            TabIntent::Delete(ids) => {
                let active = self.current().id;
                self.tabs.retain(|id| *id == active || !ids.contains(id));
                if ids.contains(&active) && epoch == self.tab_epoch {
                    self.remove_tab(active, false);
                }
            }
            TabIntent::Undo(tabs, focus) if epoch == self.tab_epoch => {
                self.requested_tab_restore = Some(RequestedTabRestore {
                    server: server.into(),
                    epoch,
                    tabs,
                    focus,
                });
                self.restore_requested_tabs();
            }
            _ => {}
        }
    }
    fn open_requested_tab(&mut self) {
        self.restore_requested_tabs();
        let Some((server, id, epoch, replace)) = self.requested_tab.clone() else {
            return;
        };
        if server != self.machines.selected || epoch != self.tab_epoch {
            self.requested_tab = None;
            return;
        }
        if let Some(index) = self.threads.iter().position(|t| t.id == id) {
            if let Some(old) = replace.filter(|old| *old != id && self.tabs.contains(old)) {
                self.tabs.retain(|tab| *tab != id);
                if let Some(position) = self.tabs.iter().position(|tab| *tab == old) {
                    self.tabs[position] = id;
                }
            }
            self.select_thread(index);
        }
    }
    fn restore_requested_tabs(&mut self) {
        let Some(request) = &self.requested_tab_restore else {
            return;
        };
        if request.server != self.machines.selected || request.epoch != self.tab_epoch {
            self.requested_tab_restore = None;
            return;
        }
        if request
            .tabs
            .iter()
            .all(|(id, _)| self.threads.iter().any(|t| t.id == *id))
        {
            let request = self
                .requested_tab_restore
                .take()
                .expect("pending tab restore");
            self.restore_tabs(request.tabs, request.focus);
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
        if self.remote_task(self.current().id).is_some()
            && command.method.starts_with("thread.")
            && !matches!(
                command.method.as_str(),
                "thread.stop" | "thread.visit" | "thread.lifecycle" | "thread.create"
            )
        {
            return Err("This is a remote task result. Open its thread on the target machine, or send a follow-up from its parent.".into());
        }
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
            if command.method != "queue.returnDraft" {
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
        }
        match command.method.as_str() {
            "thread.send" => {
                self.current_mut().recovery_command = None;
                if self.current().phase == "Stopping" {
                    return Err("Wait for the stop to finish.".into());
                }
                let input: crate::protocol::SendInput =
                    serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
                if matches!(input.mode, crate::protocol::SendMode::Unknown) {
                    return Err(crate::protocol::RUN_CONTROL_UPDATE_REQUIRED.into());
                }
                let text = input.text;
                let attachments = input
                    .attachments
                    .iter()
                    .map(|value| Attachment::from_value(value).ok_or("Invalid local attachment."))
                    .collect::<Result<Vec<_>, _>>()?;
                attachments::validate_prompt_images(&attachments)?;
                let context = input
                    .context
                    .filter(|v| !v.is_null())
                    .map(|v| crate::composer_context::decode_message_context(&v))
                    .transpose()?;
                let mut prompt = crate::composer_tokens::received_prompt_with_context(
                    text,
                    attachments,
                    input.draft,
                    context,
                )?;
                attachments::stage_prompt_files(&mut prompt)?;
                self.current_mut().draft = prompt.draft.as_ref().map_or_else(
                    || crate::composer_rich::draft_from_canonical(&prompt.text),
                    crate::composer_rich::Snapshot::editor,
                );
                self.current_mut().draft_images = prompt
                    .draft
                    .as_ref()
                    .map_or_else(|| prompt.images.to_vec(), |draft| draft.attachments.clone());
                self.current_mut().draft_context = prompt
                    .draft
                    .as_ref()
                    .filter(|d| !d.context.is_empty())
                    .map(|d| d.context.clone())
                    .or_else(|| prompt.context.as_ref().map(|c| c.records.clone()))
                    .unwrap_or_default();
                self.current_mut().native.follow_up =
                    matches!(input.mode, crate::protocol::SendMode::Queue);
                let _ = self.dispatch_send_with_options(
                    Some(input.fresh_context == Some(true)),
                    input.compact_before_send,
                );
                if self.current().has_draft_content() {
                    let error = self
                        .current()
                        .error
                        .clone()
                        .or(self.current().attachment_error.clone())
                        .unwrap_or("The thread cannot accept input now.".into());
                    self.current_mut().draft.replace_text("");
                    self.current_mut().draft_images.clear();
                    self.current_mut().draft_context.clear();
                    self.current_mut().retained_context = Default::default();
                    return Err(error);
                }
            }
            "thread.stop" => {
                if args
                    .get("runId")
                    .is_some_and(|id| !id.is_null() && !id.is_string())
                {
                    return Err("runId must be a run identity.".into());
                }
                if let Some(target) = args.get("runId")
                    && *target != json!(self.current().run_id)
                {
                    return Err(
                        "This run is no longer active. The newer run was not stopped.".into(),
                    );
                }
                self.current_mut().recovery_command = Some(command.command_id);
                self.current_mut().stop();
                self.owe_remote_stops_for(self.current().id);
                if self.current().has_draft_content() {
                    let input = json!({"text":self.current().draft.text,"attachments":self.current().draft_images,
                        "draft":crate::composer_rich::Snapshot::capture_with_context(&self.current().draft,&self.current().draft_images,&self.current().draft_context),"commandId":command.command_id});
                    self.recovered_inputs.insert(self.current().id, input);
                    self.current_mut().draft.replace_text("");
                    self.current_mut().draft_images.clear();
                    self.current_mut().draft_context.clear();
                    self.current_mut().retained_context = Default::default();
                }
            }
            "thread.resume" => {
                if args
                    .get("continuationOfRunId")
                    .is_some_and(|id| !id.is_null() && !id.is_string())
                {
                    return Err("continuationOfRunId must be a run identity.".into());
                }
                if args["generation"] != json!(self.current().request_generation) {
                    return Err("This resume belongs to an earlier runtime generation.".into());
                }
                self.current_mut().resume_run(
                    args["continuationOfRunId"].as_str(),
                    args["releaseQueue"] == true,
                )?;
                if self.current().pi.is_none() {
                    self.connect_thread(self.active);
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
                if let Some(tier) = args["serviceTier"].as_str() {
                    self.current_mut().set_service_tier(tier.into());
                }
                if !args["options"].is_null() {
                    let opts: Vec<crate::model_selection::ProviderOptionSelection> =
                        serde_json::from_value(args["options"].clone())
                            .map_err(|e| format!("Invalid options: {e}"))?;
                    for opt in opts {
                        self.current_mut().set_option(opt.id, opt.value);
                    }
                }
                if let Some(access) = args["access"].as_str() {
                    self.set_access_mode(
                        AccessMode::parse(access).unwrap_or(AccessMode::Supervised),
                    );
                } else {
                    self.reconcile_current_access_mode();
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
                let occupied: Vec<usize> =
                    serde_json::from_value(args["occupiedDrafts"].clone()).unwrap_or_default();
                let seed = if args["seed"].is_object() {
                    args["seed"].clone()
                } else {
                    self.composer_seed()
                };
                self.create_thread_in(project, &occupied, &seed);
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
                if let Some(tier) = args["serviceTier"].as_str() {
                    self.current_mut().set_service_tier(tier.into());
                }
                if let Some(access) = args["access"].as_str() {
                    self.current_mut().native.access =
                        AccessMode::parse(access).unwrap_or(AccessMode::Supervised);
                }
                self.reconcile_current_access_mode();
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
                if args["blankOnly"] == true {
                    if self.current().blank_draft() {
                        self.remove_threads(&std::collections::HashSet::from([self.current().id]));
                    }
                } else {
                    self.delete_thread = Some(self.current().id);
                    self.delete_roots =
                        serde_json::from_value(args["threadIds"].clone()).unwrap_or_default();
                    self.confirm_delete();
                }
            }
            "thread.lifecycle" => {
                let id = self.current().id;
                match args["action"].as_str() {
                    Some("pin") => self.pin_thread(id),
                    Some("archive") => self.archive_thread_record(id, args["keepBlank"] == true),
                    Some("discard") => self.discard_draft(id),
                    Some("unarchive") => {
                        self.invalidate_undo(id);
                        self.current_mut().life.archived_at = None;
                    }
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
                if self
                    .undo
                    .as_ref()
                    .is_none_or(|notice| notice.expires <= lifecycle::now())
                    || json!(self.undo) != args["expected"]
                {
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
            "settings.threadDefaults" => {
                let mut defaults: crate::ui::settings_ui::ThreadDefaults =
                    serde_json::from_value(args.clone()).map_err(|_| "Invalid thread defaults.")?;
                defaults.validate()?;
                if let Some(mode) = AccessMode::parse(&defaults.access)
                    && let Some(clamped) = crate::model::clamp_managed_access_mode(
                        self.managed_policy.as_ref(),
                        mode,
                        &AccessMode::ALL,
                    )
                {
                    defaults.access = clamped.id().into();
                }
                self.thread_defaults = defaults;
                self.checkpoint();
            }
            "settings.defaultHarness" => {
                let harness = text("harness")?;
                if !self.harness_ids().contains(&harness) {
                    return Err(format!("Unknown harness {harness}."));
                }
                self.default_harness = harness;
                self.checkpoint();
            }
            "settings.modelFavorites" => {
                if let Some(migrated) = args.get("migrateFavorites").and_then(|v| v.as_array()) {
                    if self.model_favorites.is_none() {
                        let parsed: Vec<crate::model_picker::ModelFavorite> =
                            serde_json::from_value(Value::Array(migrated.clone()))
                                .map_err(|_| "Invalid model favorites.")?;
                        self.model_favorites = Some(parsed);
                        self.checkpoint();
                    }
                    return Ok(json!({"threadId":self.current().id}));
                }
                let provider = text("provider")?;
                let model = text("model")?;
                let favorite = args["favorite"]
                    .as_bool()
                    .ok_or("favorite must be a boolean.")?;
                let list = self.model_favorites.get_or_insert_with(Vec::new);
                let exists = list
                    .iter()
                    .any(|item| item.provider == provider && item.model == model);
                if favorite {
                    if !exists {
                        list.push(crate::model_picker::ModelFavorite { provider, model });
                    }
                } else {
                    list.retain(|item| item.provider != provider || item.model != model);
                }
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
                return Err(crate::protocol::COMPOSER_UPDATE_REQUIRED.into());
            }
            "queue.update" => {
                let images = args["attachments"]
                    .as_array()
                    .ok_or("attachments must be an array.")?
                    .iter()
                    .map(|value| Attachment::from_value(value).ok_or("Invalid local attachment."))
                    .collect::<Result<Vec<_>, _>>()?;
                let context = args
                    .get("context")
                    .filter(|v| !v.is_null())
                    .map(crate::composer_context::decode_message_context)
                    .transpose()?;
                let mut prompt = crate::composer_tokens::received_prompt_with_context(
                    text("text")?,
                    images,
                    args.get("draft").cloned(),
                    context,
                )?;
                attachments::stage_prompt_files(&mut prompt)?;
                self.current_mut().update_queued(
                    number("inputId")?,
                    &text("generation")?,
                    &text("expectedRevision")?,
                    prompt,
                )?;
            }
            "queue.returnDraft" => {
                self.current_mut().recovery_command = Some(command.command_id);
                self.current_mut().native.queue_held = false;
                if self.current().pi.is_some() {
                    self.current_mut().command("clear_queue", json!({}));
                }
                let prompts: Vec<_> = self.current_mut().native.queue.drain(..).map(|q| {
                    json!({"text":q.prompt.text,"attachments":q.prompt.images.as_ref(),"context":q.prompt.context,"draft":q.prompt.draft})
                }).collect();
                return Ok(json!({"prompts":prompts}));
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
            "catalog.refresh" => {
                if args["modelCatalogOnly"] == true {
                    self.refresh_catalog_on_open();
                } else if args["force"] == true {
                    self.force_refresh_catalog();
                } else {
                    self.refresh_catalog();
                }
            }
            "commands.refresh" => {
                if self.current().pi.is_none()
                    && self.current().error.is_none()
                    && self.current().status != Status::Failed
                    && !self.current().archived()
                {
                    self.connect_thread(self.active);
                } else {
                    self.current_mut().refresh_commands();
                }
            }
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
            "captain.scope" => {
                let captain = number("captain")?;
                let remote = args
                    .get("remoteProjects")
                    .map(|value| serde_json::from_value::<Vec<RemoteProject>>(value.clone()))
                    .transpose()
                    .map_err(|e| e.to_string())?;
                if let Some(remote) = &remote {
                    self.validate_remote_captain_scope(captain, remote.clone())?;
                }
                self.set_captain_scope(
                    captain,
                    serde_json::from_value(args["projects"].clone()).map_err(|e| e.to_string())?,
                )?;
                if let Some(remote) = remote {
                    self.set_remote_captain_scope(captain, remote)?;
                }
            }
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
            inputs.push(json!({"text":prompt.text,"images":prompt.images.iter().map(Attachment::to_value).collect::<Vec<_>>(),"after":after,"context":prompt.context,"draft":prompt.draft}));
        }
        if let Some((_, prompt, after)) = &self.pending_prompt {
            inputs.push(json!({"text":prompt.text,"images":prompt.images.iter().map(Attachment::to_value).collect::<Vec<_>>(),"after":after,"context":prompt.context,"draft":prompt.draft}));
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

    fn tab_client() -> (State, State, tokio::sync::mpsc::Receiver<Command>) {
        let mut server = State::new(".".into(), false);
        server.threads = (1..=4)
            .map(|id| {
                let mut thread = Thread::new(id);
                thread.title = format!("Thread {id}");
                thread.renamed = true;
                thread.project = Some(1);
                thread
            })
            .collect();
        server.next_id = 5;
        server.tabs = vec![1, 2, 3];
        let mut state = State::new(".".into(), false);
        let mut client = crate::client::Client::new(state.data_dir.clone());
        let mut commands = client.test_commands();
        state.client = Some(client);
        state.apply_projection(server.projection()).unwrap();
        state.tabs = vec![1, 2, 3];
        state.select_thread(1);
        while commands.try_recv().is_ok() {}
        (state, server, commands)
    }

    fn accepted(result: Value) -> Value {
        json!({"result":{"status":"accepted","result":result}})
    }

    #[test]
    fn projections_never_open_server_tabs_or_override_a_later_click() {
        let (mut state, mut server, _) = tab_client();
        state.client_views["localUi"] = json!({"active":1,"tabs":[1]});
        state.select_thread(2);
        let before = (state.tabs.clone(), state.current().id);
        server.select_thread(3);
        server.new_thread_in(None);
        state.apply_projection(server.projection()).unwrap();
        state.apply_projection(server.projection()).unwrap();
        assert_eq!((state.tabs.clone(), state.current().id), before);
        server.request_delete(3);
        server.confirm_delete();
        state.apply_projection(server.projection()).unwrap();
        assert_eq!((state.tabs.clone(), state.current().id), before);
    }

    #[test]
    fn first_projection_restores_the_device_session_including_blank_tabs() {
        let (mut state, mut server, _) = tab_client();
        server.new_thread_in(None);
        let blank = server.current().id;
        let mut ui = state.ui_snapshot();
        ui["active"] = json!(blank);
        ui["tabs"] = json!([2, blank, 1]);
        state.client_views = json!({"localUi":ui});
        state.tab_epoch = 0;
        state.apply_projection(server.projection()).unwrap();
        assert_eq!(state.tabs, [2, blank, 1]);
        assert_eq!(state.current().id, blank);
    }

    #[test]
    fn a_click_before_the_first_projection_wins_over_the_saved_active_thread() {
        let (mut state, server, _) = tab_client();
        let mut ui = state.ui_snapshot();
        ui["active"] = json!(1);
        ui["tabs"] = json!([1]);
        state.client_views = json!({"localUi":ui});
        state.select_thread(2);
        state.apply_projection(server.projection()).unwrap();
        assert_eq!(state.current().id, 3);
        assert_eq!(state.tabs, [1, 2, 3]);
    }

    #[test]
    fn only_owned_current_open_receipts_can_open_tabs_even_before_projection() {
        let (mut state, mut server, mut commands) = tab_client();
        let other = Command::new("thread.create", json!({}));
        state.apply_tab_receipt("local", &other, &accepted(json!({"threadId":4})));
        state.open_requested_tab();
        assert_eq!(state.tabs, [1, 2, 3]);
        state
            .request_tab_command(
                "thread.create",
                json!({"project":null}),
                TabIntent::Open(None),
            )
            .unwrap()
            .unwrap();
        let command = std::iter::from_fn(|| commands.try_recv().ok())
            .find(|c| c.method == "thread.create")
            .unwrap();
        let result = server.execute_command(&command).unwrap();
        let fresh = result["threadId"].as_u64().unwrap() as usize;
        state.apply_tab_receipt("local", &command, &accepted(result));
        state.open_requested_tab();
        assert_eq!(state.current().id, 2);
        state.apply_projection(server.projection()).unwrap();
        state.open_requested_tab();
        assert_eq!(state.tabs, [1, 2, fresh, 3]);
        assert_eq!(state.current().id, fresh);

        state
            .request_tab_command("thread.create", json!({"project":1}), TabIntent::Open(None))
            .unwrap()
            .unwrap();
        let command = std::iter::from_fn(|| commands.try_recv().ok())
            .find(|c| c.method == "thread.create")
            .unwrap();
        state.select_thread(0);
        let before = state.tabs.clone();
        let result = server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &command, &accepted(result));
        state.open_requested_tab();
        assert_eq!(state.current().id, 1);
        assert_eq!(state.tabs, before);
    }

    #[test]
    fn client_parks_and_undo_use_device_tab_positions_not_server_positions() {
        for action in ["settle", "archive", "delete", "drop"] {
            let (mut state, mut server, mut commands) = tab_client();
            server.tabs = vec![4, 3, 2, 1];
            if action == "delete" {
                state.request_delete(2);
                state.confirm_delete();
            } else if action == "settle" {
                state.settle_thread(2);
            } else if action == "drop" {
                state.drop_thread(2, Section::Settled, None);
            } else {
                state.archive_thread(2);
            }
            let command = std::iter::from_fn(|| commands.try_recv().ok())
                .find(|c| c.method != "thread.visit")
                .unwrap();
            let result = server.execute_command(&command).unwrap();
            state.apply_projection(server.projection()).unwrap();
            assert_eq!(state.current().id, 2, "projection must not navigate");
            assert!(
                state
                    .tab_intents
                    .contains_key(&("local".into(), command.command_id)),
                "missing owned {action} intent"
            );
            assert_eq!(
                state.tab_intents[&("local".into(), command.command_id)].0,
                state.tab_epoch,
                "projection changed the navigation epoch"
            );
            state.apply_tab_receipt("local", &command, &accepted(result));
            assert_eq!(state.tabs, [1, 3], "{action}");
            assert_eq!(state.current().id, 3);
            state.undo_thread_action();
            let command = std::iter::from_fn(|| commands.try_recv().ok())
                .find(|c| c.method == "thread.undo")
                .unwrap();
            let result = server.execute_command(&command).unwrap();
            state.apply_projection(server.projection()).unwrap();
            state.apply_tab_receipt("local", &command, &accepted(result));
            assert_eq!(state.tabs, [1, 2, 3], "{action}");
            assert_eq!(state.current().id, 2);
        }
    }

    #[test]
    fn client_blank_tab_close_deletes_draft_but_written_drafts_survive() {
        let (mut state, mut server, mut commands) = tab_client();
        server.threads[1] = Thread::new(2);
        state.apply_projection(server.projection()).unwrap();
        state.close_tab(2);
        let command = std::iter::from_fn(|| commands.try_recv().ok())
            .find(|c| c.method == "thread.delete")
            .unwrap();
        assert_eq!(command.params["blankOnly"], true);
        let result = server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &command, &accepted(result));
        assert_eq!(state.tabs, [1, 3]);
        assert_eq!(state.current().id, 3);
        assert!(!server.threads.iter().any(|t| t.id == 2));
        state.apply_projection(server.projection()).unwrap();
        assert!(!state.threads.iter().any(|t| t.id == 2));
        state.current_mut().draft.replace_text("Keep this");
        state.close_tab(3);
        assert!(
            state
                .threads
                .iter()
                .any(|t| t.id == 3 && t.draft.text == "Keep this")
        );
        assert_eq!(state.current().id, 1);
    }

    #[test]
    fn client_last_archive_discard_and_delete_tabs_get_a_local_replacement() {
        for action in ["archive", "discard", "delete"] {
            let (mut state, mut server, mut commands) = tab_client();
            state.tabs = vec![2];
            match action {
                "archive" => state.archive_thread(2),
                "discard" => {
                    state.current_mut().draft.replace_text("Discarded");
                    state.discard_draft(2);
                }
                _ => {
                    state.request_delete(2);
                    state.confirm_delete();
                }
            }
            let command = std::iter::from_fn(|| commands.try_recv().ok())
                .find(|c| matches!(c.method.as_str(), "thread.lifecycle" | "thread.delete"))
                .unwrap();
            let result = server.execute_command(&command).unwrap();
            state.apply_projection(server.projection()).unwrap();
            state.apply_tab_receipt("local", &command, &accepted(result));
            assert!(state.current().client_draft);
            assert!(
                commands.try_recv().is_err(),
                "replacement must not create a server thread"
            );
            state.apply_projection(server.projection()).unwrap();
            state.open_requested_tab();
            assert!(state.current().blank_draft());
            assert_eq!(state.current().project, Some(1));
            assert_eq!(state.tabs, [state.current().id]);
        }
    }

    #[test]
    fn client_last_settle_tab_replaces_itself_with_a_sidebar_row() {
        let (mut state, mut server, mut commands) = tab_client();
        state.tabs = vec![2];
        state.settle_thread(2);
        let command = commands.try_recv().unwrap();
        let result = server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &command, &accepted(result));
        let order = state.sidebar_threads();
        let position = order
            .iter()
            .position(|i| state.threads[*i].id == 2)
            .unwrap();
        let expected = state.threads[order[(position + 1) % order.len()]].id;
        assert_eq!(state.tabs, [expected]);
        assert_eq!(state.current().id, expected);
    }

    #[test]
    fn client_discard_archives_without_transmitting_the_draft_and_undo_restores_it() {
        let (mut state, mut server, mut commands) = tab_client();
        state
            .current_mut()
            .draft
            .replace_text("Private discarded text");
        state.discard_draft(2);
        let command = commands.try_recv().unwrap();
        assert_eq!(command.method, "thread.lifecycle");
        assert!(
            !command
                .params
                .to_string()
                .contains("Private discarded text")
        );
        let result = server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &command, &accepted(result));
        assert_eq!(state.tabs, [1, 3]);
        assert!(
            server
                .threads
                .iter()
                .find(|t| t.id == 2)
                .unwrap()
                .archived()
        );
        state.undo_thread_action();
        let command = std::iter::from_fn(|| commands.try_recv().ok())
            .find(|c| c.params["action"] == "unarchive")
            .unwrap();
        server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        assert_eq!(state.current().id, 2);
        assert_eq!(state.current().draft.text, "Private discarded text");
        assert_eq!(state.tabs, [1, 2, 3]);
        assert!(!state.current().archived());
    }

    #[test]
    fn client_captain_reset_replaces_the_old_home_tab() {
        let (mut state, mut server, mut commands) = tab_client();
        server.captains = Captain::presets();
        server.open_captain(0);
        let old = server.current().id;
        state.apply_projection(server.projection()).unwrap();
        state.select_thread(state.threads.iter().position(|t| t.id == old).unwrap());
        let tabs = state.tabs.clone();
        state.reset_captain_conversation(0).unwrap();
        let command = std::iter::from_fn(|| commands.try_recv().ok())
            .find(|c| c.method == "captain.reset")
            .unwrap();
        let result = server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &command, &accepted(result));
        state.open_requested_tab();
        let fresh = state.current().id;
        assert_ne!(fresh, old);
        assert_eq!(
            state.tabs,
            tabs.into_iter()
                .map(|id| if id == old { fresh } else { id })
                .collect::<Vec<_>>()
        );
    }

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
        // This fixture starts with an existing device session on each host.
        state.checkpoint_client();
        state.machines.ui["studio"] = state.machines.ui["local"].clone();
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
        assert!(
            state.current().draft.text.is_empty(),
            "the submitted snapshot owns the draft until its receipt"
        );
        assert!(state.current().remote_send.is_some());
        state.select_machine("local");
        assert!(
            state.current().draft.text.is_empty(),
            "the draft moved, it was not copied"
        );
    }

    #[test]
    fn client_delete_closes_descendant_tabs_and_undo_focuses_the_original_active_tab() {
        let (mut state, mut server, mut commands) = tab_client();
        server.threads[1].parent = Some(1);
        server.threads[1].delegated = true;
        server.threads[2].parent = Some(1);
        server.threads[2].delegated = true;
        state.apply_projection(server.projection()).unwrap();
        state.select_thread(3);
        state.select_thread(1);
        let before = state.tabs.clone();
        state.request_delete(1);
        state.confirm_delete();
        let command = std::iter::from_fn(|| commands.try_recv().ok())
            .find(|c| c.method == "thread.delete")
            .unwrap();
        let result = server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &command, &accepted(result));
        assert_eq!(state.tabs, [4]);
        assert_eq!(state.current().id, 4);
        state.undo_thread_action();
        let command = std::iter::from_fn(|| commands.try_recv().ok())
            .find(|c| c.method == "thread.undo")
            .unwrap();
        let result = server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &command, &accepted(result));
        assert_eq!(state.tabs, before);
        assert_eq!(state.current().id, 2);
    }

    #[test]
    fn client_bulk_delete_is_one_undoable_action_and_keeps_an_unaffected_active_tab() {
        let (mut state, mut server, mut commands) = tab_client();
        state.selected_threads = vec![1, 3];
        state.request_delete(1);
        state.confirm_delete();
        let command = commands.try_recv().unwrap();
        assert_eq!(command.params["threadIds"], json!([1, 3]));
        let result = server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &command, &accepted(result));
        assert_eq!(state.tabs, [2]);
        assert_eq!(state.current().id, 2);
        assert_eq!(server.undo.as_ref().unwrap().entries.len(), 2);
        state.undo_thread_action();
        let command = commands.try_recv().unwrap();
        let result = server.execute_command(&command).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &command, &accepted(result));
        assert_eq!(state.tabs, [1, 2, 3]);
    }

    #[test]
    fn delete_undo_waits_for_its_projection_and_a_later_click_can_cancel_navigation() {
        for abandon in [false, true] {
            let (mut state, mut server, mut commands) = tab_client();
            state.request_delete(2);
            state.confirm_delete();
            let command = commands.try_recv().unwrap();
            let result = server.execute_command(&command).unwrap();
            state.apply_projection(server.projection()).unwrap();
            state.apply_tab_receipt("local", &command, &accepted(result));
            state.apply_projection(server.projection()).unwrap();
            assert!(!state.threads.iter().any(|t| t.id == 2));
            state.undo_thread_action();
            let command = std::iter::from_fn(|| commands.try_recv().ok())
                .find(|c| c.method == "thread.undo")
                .unwrap();
            let result = server.execute_command(&command).unwrap();
            state.apply_tab_receipt("local", &command, &accepted(result));
            assert_eq!(state.current().id, 3);
            assert_eq!(state.tabs, [1, 3]);
            if abandon {
                state.select_thread(0);
            }
            state.apply_projection(server.projection()).unwrap();
            state.open_requested_tab();
            if abandon {
                assert_eq!(state.current().id, 1);
                assert_eq!(state.tabs, [1, 3]);
            } else {
                assert_eq!(state.current().id, 2);
                assert_eq!(state.tabs, [1, 2, 3]);
            }
        }
    }

    #[test]
    fn client_empty_draft_archive_and_discard_restore_the_tab_on_undo() {
        for action in [Action::Archive, Action::Discard] {
            let (mut state, mut server, mut commands) = tab_client();
            server.threads[1] = Thread::new(2);
            state.apply_projection(server.projection()).unwrap();
            if action == Action::Archive {
                state.archive_thread(2);
            } else {
                state.discard_draft(2);
            }
            let command = commands.try_recv().unwrap();
            let result = server.execute_command(&command).unwrap();
            state.apply_projection(server.projection()).unwrap();
            state.apply_tab_receipt("local", &command, &accepted(result));
            state.apply_projection(server.projection()).unwrap();
            assert_eq!(state.tabs, [1, 3]);
            assert_eq!(state.undo.as_ref().unwrap().action, action);
            state.undo_thread_action();
            let command = std::iter::from_fn(|| commands.try_recv().ok())
                .find(|c| c.method == "thread.undo")
                .unwrap();
            let result = server.execute_command(&command).unwrap();
            state.apply_tab_receipt("local", &command, &accepted(result));
            state.apply_projection(server.projection()).unwrap();
            state.open_requested_tab();
            assert_eq!(state.current().id, 2);
            assert_eq!(state.tabs, [1, 2, 3]);
            assert!(state.undo.is_none());
        }
    }

    #[test]
    fn the_latest_create_request_wins_without_waiting_for_an_earlier_receipt() {
        let (mut state, mut server, mut commands) = tab_client();
        state
            .request_tab_command(
                "thread.create",
                json!({"project":null}),
                TabIntent::Open(None),
            )
            .unwrap()
            .unwrap();
        state
            .request_tab_command("thread.create", json!({"project":1}), TabIntent::Open(None))
            .unwrap()
            .unwrap();
        let creates: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok())
            .filter(|c| c.method == "thread.create")
            .collect();
        assert_eq!(creates.len(), 2);
        let first = server.execute_command(&creates[0]).unwrap();
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &creates[0], &accepted(first));
        state.open_requested_tab();
        assert_eq!(state.current().id, 2);
        let second = server.execute_command(&creates[1]).unwrap();
        let fresh = second["threadId"].as_u64().unwrap() as usize;
        state.apply_projection(server.projection()).unwrap();
        state.apply_tab_receipt("local", &creates[1], &accepted(second));
        state.open_requested_tab();
        assert_eq!(state.current().id, fresh);
        assert_eq!(state.tabs, [1, 2, fresh, 3]);
    }

    #[test]
    fn client_create_carries_the_composer_selection_captured_on_this_device() {
        let (mut state, mut server, mut commands) = tab_client();
        let source = &mut server.threads[1];
        source.harness = "codex".into();
        source.models = vec![AgentModel::from_harness_value(&json!({"provider":"codex","id":"chosen","name":"Chosen","reasoningLevels":["high"]}), "codex").unwrap()];
        source.models[0].thinking_levels = vec!["high".into()];
        source.selected_model = Some(("codex".into(), "chosen".into()));
        source.selected_thinking = Some("high".into());
        state.apply_projection(server.projection()).unwrap();
        state.new_thread_in(Some(1));
        let draft = state.current().id;
        assert_eq!(state.tabs, [1, 2, draft, 3]);
        assert!(state.current().client_draft);
        assert!(commands.try_recv().is_err(), "new drafts are device-owned");
        server.threads[1].selected_model = Some(("codex".into(), "changed-elsewhere".into()));
        state.apply_projection(server.projection()).unwrap();
        assert_eq!(state.current().id, draft);
        assert_eq!(state.current().harness, "codex");
        assert_eq!(
            state.current().selected_model,
            Some(("codex".into(), "chosen".into()))
        );
        assert_eq!(state.current().selected_thinking.as_deref(), Some("high"));
        state.new_thread_in(Some(1));
        assert_eq!(
            state.current().id,
            draft,
            "reuse the one blank project draft"
        );
        assert_eq!(state.tabs, [1, 2, draft, 3]);
        state
            .current_mut()
            .draft
            .replace_text("Send with the inherited selection");
        state.send();
        let create = commands.try_recv().unwrap();
        assert_eq!(create.params["harness"], "codex");
        assert_eq!(create.params["selectedModel"], json!(["codex", "chosen"]));
        assert_eq!(create.params["thinking"], "high");
    }

    #[test]
    fn native_catalog_projection_populates_a_device_draft_before_any_native_session() {
        let (mut state, mut server, mut commands) = tab_client();
        server.codex_catalog.models = vec![AgentModel::from_harness_value(&json!({
            "provider":"codex","id":"catalog-only","name":"Catalog only","thinkingLevels":["low","high"],
            "serviceTiers":[{"id":"default","name":"Standard"},{"id":"fast","name":"Fast"}]
        }),"codex").unwrap()];
        state.apply_projection(server.projection()).unwrap();
        state.new_thread_in(Some(1));
        state.choose_harness("codex");
        assert!(state.current().client_draft);
        assert!(state.current().session.is_none());
        assert_eq!(state.current().models[0].id, "catalog-only");
        state
            .current_mut()
            .set_model("codex".into(), "catalog-only".into());
        state.current_mut().set_service_tier("fast".into());
        assert!(commands.try_recv().is_err());
        state.current_mut().draft.replace_text("Fixture");
        state.send();
        let create = commands.try_recv().unwrap();
        assert_eq!(create.params["serviceTier"], "fast");
        let result = server.execute_command(&create).unwrap();
        let created = server
            .threads
            .iter()
            .find(|thread| Some(thread.id as u64) == result["threadId"].as_u64())
            .unwrap();
        assert_eq!(created.harness_state["serviceTier"], "fast");
    }

    #[test]
    fn client_promotion_replaces_its_tab_in_place_without_overriding_later_navigation() {
        for navigate in [false, true] {
            for receipt_first in [false, true] {
                let (mut state, mut server, mut commands) = tab_client();
                // An open blank server thread must not be reused by this promotion.
                server.threads[0] = Thread::new(1);
                server.threads[0].project = Some(1);
                state.apply_projection(server.projection()).unwrap();
                state.new_thread_in(Some(1));
                let draft = state.current().id;
                state.current_mut().draft.replace_text("Captured prompt");
                state.send();
                let create = commands.try_recv().unwrap();
                let result = server.execute_command(&create).unwrap();
                let id = result["threadId"].as_u64().unwrap() as usize;
                assert_ne!(id, 1);
                let expected = vec![1, 2, id, 3];
                state.complete_client_creation("local", &create, &accepted(result));
                let send = commands.try_recv().unwrap();
                assert_eq!(send.method, "thread.send");
                assert_eq!(send.params["threadId"], id);
                if navigate {
                    state.select_thread(0);
                }
                if receipt_first {
                    // The draft still has its device ID. Match the send by command ID.
                    state.machines.receipts.insert(
                        "local".into(),
                        vec![crate::client::Event::Receipt(
                            send.clone(),
                            accepted(json!({})),
                        )],
                    );
                    state.poll_client(&mut Tx::default());
                    assert!(
                        state
                            .threads
                            .iter()
                            .find(|t| t.id == draft)
                            .unwrap()
                            .draft
                            .text
                            .is_empty()
                    );
                }
                state.apply_projection(server.projection()).unwrap();
                state.finish_client_promotions();
                assert_eq!(state.tabs, expected);
                assert_eq!(state.current().id, if navigate { 1 } else { id });
                assert!(!state.threads.iter().any(|t| t.id == draft));
                assert!(
                    !state
                        .threads
                        .iter()
                        .find(|t| t.id == id)
                        .unwrap()
                        .client_draft
                );
                if !receipt_first {
                    state.machines.receipts.insert(
                        "local".into(),
                        vec![crate::client::Event::Receipt(send, accepted(json!({})))],
                    );
                    state.poll_client(&mut Tx::default());
                }
                assert!(
                    state
                        .threads
                        .iter()
                        .find(|t| t.id == id)
                        .unwrap()
                        .draft
                        .text
                        .is_empty()
                );
                assert_eq!(state.tabs, expected);
                assert_eq!(state.current().id, if navigate { 1 } else { id });
            }
        }
    }

    #[test]
    fn fork_and_captain_receipts_open_only_for_owned_current_tab_intents() {
        for method in [
            "thread.fork",
            "captain.open",
            "captain.put",
            "captain.reset",
        ] {
            for stale in [false, true] {
                let (mut state, _, mut commands) = tab_client();
                let other = Command::new(method, json!({}));
                state.machines.receipts.insert(
                    "local".into(),
                    vec![crate::client::Event::Receipt(
                        other,
                        accepted(json!({"threadId":4})),
                    )],
                );
                state.poll_client(&mut Tx::default());
                assert_eq!(state.tabs, [1, 2, 3], "{method}");
                assert_eq!(state.current().id, 2, "{method}");
                match method {
                    "captain.put" => {
                        state.begin_new_captain();
                        state.save_captain();
                    }
                    "captain.reset" => state.reset_captain_conversation(0).unwrap(),
                    _ => {
                        state.forward(method, json!({"threadId":2,"captain":0}));
                    }
                }
                let command = commands.try_recv().unwrap();
                assert_eq!(command.method, method);
                if stale {
                    state.select_thread(0);
                }
                state.machines.receipts.insert(
                    "local".into(),
                    vec![crate::client::Event::Receipt(
                        command,
                        accepted(json!({"threadId":4})),
                    )],
                );
                state.poll_client(&mut Tx::default());
                assert_eq!(
                    state.tabs,
                    if stale {
                        vec![1, 2, 3]
                    } else {
                        vec![1, 2, 4, 3]
                    },
                    "{method}"
                );
                assert_eq!(state.current().id, if stale { 1 } else { 4 }, "{method}");
            }
        }
    }

    #[test]
    fn closing_a_client_draft_discards_it_without_any_server_command() {
        for sole in [false, true] {
            let (mut state, server, mut commands) = tab_client();
            state.new_thread_in(Some(1));
            let draft = state.current().id;
            if sole {
                state.tabs = vec![draft];
            }
            state.close_tab(draft);
            assert!(!state.threads.iter().any(|t| t.id == draft));
            assert!(!state.tabs.contains(&draft));
            assert!(state.tabs.contains(&state.current().id));
            if sole {
                assert!(state.current().client_draft);
                assert_eq!(state.current().project, Some(1));
                assert_eq!(state.tabs, [state.current().id]);
            } else {
                assert_eq!(state.tabs, [1, 2, 3]);
                assert_eq!(state.current().id, 3);
            }
            assert!(commands.try_recv().is_err());
            state.apply_projection(server.projection()).unwrap();
            assert!(
                !state.threads.iter().any(|t| t.id == draft),
                "projection cannot resurrect a closed draft"
            );
        }
    }

    #[test]
    fn client_draft_archive_discard_and_delete_are_local_and_restore_the_tab_on_undo() {
        for action in [Action::Archive, Action::Discard, Action::Delete] {
            let (mut state, server, mut commands) = tab_client();
            state.new_thread_in(Some(1));
            let draft = state.current().id;
            state
                .current_mut()
                .draft
                .replace_text("Private local draft");
            state.tabs = vec![draft];
            match action {
                Action::Archive => state.archive_thread(draft),
                Action::Discard => state.discard_draft(draft),
                _ => {
                    state.request_delete(draft);
                    state.confirm_delete();
                }
            }
            let replacement = state.current().id;
            assert_ne!(replacement, draft);
            assert!(state.current().client_draft);
            assert_eq!(state.current().project, Some(1));
            assert_eq!(state.tabs, [replacement]);
            assert!(commands.try_recv().is_err());
            if action == Action::Archive {
                let mut restored = State::new(PathBuf::new(), false);
                restored.client = Some(crate::client::Client::new(restored.data_dir.clone()));
                restored.client_views["localUi"] = state.machines.ui["local"].clone();
                restored.apply_projection(server.projection()).unwrap();
                let archived = restored.threads.iter().find(|t| t.id == draft).unwrap();
                assert!(archived.archived());
                assert_eq!(archived.draft.text, "Private local draft");
                assert_eq!(restored.tabs, [replacement]);
            }
            state.apply_projection(server.projection()).unwrap();
            state.undo_thread_action();
            assert_eq!(state.tabs, [draft, replacement]);
            assert_eq!(state.current().id, draft);
            assert_eq!(state.current().draft.text, "Private local draft");
            assert!(commands.try_recv().is_err());
        }
    }

    #[test]
    fn settling_a_client_draft_and_undoing_it_never_send_its_device_id_to_the_server() {
        let (mut state, server, mut commands) = tab_client();
        state.new_thread_in(Some(1));
        let draft = state.current().id;
        state.current_mut().draft.replace_text("Still private");
        state.settle_thread(draft);
        assert_eq!(state.tabs, [1, 2, 3]);
        assert_eq!(state.current().id, 3);
        state.apply_projection(server.projection()).unwrap();
        state.undo_thread_action();
        assert_eq!(state.tabs, [1, 2, draft, 3]);
        assert_eq!(state.current().id, draft);
        assert_eq!(state.current().draft.text, "Still private");
        for command in std::iter::from_fn(|| commands.try_recv().ok()) {
            assert_eq!(command.method, "thread.visit");
            assert_ne!(command.params["threadId"], json!(draft));
        }
    }

    #[test]
    fn machine_switches_without_saved_tabs_open_a_device_draft_and_restore_it() {
        for cached in [false, true] {
            let (mut state, mut local, mut remote, projection) = paired_client_fixture();
            state.machines.ui.as_object_mut().unwrap().remove("studio");
            if !cached {
                state.machines.views.remove("studio");
            }
            state.select_machine("studio");
            let draft = state.current().id;
            assert!(state.current().client_draft);
            assert_eq!(state.tabs, [draft]);
            assert_eq!(state.machines.selected, "studio");
            if !cached {
                state.apply_projection(projection).unwrap();
                assert_eq!(state.current().id, draft);
                assert_eq!(state.tabs, [draft]);
            }
            state
                .current_mut()
                .draft
                .replace_text("Remote device draft");
            state.select_machine("local");
            state.select_machine("studio");
            assert_eq!(state.current().id, draft);
            assert_eq!(state.tabs, [draft]);
            assert_eq!(state.current().draft.text, "Remote device draft");
            assert!(local.try_recv().is_err());
            assert!(remote.try_recv().is_err());
        }
        let (mut state, _, _, _) = paired_client_fixture();
        state.machines.ui["studio"]["tabs"] = json!([999]);
        state.machines.ui["studio"]["active"] = json!(999);
        state.select_machine("studio");
        assert!(
            state.current().client_draft,
            "deleted saved tabs need a draft too"
        );
        assert_eq!(state.tabs, [state.current().id]);
    }

    #[test]
    fn an_owned_project_put_receipt_selects_only_its_project_even_with_a_thread_id() {
        let (mut state, mut server, mut commands) = tab_client();
        let before = (state.tabs.clone(), state.current().id);
        assert_eq!(
            state.add_project(PathBuf::from("/srv/project")).unwrap(),
            None
        );
        let command = commands.try_recv().unwrap();
        assert!(
            !state
                .tab_intents
                .contains_key(&("local".into(), command.command_id))
        );
        server
            .projects
            .entries
            .push(Project::new(9, PathBuf::from("/srv/project")));
        server.projects.next_id = 10;
        state.apply_projection(server.projection()).unwrap();
        state.machines.receipts.insert(
            "local".into(),
            vec![crate::client::Event::Receipt(
                command,
                accepted(json!({"projectId":9,"threadId":4})),
            )],
        );
        state.poll_client(&mut Tx::default());
        assert_eq!((state.tabs.clone(), state.current().id), before);
        assert_eq!(state.project_scope, ProjectScope::Project(9));
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
    fn failed_remote_send_restores_rich_snapshot_after_later_edits_and_navigation() {
        for navigate in [false, true] {
            let (mut state, _, mut remote, _) = paired_client_fixture();
            state.retarget_draft("studio");
            state.current_mut().draft.replace_text("Submitted");
            state.current_mut().draft.set_selection(0, 9);
            state.current_mut().draft.toggle_mark(rune::Mark::Bold);
            state.send();
            let create = remote.try_recv().unwrap();
            assert!(state.current().draft.text.is_empty());
            state.current_mut().draft.replace_text("Later");
            state.current_mut().draft.set_selection(0, 5);
            state.current_mut().draft.toggle_mark(rune::Mark::Italic);
            state.complete_client_creation(
                "studio",
                &create,
                &json!({"result":{"status":"accepted","result":{"threadId":1}}}),
            );
            let sent = remote.try_recv().unwrap();
            state.finish_client_promotions();
            if navigate {
                state.select_machine("local");
            }
            state.client.as_ref().unwrap().linked["studio"].test_receipt(
                sent,
                json!({"result":{"status":"rejected","result":{"error":"Fixture refusal"}}}),
            );
            state.poll_client(&mut Tx::default());
            if navigate {
                state.select_machine("studio");
                state.poll_client(&mut Tx::default());
            }
            assert_eq!(
                crate::composer_rich::markdown(&state.current().draft),
                "**Submitted**\n\n*Later*",
                "navigate={navigate}"
            );
        }
    }

    #[test]
    fn queued_edits_survive_machine_switches_and_recover_queue_departures_on_return() {
        for leaves_queue in [false, true] {
            for ordinary_empty in [false, true] {
                let (mut state, _, _, mut projection) = paired_client_fixture();
                state.select_thread(0);
                let id = state.current().id;
                let view = projection["threads"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|view| view["id"] == json!(id))
                    .unwrap();
                view["queue"] = json!([[9, "Queued original", []]]);
                state.apply_projection(projection.clone()).unwrap();
                state
                    .machines
                    .views
                    .insert("local".into(), projection.clone());
                state.current_mut().draft.replace_text(if ordinary_empty {
                    ""
                } else {
                    "Ordinary"
                });
                state.current_mut().edit_queued(9);
                state.current_mut().draft.replace_text("Edited");
                state.select_machine("studio");
                if leaves_queue {
                    projection["threads"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|view| view["id"] == json!(id))
                        .unwrap()["queue"] = json!([]);
                    state.machines.views.insert("local".into(), projection);
                }
                state.select_machine("local");
                if leaves_queue {
                    assert!(state.current().queued_edit.is_none());
                    assert_eq!(
                        state.current().draft.text,
                        if ordinary_empty { "Edited" } else { "Ordinary" }
                    );
                } else {
                    assert!(state.current().queued_edit.is_some());
                    assert_eq!(state.current().draft.text, "Edited");
                    state.current_mut().cancel_queued_edit();
                    assert_eq!(
                        state.current().draft.text,
                        if ordinary_empty { "" } else { "Ordinary" }
                    );
                }
            }
        }
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
        let tabs = state.tabs.clone();
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
        assert_eq!(state.tabs, tabs);
        assert!(state.requested_tab.is_none());
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
        assert_eq!(state.tabs, tabs);
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
    fn stopped_work_and_retry_events_survive_projection_and_history_reload() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"message_start","message":{"role":"user","content":"Go"}}));
        thread.apply(json!({"type":"agent_start"}));
        thread.apply(json!({"type":"auto_retry_start","attempt":2,"maxAttempts":5,"errorMessage":"Temporary"}));
        thread.apply(json!({"type":"auto_retry_end","success":true}));
        thread.apply(json!({"type":"agent_settled","aborted":true}));
        thread.work[0].elapsed = Some(134);
        let saved = serde_json::to_value(ThreadView::capture(&thread)).unwrap();
        let view: ThreadView = serde_json::from_value(saved.clone()).unwrap();
        let mut restored = Thread::new(1);
        view.apply(&mut restored).unwrap();
        assert!(restored.work[0].stopped);
        assert_eq!(restored.work[0].elapsed, Some(134));
        assert_eq!(crate::work::retry_events(&restored).len(), 1);
        assert_eq!(crate::work::retry_events(&restored)[0]["success"], true);
        // A provider can omit an empty interrupted response from native replay.
        restored.apply(
            json!({"type":"response","id":"history","command":"get_messages","success":true,
            "data":{"messages":[{"role":"user","content":"Go"}]}}),
        );
        assert!(restored.work[0].stopped);
        assert_eq!(restored.work[0].elapsed, Some(134));
        let mut legacy = saved;
        legacy["work"][0].as_object_mut().unwrap().remove("stopped");
        let view: ThreadView = serde_json::from_value(legacy).unwrap();
        view.apply(&mut restored).unwrap();
        assert!(!restored.work[0].stopped);
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
        state.checkpoint_client();
        state.machines.ui["server-b"] = state.machines.ui["local"].clone();
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
        state.preferences.confirm_delete = false;
        state.current_mut().draft.replace_text("local unsent text");
        state.checkpoint_client();
        let mut remote_ui = state.machines.ui["local"].clone();
        remote_ui["dark"] = json!(true);
        remote_ui["zoom"] = json!(1.8);
        remote_ui["sidebar"] = json!(true);
        remote_ui["sidebarWidth"] = json!(200.0);
        remote_ui["shelves"] = json!([false, false, true]);
        remote_ui["preferences"]["confirm_delete"] = json!(true);
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
            assert!(!state.preferences.confirm_delete);
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
        assert!(!runtime.state.recovered_inputs.contains_key(&1));
        assert!(runtime.state.current().native.queue_held);
        assert_eq!(
            runtime.state.current().native.queue[0].prompt.text,
            "queued text"
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
            id: uuid::Uuid::new_v4().to_string(),
            role: "Pi",
            text: "durable server reply".into(),
            images: vec![],
            context: None,
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

    #[test]
    fn retargeted_thread_pill_keeps_owner_and_blocks_the_send() {
        let mut state = State::new(PathBuf::new(), false);
        let (mut client, _rx) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote_client, _rx2) = crate::client::Client::test_connected(state.data_dir.clone());
        client.linked.insert("server-b".into(), remote_client);
        state.client = Some(client);

        let mut local_thread_7 = Thread::new(7);
        local_thread_7.machine = "local".into();
        local_thread_7.title = "Local seven".into();
        let mut local_draft = Thread::new(1);
        local_draft.machine = "local".into();
        local_draft.client_draft = true;

        let mut remote_thread_7 = Thread::new(7);
        remote_thread_7.machine = "server-b".into();
        remote_thread_7.title = "Remote seven".into();
        let mut remote_draft = Thread::new(1);
        remote_draft.machine = "server-b".into();
        remote_draft.client_draft = true;

        state.threads = vec![local_draft, local_thread_7];
        state
            .machines
            .threads
            .insert("server-b".into(), vec![remote_draft, remote_thread_7]);
        state.peers.push(crate::peer::Peer {
            server_id: "server-b".into(),
            host: "studio.local".into(),
            label: "Studio".into(),
            linked_at: 0,
            capabilities: json!({}),
            last_reached_at: None,
            last_error: None,
        });

        let token = crate::composer_tokens::Token::Thread {
            id: 7,
            title: "Local seven".into(),
            machine: Some("local".into()),
            extra: Default::default(),
        };
        let mut draft = crate::composer_rich::draft("check @loc");
        draft.replace_with_pill(6..10, token.id(), "Local seven");
        state.current_mut().draft = draft;

        let same_machine_prompt = crate::composer_tokens::prompt_for(
            &state.current().draft,
            &[],
            Some(&state.current().machine),
            &|machine| state.machine_label(machine),
        )
        .unwrap();
        assert_eq!(
            same_machine_prompt.text,
            "check Shina thread \"Local seven\", read_thread({\"thread_id\":7})"
        );

        state.retarget_draft("server-b");
        assert_eq!(state.machines.selected, "server-b");
        let blocked = crate::composer_tokens::prompt_for(
            &state.current().draft,
            &[],
            Some(&state.current().machine),
            &|machine| state.machine_label(machine),
        )
        .err()
        .expect("a thread on another machine blocks the send");
        assert!(
            blocked.contains("Attached thread is on another machine")
                && blocked.contains("Local seven")
                && blocked.contains("not linked")
        );
    }

    #[test]
    fn older_peer_without_composer_context_receives_preprojected_text_without_context_field() {
        let mut rec = crate::composer_context::thread_context_record("local", 5, "Design doc");
        rec.label = "Design doc".into();
        let command = Command::new(
            "thread.send",
            json!({
                "threadId": 1,
                "text": "See [Design doc](t3-context://v1/thread/thread_5)",
                "attachments": [],
                "context": { "version": 1, "records": [rec] },
                "draft": { "text": "See Design doc", "html": "<p>See <span data-pill=\"t3-context://v1/thread/thread_5\">Design doc</span></p>", "attachments": [] },
                "mode": "prompt",
            }),
        );
        let downgraded = crate::client::downgrade_composer_context_command(command.clone(), false);
        assert!(downgraded.params.get("context").is_none());
        assert!(downgraded.params.get("draft").is_none());
        let projected_text = downgraded.params["text"].as_str().unwrap();
        assert!(projected_text.contains("[Thread: Design doc; ref=thread_5]"));
        assert!(projected_text.contains("<t3_context version=\"1\">"));
        // Verify it deserializes into older SendInput with deny_unknown_fields.
        assert!(serde_json::from_value::<crate::protocol::SendInput>(downgraded.params).is_ok());

        // Newer peer keeps structured context intact.
        let kept = crate::client::downgrade_composer_context_command(command, true);
        assert!(kept.params.get("context").is_some());
        assert!(kept.params.get("draft").is_some());
    }

    #[test]
    fn clipboard_paste_fetches_image_asset_from_linked_server_and_shows_unresolved_when_missing() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let mut png_cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([10, 20, 30, 255]),
        ))
        .write_to(&mut png_cursor, image::ImageFormat::Png)
        .unwrap();
        let png_bytes = png_cursor.into_inner();
        let ok_blob_id = "remote-shot.png";

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let served_bytes = png_bytes.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten().take(4) {
                let mut stream = stream;
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                if req.contains(&format!("/v1/blobs/{ok_blob_id}")) {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        served_bytes.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&served_bytes);
                } else {
                    let _ = stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        });

        let mut state = State::new(PathBuf::new(), false);
        let source_server_id = uuid::Uuid::new_v4().to_string();
        let (mut client, _rx) = crate::client::Client::test_connected(state.data_dir.clone());
        let (mut source_client, _rx2) =
            crate::client::Client::test_connected(state.data_dir.clone());
        source_client.remote = Some(crate::peer::Link {
            server_id: source_server_id.clone(),
            host: format!("127.0.0.1:{port}"),
            device_id: uuid::Uuid::new_v4().to_string(),
            token: "test-token".into(),
            enabled: true,
        });
        client
            .linked
            .insert(source_server_id.clone(), source_client);
        state.client = Some(client);

        let fragment = crate::composer_context::ComposerContextClipboardFragment {
            version: 1,
            source: crate::composer_context::ClipboardSource {
                environment_id: source_server_id,
                thread_id: Some("1".into()),
                message_id: None,
            },
            records: vec![
                crate::composer_context::ComposerContextRecord {
                    version: 1,
                    context_id: "image_ok".into(),
                    label: "remote.png".into(),
                    data: crate::composer_context::ComposerContextData::Image {
                        attachment_id: ok_blob_id.into(),
                        name: "remote.png".into(),
                        mime_type: "image/png".into(),
                        size_bytes: png_bytes.len() as u64,
                    },
                },
                crate::composer_context::ComposerContextRecord {
                    version: 1,
                    context_id: "image_missing".into(),
                    label: "missing.png".into(),
                    data: crate::composer_context::ComposerContextData::Image {
                        attachment_id: "missing-blob.png".into(),
                        name: "missing.png".into(),
                        mime_type: "image/png".into(),
                        size_bytes: 10,
                    },
                },
            ],
        };
        let encoded = crate::composer_context::encode_composer_context_fragment(&fragment).unwrap();
        let md = "![remote.png](t3-context://v1/image/image_ok) ![missing.png](t3-context://v1/image/image_missing)";
        let html =
            crate::composer_context::encode_composer_context_clipboard_html(md, &encoded, None);
        assert!(
            state.paste_composer_content(&rune::ClipboardContent::new(md).with_html(html), false)
        );

        // While downloads are pending, send is blocked cleanly.
        assert!(state.has_pending_context_imports("local", state.current().id));
        state.send();
        assert!(
            state
                .current()
                .attachment_error
                .as_deref()
                .unwrap()
                .contains("Still bringing attached context into this message")
        );

        let deadline = Instant::now() + Duration::from_secs(5);
        while state.has_pending_context_imports("local", state.current().id) {
            state.poll_context_imports();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }

        // Valid image resolved with rewritten contextId; missing image stays unresolved with error.
        assert_eq!(state.current().draft_images.len(), 1);
        assert_eq!(state.current().draft_images[0].name, "remote.png");
        assert_eq!(state.current().draft.pills().count(), 2);
        assert!(
            state
                .current()
                .attachment_error
                .as_deref()
                .unwrap()
                .contains("Couldn't bring missing.png into this message")
        );
        assert_eq!(state.current().draft_context.len(), 1);
        assert_ne!(state.current().draft_context[0].context_id, "image_ok");
    }

    #[test]
    fn linked_machine_thread_pill_allows_thread_read_v2_and_names_missing_link_or_capability() {
        let mut state = State::new(PathBuf::new(), false);
        let (mut client, _rx) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote_client, _rx2) = crate::client::Client::test_connected(state.data_dir.clone());
        client.linked.insert("server-b".into(), remote_client);
        state.client = Some(client);
        state.peers = vec![
            crate::peer::Peer {
                server_id: "server-b".into(),
                host: "studio.local".into(),
                label: "Studio".into(),
                linked_at: 0,
                capabilities: json!({"contracts": [crate::protocol::THREAD_READ]}),
                last_reached_at: None,
                last_error: None,
            },
            crate::peer::Peer {
                server_id: "server-old".into(),
                host: "old.local".into(),
                label: "OldBox".into(),
                linked_at: 0,
                capabilities: json!({"contracts": []}),
                last_reached_at: None,
                last_error: None,
            },
        ];

        // 1. Linked peer WITH thread-read-v2 succeeds and expands machine + threadId.
        let rec_b = crate::composer_context::thread_context_record("server-b", 7, "Remote seven");
        state.current_mut().draft = crate::composer_rich::draft("check ");
        state.current_mut().draft.set_caret(6);
        state.current_mut().draft.insert_pill(
            crate::composer_context::format_composer_context_href("thread", &rec_b.context_id),
            "Remote seven",
        );
        state.current_mut().draft_context = vec![rec_b];
        let prompt = crate::composer_tokens::prompt_for_target(
            &state.current().draft,
            &[],
            &state.current().draft_context,
            Some("local"),
            None,
            &state.peers,
            &|m| state.machine_label(m),
        )
        .unwrap();
        let expanded = crate::attachments::provider_text(&prompt);
        assert!(
            expanded.contains(r#"read_thread({"machine":"server-b","threadId":7})"#)
                || expanded.contains(r#"read_thread({"threadId":7,"machine":"server-b"})"#),
            "{expanded}"
        );

        // 2. Linked peer WITHOUT thread-read-v2 blocks and names the missing capability.
        let rec_old = crate::composer_context::thread_context_record("server-old", 8, "Old seven");
        state.current_mut().draft = crate::composer_rich::draft("check ");
        state.current_mut().draft.set_caret(6);
        state.current_mut().draft.insert_pill(
            crate::composer_context::format_composer_context_href("thread", &rec_old.context_id),
            "Old seven",
        );
        state.current_mut().draft_context = vec![rec_old];
        let err_cap = crate::composer_tokens::prompt_for_target(
            &state.current().draft,
            &[],
            &state.current().draft_context,
            Some("local"),
            None,
            &state.peers,
            &|m| state.machine_label(m),
        )
        .unwrap_err();
        assert!(err_cap.contains("thread-read-v2"), "{err_cap}");

        // 3. Unlinked machine blocks and says it is not linked.
        let rec_unlinked =
            crate::composer_context::thread_context_record("server-c", 9, "Stranger");
        state.current_mut().draft = crate::composer_rich::draft("check ");
        state.current_mut().draft.set_caret(6);
        state.current_mut().draft.insert_pill(
            crate::composer_context::format_composer_context_href(
                "thread",
                &rec_unlinked.context_id,
            ),
            "Stranger",
        );
        state.current_mut().draft_context = vec![rec_unlinked];
        let err_unlinked = crate::composer_tokens::prompt_for_target(
            &state.current().draft,
            &[],
            &state.current().draft_context,
            Some("local"),
            None,
            &state.peers,
            &|m| state.machine_label(m),
        )
        .unwrap_err();
        assert!(err_unlinked.contains("not linked"), "{err_unlinked}");
    }

    #[test]
    fn foreground_and_background_sends_guard_context_and_restore_full_snapshots() {
        for background in [false, true] {
            for new_thread in [false, true] {
                let (mut state, server, mut commands) = tab_client();
                state.apply_projection(server.projection()).unwrap();
                if new_thread {
                    state.new_thread_in(Some(1));
                }
                let source_id = state.current().id;
                let record =
                    crate::composer_context::thread_context_record("local", 1, "Reference");
                let text = format!(
                    "Read [Reference]({})",
                    crate::composer_context::format_composer_context_href(
                        "thread",
                        &record.context_id
                    )
                );
                state.current_mut().draft = crate::composer_rich::draft_from_canonical(&text);
                state.current_mut().draft_context = vec![record.clone()];
                let send = |state: &mut State| {
                    if background {
                        state.send_background()
                    } else {
                        state.send()
                    }
                };
                // A pending import blocks both dispatch paths before promotion/navigation.
                state
                    .pending_context_imports
                    .push(("local".into(), source_id, "pending".into()));
                send(&mut state);
                assert_eq!(state.current().id, source_id);
                assert!(commands.try_recv().is_err());
                state.pending_context_imports.clear();
                // An unlinked thread blocks both paths before thread.create as well.
                state.current_mut().draft_context[0] =
                    crate::composer_context::thread_context_record("unlinked", 1, "Reference");
                send(&mut state);
                assert_eq!(state.current().id, source_id);
                assert!(commands.try_recv().is_err());
                assert!(
                    state
                        .current()
                        .attachment_error
                        .as_deref()
                        .unwrap()
                        .contains("not linked")
                );
                state.current_mut().draft_context = vec![record.clone()];
                send(&mut state);
                let command = commands.try_recv().unwrap();
                let rejected = json!({"result":{"status":"rejected","result":{"error":"Refused"}}});
                if new_thread {
                    assert_eq!(command.method, "thread.create");
                    state.complete_client_creation("local", &command, &rejected);
                } else {
                    assert_eq!(command.method, "thread.send");
                    state.machines.receipts.insert(
                        "local".into(),
                        vec![crate::client::Event::Receipt(command, rejected)],
                    );
                    state.poll_client(&mut Tx::default());
                }
                let source = state.threads.iter().find(|t| t.id == source_id).unwrap();
                assert_eq!(crate::composer_rich::markdown(&source.draft), text);
                assert_eq!(source.draft_context, vec![record]);
                assert!(source.remote_send.is_none());
            }
        }
    }

    #[test]
    fn remote_send_background_promotes_opens_fresh_draft_and_handles_success_failure_and_offline() {
        // 1. Offline machine shows warning toast and does not send or open a new draft.
        {
            let mut state = State::new(PathBuf::new(), false);
            let mut client = crate::client::Client::new(state.data_dir.clone());
            client.linked.insert(
                "offline".into(),
                crate::client::Client::new(state.data_dir.clone()),
            );
            state.client = Some(client);
            state.machines.selected = "offline".into();
            state.current_mut().client_draft = true;
            state.current_mut().draft.replace_text("Should not send");
            let id = state.current().id;
            state.send_background();
            assert_eq!(state.current().id, id);
            assert_eq!(state.current().draft.text, "Should not send");
            assert_eq!(state.toasts.len(), 1);
            assert_eq!(state.toasts[0].kind, ToastKind::Warning);
            assert_eq!(state.toasts[0].title, "Not connected: message not sent");
            assert_eq!(
                state.toasts[0].description.as_deref(),
                Some("Reconnecting to the environment. Try again once it is connected.")
            );
        }

        // 2. Remote draft sendBackground: promotes in background, stays visible in sidebar while launching,
        // opens fresh draft immediately, and shows "Started in background" once accepted.
        {
            let (mut state, mut server, mut commands) = tab_client();
            state.apply_projection(server.projection()).unwrap();
            state.new_thread_in(Some(1));
            let draft_id = state.current().id;
            state.current_mut().draft.replace_text("Remote bg task");
            state.send_background();

            let fresh_id = state.current().id;
            assert_ne!(fresh_id, draft_id);
            assert!(state.current().client_draft);
            assert!(state.current().draft.text.is_empty());
            assert!(
                state
                    .sidebar_threads()
                    .iter()
                    .any(|&i| state.threads[i].id == draft_id),
                "launching background draft remains in sidebar"
            );

            let create = commands.try_recv().unwrap();
            assert_eq!(create.method, "thread.create");
            let created = server.execute_command(&create).unwrap();
            let server_id = created["threadId"].as_u64().unwrap() as usize;
            state.complete_client_creation("local", &create, &accepted(created));
            let send = commands.try_recv().unwrap();
            assert_eq!(send.method, "thread.send");
            state.apply_projection(server.projection()).unwrap();
            state.finish_client_promotions();
            assert_eq!(state.current().id, fresh_id, "stays on fresh composer");

            state.machines.receipts.insert(
                "local".into(),
                vec![crate::client::Event::Receipt(
                    send,
                    json!({"result":{"status":"accepted","result":{}}}),
                )],
            );
            state.poll_client(&mut Tx::default());
            assert_eq!(state.current().id, fresh_id);
            assert_eq!(state.toasts.len(), 1);
            assert_eq!(state.toasts[0].kind, ToastKind::Success);
            assert_eq!(state.toasts[0].title, "Started in background");
            assert_eq!(state.toasts[0].action_label.as_deref(), Some("Open"));

            let toast_id = state.toasts[0].id;
            state.activate_toast(toast_id);
            assert_eq!(state.current().id, server_id);
        }

        // 3. Remote background send failure on thread.create restores draft and shows "Background task failed".
        {
            let (mut state, server, mut commands) = tab_client();
            state.apply_projection(server.projection()).unwrap();
            state.new_thread_in(Some(1));
            let draft_id = state.current().id;
            state.current_mut().draft.replace_text("Will fail create");
            state.send_background();
            let fresh_id = state.current().id;
            assert_ne!(fresh_id, draft_id);

            let create = commands.try_recv().unwrap();
            state.complete_client_creation(
                "local",
                &create,
                &json!({"result":{"status":"rejected","result":{"error":"Create refused"}}}),
            );
            assert_eq!(state.current().id, fresh_id);
            assert_eq!(state.toasts.len(), 1);
            assert_eq!(state.toasts[0].kind, ToastKind::Error);
            assert_eq!(state.toasts[0].title, "Background task failed");
            assert_eq!(
                state.toasts[0].description.as_deref(),
                Some("Create refused")
            );
            assert_eq!(state.toasts[0].action_label.as_deref(), Some("Open draft"));
            let toast_id = state.toasts[0].id;
            state.activate_toast(toast_id);
            assert_eq!(state.current().id, draft_id);
            assert_eq!(state.current().draft.text, "Will fail create");
            assert_eq!(state.current().error.as_deref(), Some("Create refused"));
        }

        // 4. Remote sendAndNewThread on existing thread waits for accepted receipt, and refuses to switch on failure.
        {
            let (mut state, mut server, mut commands) = tab_client();
            server.threads[0].messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "You",
                text: "Prior turn".into(),
                images: vec![],
                context: None,
            });
            state.apply_projection(server.projection()).unwrap();
            state.select_thread(0);
            while commands.try_recv().is_ok() {}
            let existing_id = state.current().id;
            assert!(!state.current().is_draft());
            state
                .current_mut()
                .draft
                .replace_text("Existing remote send");
            state.send_background();
            // Still on existing thread while receipt is pending.
            assert_eq!(state.current().id, existing_id);
            let send = commands.try_recv().unwrap();
            assert_eq!(send.method, "thread.send");

            // Rejected receipt restores draft and stays on existing thread.
            state.machines.receipts.insert(
                "local".into(),
                vec![crate::client::Event::Receipt(
                    send,
                    json!({"result":{"status":"rejected","result":{"error":"Send refused"}}}),
                )],
            );
            state.poll_client(&mut Tx::default());
            assert_eq!(state.current().id, existing_id);
            assert_eq!(state.current().draft.text, "Existing remote send");
            assert_eq!(state.current().error.as_deref(), Some("Send refused"));

            // Accepted receipt opens a fresh draft in the same project.
            state.send_background();
            let send2 = commands.try_recv().unwrap();
            state.machines.receipts.insert(
                "local".into(),
                vec![crate::client::Event::Receipt(
                    send2,
                    json!({"result":{"status":"accepted","result":{}}}),
                )],
            );
            state.poll_client(&mut Tx::default());
            assert_ne!(state.current().id, existing_id);
            assert!(state.current().client_draft);
            assert!(state.toasts.is_empty());
        }
    }

    #[test]
    fn model_favorites_negotiates_capability_and_falls_back_to_client_preferences_when_peer_lacks_it()
     {
        let (mut state, mut server, mut commands) = tab_client();
        // When peer lacks MODEL_FAVORITES capability, favorites live locally in Preferences.
        assert!(!state.supports_model_favorites());
        state.toggle_model_favorite("codex", "gpt-5.6-sol");
        assert!(commands.try_recv().is_err());
        assert_eq!(
            state.effective_model_favorites(),
            &[crate::model_picker::ModelFavorite {
                provider: "codex".into(),
                model: "gpt-5.6-sol".into(),
            }]
        );
        // Persistence roundtrip in Preferences (ui_snapshot)
        let ui = state.ui_snapshot();
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_ui(&ui).unwrap();
        assert_eq!(restored.preferences.model_favorites.len(), 1);

        // When peer advertises MODEL_FAVORITES capability, favorites migrate and sync via settings.modelFavorites.
        *state.client.as_ref().unwrap().info.lock().unwrap() =
            json!({"capabilities": [crate::protocol::MODEL_FAVORITES]});
        assert!(state.supports_model_favorites());
        state.apply_projection(server.projection()).unwrap();
        let migrate = commands.try_recv().unwrap();
        assert_eq!(migrate.method, "settings.modelFavorites");
        server.execute_command(&migrate).unwrap();
        assert_eq!(server.model_favorites.as_ref().unwrap().len(), 1);

        state.toggle_model_favorite("pi", "cpa/opus");
        let add_cmd = commands.try_recv().unwrap();
        assert_eq!(add_cmd.method, "settings.modelFavorites");
        assert_eq!(add_cmd.params["provider"], "pi");
        assert_eq!(add_cmd.params["model"], "cpa/opus");
        assert_eq!(add_cmd.params["favorite"], true);
        server.execute_command(&add_cmd).unwrap();
        state.apply_projection(server.projection()).unwrap();
        assert_eq!(state.effective_model_favorites().len(), 2);
    }

    #[test]
    fn catalog_refresh_on_open_gates_on_capability_and_sends_model_catalog_only() {
        let (mut state, mut server, mut commands) = tab_client();
        // Older peer without MODEL_CATALOG_REFRESH: opening picker skips modelCatalogOnly RPC.
        assert!(!state.supports_model_catalog_refresh());
        state.open_model_picker(SettingsPicker::Model);
        assert!(commands.try_recv().is_err());

        // Capable peer: opening picker sends catalog.refresh { modelCatalogOnly: true }.
        *state.client.as_ref().unwrap().info.lock().unwrap() =
            json!({"capabilities": [crate::protocol::MODEL_CATALOG_REFRESH]});
        assert!(state.supports_model_catalog_refresh());
        state.open_model_picker(SettingsPicker::Model);
        let on_open = commands.try_recv().unwrap();
        assert_eq!(on_open.method, "catalog.refresh");
        assert_eq!(on_open.params["modelCatalogOnly"], true);
        server.execute_command(&on_open).unwrap();

        // Explicit retry-catalog sends catalog.refresh { force: true } (not modelCatalogOnly).
        state.force_refresh_catalog();
        let retry = commands.try_recv().unwrap();
        assert_eq!(retry.method, "catalog.refresh");
        assert_eq!(retry.params["force"], true);
        assert!(retry.params.get("modelCatalogOnly").is_none());
    }

    #[test]
    fn model_options_gate_on_capability_and_fall_back_for_older_peers() {
        let (mut state, mut server, mut commands) = tab_client();
        state.select_thread(0);
        while commands.try_recv().is_ok() {}
        server.threads[0].harness = "claude".into();
        server.threads[0].models = vec![crate::session::AgentModel {
            provider: "claude".into(),
            id: "claude-opus-5-5".into(),
            name: "Claude Opus 5.5".into(),
            thinking_levels: vec!["low".into(), "medium".into(), "high".into(), "max".into()],
            capabilities: crate::descriptors::claude_manifest_model_entry("claude-opus-5-5")
                .map(|m| m.capabilities)
                .unwrap_or_default(),
            ..Default::default()
        }];
        server.threads[0].set_model("claude".into(), "claude-opus-5-5".into());
        state.apply_projection(server.projection()).unwrap();

        // Older peer without MODEL_OPTIONS falls back to `thinking` / `serviceTier` and refuses descriptor-only options.
        assert!(!state.supports_model_options());
        state.set_composer_option_sticky(
            "effort".into(),
            crate::model_selection::OptionValue::Str("high".into()),
        );
        let fallback_effort = commands.try_recv().unwrap();
        assert_eq!(fallback_effort.method, "thread.configure");
        assert_eq!(fallback_effort.params["thinking"], "high");
        assert!(fallback_effort.params.get("options").is_none());

        state.set_composer_option_sticky(
            "serviceTier".into(),
            crate::model_selection::OptionValue::Str("fast".into()),
        );
        let fallback_tier = commands.try_recv().unwrap();
        assert_eq!(fallback_tier.method, "thread.configure");
        assert_eq!(fallback_tier.params["serviceTier"], "fast");

        state.set_composer_option_sticky(
            "fastMode".into(),
            crate::model_selection::OptionValue::Bool(true),
        );
        assert!(commands.try_recv().is_err());
        assert!(state.current().error.is_some());

        // Capable peer advertising MODEL_OPTIONS forwards descriptor `options` via thread.configure.
        state.current_mut().error = None;
        *state.client.as_ref().unwrap().info.lock().unwrap() =
            json!({"capabilities": [crate::protocol::MODEL_OPTIONS]});
        assert!(state.supports_model_options());
        state.set_composer_option_sticky(
            "fastMode".into(),
            crate::model_selection::OptionValue::Bool(true),
        );
        let cmd = commands.try_recv().unwrap();
        assert_eq!(cmd.method, "thread.configure");
        assert_eq!(cmd.params["options"][0]["id"], "fastMode");
        assert_eq!(cmd.params["options"][0]["value"], true);
        server.execute_command(&cmd).unwrap();
        assert_eq!(
            server.threads[0].model_options,
            vec![crate::model_selection::ProviderOptionSelection {
                id: "fastMode".into(),
                value: crate::model_selection::OptionValue::Bool(true),
            }]
        );
    }

    #[test]
    fn retry_failure_notifies_could_not_reconnect_environment_and_shows_diagnosis() {
        let dir = std::env::temp_dir().join(format!("shina-retry-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::headless(dir.clone());
        let (mut client, _) = crate::client::Client::test_connected(dir.clone());
        let remote = crate::client::Client::new(dir);
        remote.set_test_connection_state(crate::client::ConnectionState {
            phase: crate::client::ConnectionPhase::Error,
            attempt: 1,
            attempt_seq: 1,
            last_failure: Some(crate::client::FailureKind::AuthDenied),
            last_error: Some("Authentication denied.".into()),
        });
        client.linked.insert("studio".into(), remote);
        state.client = Some(client);
        state.machines.selected = "studio".into();
        state.tick_composer_notices_at(Instant::now(), None);

        let notices = state.composer_notices.ordered();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].title, "studio is offline");
        assert_eq!(
            notices[0].description.as_deref(),
            Some("Shina on studio did not accept this device")
        );

        state.retry_machine("studio");
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].kind, ToastKind::Error);
        assert_eq!(state.toasts[0].title, "Could not reconnect environment");
        assert_eq!(
            state.toasts[0].description.as_deref(),
            Some("Authentication denied.")
        );
    }

    #[test]
    fn disconnect_disables_persists_hides_threads_returns_home_and_retry_reenables() {
        let dir =
            std::env::temp_dir().join(format!("shina-disconnect-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let server_id = uuid::Uuid::new_v4().to_string();
        let device_id = uuid::Uuid::new_v4().to_string();
        let link = crate::peer::Link {
            server_id: server_id.clone(),
            host: "127.0.0.1:9".into(),
            device_id,
            token: "test-token".into(),
            enabled: true,
        };
        link.save(&dir).unwrap();

        let mut state = State::headless(dir.clone());
        let (mut client, _) = crate::client::Client::test_connected(dir.clone());
        let mut remote = crate::client::Client::new(dir.clone());
        remote.remote = Some(link);
        client.linked.insert(server_id.clone(), remote);
        state.client = Some(client);

        let mut remote_proj = State::headless(dir.clone());
        remote_proj.threads[0].title = "Remote thread".into();
        remote_proj.threads[0].messages.push(Message {
            id: "m1".into(),
            role: "You",
            text: "hi".into(),
            images: vec![],
            context: None,
        });
        state
            .machines
            .views
            .insert(server_id.clone(), remote_proj.projection());
        state.open_machine_thread(&server_id, 1);
        assert_eq!(state.machines.selected, server_id);
        assert!(!state.current().messages.is_empty());

        // Disconnect server disables the machine, hides its threads, and returns to the home screen.
        state.disconnect_server(&server_id);
        assert!(!state.machine_enabled(&server_id));
        assert!(!state.machines.views.contains_key(&server_id));
        assert_eq!(state.machines.selected, "local");
        assert!(state.current().messages.is_empty());

        // The enabled flag survives a save and reload using the temporary data directory.
        let reloaded = crate::peer::Link::load_all(&dir).unwrap();
        assert_eq!(reloaded.len(), 1);
        assert!(!reloaded[0].enabled);

        // Retry on a disabled machine re-enables it first and persists enabled = true.
        state.retry_machine(&server_id);
        assert!(state.machine_enabled(&server_id));
        let reloaded = crate::peer::Link::load_all(&dir).unwrap();
        assert!(reloaded[0].enabled);
        state
            .client
            .as_mut()
            .unwrap()
            .linked
            .get_mut(&server_id)
            .unwrap()
            .shutdown();

        // Persistence failure notifies "Could not disconnect server".
        let bad_dir =
            std::env::temp_dir().join(format!("shina-disconnect-bad-{}", uuid::Uuid::new_v4()));
        std::fs::write(&bad_dir, b"not-a-directory").unwrap();
        let mut bad_state = State::headless(bad_dir.clone());
        let (mut bad_client, _) = crate::client::Client::test_connected(bad_dir.clone());
        let mut bad_remote = crate::client::Client::new(bad_dir.clone());
        bad_remote.remote = Some(crate::peer::Link {
            server_id: server_id.clone(),
            host: "127.0.0.1:9".into(),
            device_id: uuid::Uuid::new_v4().to_string(),
            token: "test-token".into(),
            enabled: true,
        });
        bad_client.linked.insert(server_id.clone(), bad_remote);
        bad_state.client = Some(bad_client);
        bad_state.machines.selected = server_id.clone();
        bad_state.disconnect_server(&server_id);
        assert_eq!(bad_state.machines.selected, server_id);
        assert!(bad_state.machine_enabled(&server_id));
        assert_eq!(bad_state.toasts.len(), 1);
        assert_eq!(bad_state.toasts[0].kind, ToastKind::Error);
        assert_eq!(bad_state.toasts[0].title, "Could not disconnect server");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&bad_dir);
    }
}
