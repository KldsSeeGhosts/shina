use crate::backend::{Event, Pi};
use crate::captains::{Captain, CaptainEditor};
use rune::{TextEdit, Tx, Waker};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Instant};

/// The label shown when a thread uses whatever model Pi is configured with.
pub const DEFAULT_MODEL: &str = "Default model";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Offline,
    Connecting,
    Ready,
    Running,
    Waiting,
    Settled,
    Failed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Offline => "Offline",
            Self::Connecting => "Connecting",
            Self::Ready => "Ready",
            Self::Running => "Working",
            Self::Waiting => "Needs you",
            Self::Settled => "Settled",
            Self::Failed => "Error",
        }
    }
    pub fn busy(self) -> bool {
        matches!(self, Self::Connecting | Self::Running | Self::Waiting)
    }
    /// Which state a captain's portrait should show when its threads disagree. Attention
    /// outranks work, work outranks a ready conversation, and an empty crew is offline.
    pub fn urgency(self) -> u8 {
        match self {
            Self::Failed => 6,
            Self::Waiting => 5,
            Self::Running => 4,
            Self::Connecting => 3,
            Self::Ready => 2,
            Self::Settled => 1,
            Self::Offline => 0,
        }
    }
}

pub struct Message {
    pub role: &'static str,
    pub text: String,
}

pub struct Tool {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub done: bool,
    pub failed: bool,
    pub output: String,
}

#[derive(Clone, Debug)]
pub struct AgentModel {
    pub provider: String,
    pub id: String,
    pub name: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsPicker {
    Model,
    Thinking,
}

pub struct Thread {
    pub id: usize,
    pub title: String,
    pub status: Status,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub draft: TextEdit,
    pub pi: Option<Pi>,
    pub model: String,
    pub thinking: String,
    pub model_provider: String,
    pub model_id: String,
    pub models: Vec<AgentModel>,
    pub thinking_levels: Vec<String>,
    pub configuring: bool,
    pub phase: String,
    pub error: Option<String>,
    pub diagnostics: Vec<String>,
    pub request: Option<Value>,
    pub reply: TextEdit,
    pub session: Option<String>,
    pub archived: bool,
    pub unread: bool,
    pub expanded_tools: bool,
    pub started: Option<Instant>,
    pub elapsed: Option<u64>,
    pub captain: Option<usize>,
    pub captain_home: bool,
    pub pinned: bool,
    assistant: Option<usize>,
    serial: usize,
    state_loaded: bool,
    history_loaded: bool,
    send_when_ready: bool,
    pending_thinking: Option<String>,
    pending_prompt: Option<(String, String, usize)>,
}

impl Thread {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            title: "New thread".into(),
            status: Status::Offline,
            messages: vec![],
            tools: vec![],
            draft: TextEdit::default(),
            pi: None,
            model: DEFAULT_MODEL.into(),
            thinking: String::new(),
            model_provider: String::new(),
            model_id: String::new(),
            models: vec![],
            thinking_levels: vec![],
            configuring: false,
            phase: String::new(),
            error: None,
            diagnostics: vec![],
            request: None,
            reply: TextEdit::default(),
            session: None,
            archived: false,
            unread: false,
            expanded_tools: false,
            started: None,
            elapsed: None,
            captain: None,
            captain_home: false,
            pinned: false,
            assistant: None,
            serial: 0,
            state_loaded: false,
            history_loaded: false,
            send_when_ready: false,
            pending_thinking: None,
            pending_prompt: None,
        }
    }

    pub fn connect(&mut self, path: PathBuf, wake: Waker) {
        self.connect_with_instructions(path, wake, None);
    }

    pub fn connect_with_instructions(
        &mut self,
        path: PathBuf,
        wake: Waker,
        instructions: Option<String>,
    ) {
        if self.pi.is_some() {
            return;
        }
        self.error = None;
        self.status = Status::Connecting;
        self.state_loaded = false;
        self.history_loaded = false;
        self.models.clear();
        self.thinking_levels.clear();
        self.pi = Some(match instructions {
            Some(instructions) => {
                Pi::start_with_instructions(path, self.session.clone(), wake, Some(instructions))
            }
            None => Pi::start(path, self.session.clone(), wake),
        });
    }

    pub fn set_model(&mut self, provider: String, model_id: String) {
        if provider != "cpa" {
            self.error = Some(
                "Shina uses CPA proxy models only. Choose a model from the CPA catalog.".into(),
            );
            return;
        }
        if !self.status.busy()
            && !self.configuring
            && self.command("set_model", json!({"provider":provider,"modelId":model_id}))
        {
            self.configuring = true;
        }
    }

    pub fn set_thinking(&mut self, level: String) {
        if !self.status.busy()
            && !self.configuring
            && self.command("set_thinking_level", json!({"level":level}))
        {
            self.configuring = true;
            self.pending_thinking = Some(level);
        }
    }

    pub fn command(&mut self, kind: &str, mut data: Value) -> bool {
        self.serial += 1;
        data["type"] = kind.into();
        data["id"] = format!("{}-{}", self.id, self.serial).into();
        let result = self
            .pi
            .as_ref()
            .ok_or_else(|| "Connect Pi before sending a message.".to_owned())
            .and_then(|pi| pi.send(data));
        if let Err(error) = result {
            self.error = Some(error);
            return false;
        }
        true
    }

    pub fn send(&mut self) {
        if self.status.busy()
            || self.configuring
            || self.pi.is_none()
            || self.draft.text.trim().is_empty()
        {
            return;
        }
        if self.model_provider != "cpa" {
            self.error = Some("Choose a CPA model before sending. Shina will not send prompts to a direct provider.".into());
            return;
        }
        let prompt = self.draft.text.trim().to_owned();
        if !self.command("prompt", json!({"message":prompt})) {
            return;
        }
        self.pending_prompt = Some((
            format!("{}-{}", self.id, self.serial),
            prompt.clone(),
            self.messages.len(),
        ));
        if self.messages.is_empty() && !self.captain_home {
            self.title = prompt.chars().take(45).collect();
        }
        self.messages.push(Message {
            role: "You",
            text: prompt,
        });
        self.draft.replace_text("");
        self.status = Status::Running;
        self.started = Some(Instant::now());
        self.elapsed = None;
        self.assistant = None;
        self.error = None;
        self.phase = "Waiting for Pi".into();
    }

    pub fn stop(&mut self) {
        if self.status == Status::Connecting {
            self.disconnect();
            return;
        }
        if self.status == Status::Running || self.status == Status::Waiting {
            // Release a blocking extension dialog before asking Pi to wait for idle.
            if self.request.is_some() {
                self.respond(json!({"cancelled":true}));
            }
            self.command("clear_queue", json!({}));
            self.command("abort", json!({}));
            self.phase = "Stopping".into();
        }
    }

    pub fn respond(&mut self, mut response: Value) {
        let Some(request) = &self.request else {
            return;
        };
        response["type"] = "extension_ui_response".into();
        response["id"] = request["id"].clone();
        let result = self
            .pi
            .as_ref()
            .ok_or_else(|| "Pi is disconnected.".to_owned())
            .and_then(|pi| pi.send(response));
        match result {
            Ok(()) => {
                self.request = None;
                self.status = Status::Running;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn disconnect(&mut self) {
        if let Some((_, prompt, _)) = self.pending_prompt.take() {
            self.restore_prompt(&prompt);
        }
        self.pi = None;
        self.status = Status::Offline;
        self.assistant = None;
        self.request = None;
        self.send_when_ready = false;
        self.configuring = false;
        self.pending_thinking = None;
        self.phase.clear();
    }

    fn assistant(&mut self) -> &mut Message {
        let index = *self.assistant.get_or_insert_with(|| {
            self.messages.push(Message {
                role: "Pi",
                text: String::new(),
            });
            self.messages.len() - 1
        });
        &mut self.messages[index]
    }

    pub fn apply(&mut self, record: Value) {
        match string(&record, "type") {
            "response" => {
                let command = string(&record, "command");
                if record["success"] == false {
                    self.error = Some(string(&record, "error").to_owned());
                    if matches!(command, "set_model" | "set_thinking_level") {
                        self.configuring = false;
                        self.pending_thinking = None;
                    }
                    if command == "prompt"
                        && self
                            .pending_prompt
                            .as_ref()
                            .is_some_and(|(id, _, _)| record["id"] == *id)
                        && let Some((_, prompt, index)) = self.pending_prompt.take()
                    {
                        self.restore_prompt(&prompt);
                        if self
                            .messages
                            .get(index)
                            .is_some_and(|m| m.role == "You" && m.text == prompt)
                        {
                            self.messages.remove(index);
                        }
                    }
                    if matches!(command, "prompt" | "get_state" | "get_messages") {
                        self.status = Status::Failed;
                        self.send_when_ready = false;
                        self.started = None;
                    }
                    return;
                }
                match command {
                    "get_state" => {
                        let data = &record["data"];
                        self.update_model(&data["model"]);
                        self.thinking = string(data, "thinkingLevel").to_owned();
                        self.session = data["sessionFile"].as_str().map(str::to_owned);
                        if record["id"] == "state" {
                            self.state_loaded = true;
                            if data["isStreaming"] == true {
                                self.status = Status::Running;
                            } else if self.history_loaded && self.status == Status::Connecting {
                                self.status = Status::Ready;
                            }
                        }
                    }
                    "get_available_models" => {
                        self.models = record["data"]["models"]
                            .as_array()
                            .map(|models| {
                                models
                                    .iter()
                                    .filter(|model| model["provider"] == "cpa")
                                    .filter_map(|model| {
                                        Some(AgentModel {
                                            provider: model["provider"].as_str()?.into(),
                                            id: model["id"].as_str()?.into(),
                                            name: model["name"]
                                                .as_str()
                                                .or(model["id"].as_str())?
                                                .into(),
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                    }
                    "get_available_thinking_levels" => {
                        self.thinking_levels = record["data"]["levels"]
                            .as_array()
                            .map(|levels| {
                                levels
                                    .iter()
                                    .filter_map(|level| level.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default();
                    }
                    "set_model" => {
                        self.update_model(&record["data"]);
                        // Refresh the effective level too. Pi may clamp it for the new model.
                        self.command("get_state", json!({}));
                        self.command("get_available_thinking_levels", json!({}));
                        self.configuring = false;
                    }
                    "set_thinking_level" => {
                        if let Some(level) = self.pending_thinking.take() {
                            self.thinking = level;
                        }
                        self.command("get_state", json!({}));
                        self.configuring = false;
                    }
                    "get_messages" if record["id"] == "history" => {
                        if let Some(messages) = record["data"]["messages"].as_array() {
                            self.messages.clear();
                            self.tools.clear();
                            self.assistant = None;
                            for message in messages {
                                let role = string(message, "role");
                                if matches!(role, "user" | "assistant") {
                                    let text = content_text(&message["content"]);
                                    if !text.is_empty() {
                                        self.messages.push(Message {
                                            role: if role == "user" { "You" } else { "Pi" },
                                            text,
                                        });
                                    }
                                }
                                if role == "assistant"
                                    && let Some(blocks) = message["content"].as_array()
                                {
                                    for block in blocks.iter().filter(|b| b["type"] == "toolCall") {
                                        self.tools.push(Tool {
                                            id: string(block, "id").into(),
                                            name: string(block, "name").into(),
                                            detail: tool_detail(&block["arguments"]),
                                            done: true,
                                            failed: true,
                                            output: "No saved result for this tool call.".into(),
                                        });
                                    }
                                }
                                if role == "toolResult"
                                    && let Some(tool) = self
                                        .tools
                                        .iter_mut()
                                        .rev()
                                        .find(|t| t.id == string(message, "toolCallId"))
                                {
                                    tool.failed = message["isError"] == true;
                                    tool.output =
                                        truncate(&content_text(&message["content"]), 6000);
                                }
                            }
                        }
                        self.history_loaded = true;
                        if self.state_loaded && self.status == Status::Connecting {
                            self.status = Status::Ready;
                        }
                    }
                    "prompt" => {
                        self.pending_prompt = None;
                        if record["data"]["disposition"] == "handled"
                            && self.status == Status::Running
                        {
                            self.status = Status::Ready;
                            self.started = None;
                            self.phase.clear();
                        }
                        // Capture the newly created session path before a long first run finishes.
                        self.command("get_state", json!({}));
                    }
                    _ => {}
                }
            }
            "agent_start" => {
                self.status = Status::Running;
                self.assistant = None;
                self.phase = "Waiting for the model".into();
            }
            // agent_end is deliberately NOT settlement. Pi can still retry or compact.
            "agent_settled" => {
                self.request = None;
                self.finish_run();
            }
            "message_start" if record["message"]["role"] == "assistant" => {
                self.assistant = None;
                self.request = None;
                self.status = Status::Running;
            }
            "message_update" => {
                let update = &record["assistantMessageEvent"];
                if update["type"] == "text_delta" {
                    self.assistant().text.push_str(string(update, "delta"));
                    self.phase = "Writing".into();
                } else if update["type"] == "thinking_delta" {
                    self.phase = "Thinking".into();
                }
            }
            "message_end" if record["message"]["role"] == "assistant" => {
                let message = &record["message"];
                let text = content_text(&message["content"]);
                if !text.is_empty() {
                    self.assistant().text = text;
                    self.error = None;
                }
                if message["stopReason"] != "aborted"
                    && let Some(error) = message["errorMessage"].as_str().filter(|s| !s.is_empty())
                {
                    self.error = Some(error.to_owned());
                }
                self.assistant = None;
            }
            "tool_execution_start" => {
                self.phase = format!("Running {}", string(&record, "toolName"));
                self.tools.push(Tool {
                    id: string(&record, "toolCallId").into(),
                    name: string(&record, "toolName").into(),
                    detail: tool_detail(&record["args"]),
                    done: false,
                    failed: false,
                    output: String::new(),
                });
            }
            "auto_retry_start" => {
                self.phase = format!("Retrying, attempt {}", record["attempt"]);
                self.diagnostics
                    .push(string(&record, "errorMessage").into());
            }
            "auto_retry_end" => {
                if record["success"] == true {
                    self.error = None;
                } else {
                    self.error = Some(string(&record, "finalError").into());
                }
            }
            "compaction_start" => self.phase = "Compacting context".into(),
            "compaction_end" => {
                if let Some(error) = record["errorMessage"].as_str() {
                    self.error = Some(error.into());
                }
            }
            "thinking_level_changed" => self.thinking = string(&record, "level").into(),
            "tool_execution_update" | "tool_execution_end" => {
                let ending = record["type"] == "tool_execution_end";
                if let Some(tool) = self
                    .tools
                    .iter_mut()
                    .find(|tool| tool.id == string(&record, "toolCallId"))
                {
                    tool.done = ending;
                    tool.failed = record["isError"] == true;
                    tool.output = truncate(
                        &content_text(if ending {
                            &record["result"]["content"]
                        } else {
                            &record["partialResult"]["content"]
                        }),
                        6000,
                    );
                }
            }
            "extension_ui_request" => {
                if matches!(
                    string(&record, "method"),
                    "confirm" | "select" | "input" | "editor"
                ) {
                    self.reply.replace_text(string(&record, "prefill"));
                    self.request = Some(record);
                    self.status = Status::Waiting;
                } else if record["method"] == "notify" {
                    self.diagnostics.push(string(&record, "message").into());
                } else if record["method"] == "set_editor_text" {
                    self.draft.replace_text(string(&record, "text"));
                }
            }
            _ => {}
        }
    }

    fn update_model(&mut self, model: &Value) {
        if let Some(id) = model["id"].as_str() {
            self.model_id = id.into();
            self.model_provider = string(model, "provider").into();
            self.model = model["name"].as_str().unwrap_or(id).into();
        }
    }

    fn restore_prompt(&mut self, prompt: &str) {
        let text = if self.draft.text.is_empty() {
            prompt.to_owned()
        } else {
            format!("{prompt}\n\n{}", self.draft.text)
        };
        self.draft.replace_text(&text);
    }

    fn finish_run(&mut self) {
        self.status = if self.error.is_some() {
            Status::Failed
        } else {
            Status::Settled
        };
        self.elapsed = self
            .started
            .take()
            .map(|start| start.elapsed().as_secs())
            .or(self.elapsed);
        self.assistant = None;
        self.phase.clear();
    }
}

pub struct State {
    pub workspace: PathBuf,
    pub threads: Vec<Thread>,
    pub active: usize,
    pub next_id: usize,
    pub dark: bool,
    pub show_activity: bool,
    pub show_archive: bool,
    pub sidebar: bool,
    /// Whether the sidebar is shown while the window is too narrow for it to stay docked.
    pub compact_sidebar: bool,
    pub search: TextEdit,
    pub demo: bool,
    pub wake: Waker,
    pub store: Option<crate::store::Store>,
    pub storage_error: Option<String>,
    pub artwork: Arc<crate::visuals::Artwork>,
    pub captain_picker: bool,
    pub captains: Vec<Captain>,
    pub captain_editor: Option<CaptainEditor>,
    pub search_open: bool,
    pub settings_picker: Option<SettingsPicker>,
    pub model_search: TextEdit,
}

impl State {
    pub fn new(workspace: PathBuf, demo: bool) -> Self {
        let artwork = crate::visuals::Artwork::load(&workspace);
        let mut state = Self {
            workspace,
            threads: vec![Thread::new(1)],
            active: 0,
            next_id: 2,
            dark: false,
            show_activity: false,
            show_archive: false,
            sidebar: true,
            compact_sidebar: false,
            search: TextEdit::default(),
            demo,
            wake: Arc::new(|| {}),
            store: None,
            storage_error: None,
            artwork,
            captain_picker: false,
            captains: Captain::defaults(),
            captain_editor: None,
            search_open: false,
            settings_picker: None,
            model_search: TextEdit::default(),
        };
        if demo {
            state.populate_demo();
        }
        state
    }
    pub fn current(&self) -> &Thread {
        &self.threads[self.active]
    }
    pub fn current_mut(&mut self) -> &mut Thread {
        &mut self.threads[self.active]
    }
    pub fn new_thread(&mut self) {
        let mut thread = Thread::new(self.next_id);
        thread.captain = self.current().captain;
        self.threads.push(thread);
        self.next_id += 1;
        self.active = self.threads.len() - 1;
        self.show_archive = false;
        self.checkpoint();
    }
    pub fn open_captain(&mut self, captain: usize) {
        if captain >= self.captains.len() {
            return;
        }
        if let Some(index) = self.threads.iter().position(|thread| {
            thread.captain == Some(captain) && thread.captain_home && !thread.archived
        }) {
            self.active = index;
            self.current_mut().unread = false;
        } else {
            let mut thread = Thread::new(self.next_id);
            self.next_id += 1;
            thread.title = self.captains[captain].name.clone();
            thread.captain = Some(captain);
            thread.captain_home = true;
            self.threads.push(thread);
            self.active = self.threads.len() - 1;
        }
        self.show_archive = false;
        self.captain_picker = false;
        self.checkpoint();
    }

    pub fn begin_new_captain(&mut self) {
        self.captain_picker = false;
        self.captain_editor = Some(CaptainEditor::new());
    }

    pub fn edit_captain(&mut self, captain: usize) {
        let Some(profile) = self.captains.get(captain) else {
            return;
        };
        let mut editor = CaptainEditor::new();
        editor.editing = Some(captain);
        editor.name.replace_text(&profile.name);
        editor.role.replace_text(&profile.role);
        editor.instructions.replace_text(&profile.instructions);
        editor.portrait = profile.portrait;
        self.captain_editor = Some(editor);
    }

    pub fn save_captain(&mut self) {
        let Some(editor) = &self.captain_editor else {
            return;
        };
        let profile = editor.profile();
        let editing = editor.editing;
        let error = profile
            .validate()
            .err()
            .or_else(|| {
                self.captains
                    .iter()
                    .enumerate()
                    .any(|(i, existing)| {
                        Some(i) != editing
                            && existing.name.to_lowercase() == profile.name.to_lowercase()
                    })
                    .then(|| {
                        "That captain name is already in this workspace. Choose a different name."
                            .into()
                    })
            })
            .or_else(|| {
                editing
                    .filter(|&id| {
                        self.threads
                            .iter()
                            .any(|t| t.captain == Some(id) && t.status.busy())
                    })
                    .map(|_| {
                        "Stop this captain's running threads before editing its profile.".into()
                    })
            });
        if let Some(error) = error {
            self.captain_editor.as_mut().unwrap().error = Some(error);
            return;
        }
        let id = if let Some(id) = editing {
            if id >= self.captains.len() {
                return;
            }
            self.captains[id] = profile;
            // Restart idle connections on demand so the next send uses the edited profile.
            for thread in self.threads.iter_mut().filter(|t| t.captain == Some(id)) {
                thread.disconnect();
                if thread.captain_home {
                    thread.title = self.captains[id].name.clone();
                }
            }
            id
        } else {
            self.captains.push(profile);
            self.captains.len() - 1
        };
        self.captain_editor = None;
        self.open_captain(id);
        self.current_mut().draft.focused = true;
    }
    /// The captain's most urgent live thread. Archived conversations do not vote.
    pub fn captain_status(&self, captain: usize) -> Status {
        self.threads
            .iter()
            .filter(|thread| thread.captain == Some(captain) && !thread.archived)
            .map(|thread| thread.status)
            .max_by_key(|status| status.urgency())
            .unwrap_or(Status::Offline)
    }
    pub fn toggle_pin(&mut self) {
        self.current_mut().pinned = !self.current().pinned;
        self.checkpoint();
    }
    pub fn connect(&mut self) {
        if self.demo {
            return;
        }
        let path = self.workspace.clone();
        let wake = self.wake.clone();
        let instructions = self
            .current()
            .captain
            .and_then(|id| self.captains.get(id))
            .map(Captain::prompt);
        if instructions.is_none() {
            self.current_mut().connect(path, wake);
        } else {
            self.current_mut()
                .connect_with_instructions(path, wake, instructions);
        }
    }
    pub fn send(&mut self) {
        if self.demo
            || self.current().archived
            || self.current().status.busy()
            || self.current().configuring
            || self.current().draft.text.trim().is_empty()
        {
            return;
        }
        if self.current().pi.is_none() {
            self.connect();
            self.current_mut().send_when_ready = true;
        } else {
            self.current_mut().send();
        }
        self.checkpoint();
    }
    pub fn switch_workspace(&mut self, path: PathBuf) {
        if self
            .threads
            .iter()
            .any(|thread| thread.status.busy() || !thread.draft.text.is_empty())
        {
            self.current_mut().error = Some("Stop running threads and clear unsent drafts before opening another folder. Your current workspace has not changed.".into());
            return;
        }
        let path = match path.canonicalize() {
            Ok(path) if path.is_dir() => path,
            _ => {
                self.current_mut().error =
                    Some("That workspace folder could not be opened.".into());
                return;
            }
        };
        self.checkpoint();
        let mut next = State::new(path, false);
        next.setup(self.wake.clone());
        *self = next;
    }
    pub fn archive(&mut self) {
        if self.current().status.busy() {
            return;
        }
        let archived = !self.current().archived;
        self.current_mut().archived = archived;
        if archived {
            self.current_mut().disconnect();
            if let Some(index) = self.threads.iter().position(|thread| !thread.archived) {
                self.active = index;
            } else {
                self.new_thread();
            }
        }
        self.checkpoint();
    }
    pub fn setup(&mut self, wake: Waker) {
        self.wake = wake.clone();
        if self.demo {
            return;
        }
        let Some(path) = crate::store::path(&self.workspace) else {
            return;
        };
        match crate::store::load(&path) {
            Ok(Some(index)) if index["version"] == 1 || index["version"] == 2 => {
                if index["version"] == 2 {
                    let Some(profiles) = index["captains"].as_array().filter(|p| !p.is_empty())
                    else {
                        self.storage_error = Some("The saved captain profiles are invalid. The index has not been overwritten.".into());
                        return;
                    };
                    let restored: Option<Vec<Captain>> =
                        profiles.iter().map(Captain::from_value).collect();
                    let Some(restored) = restored else {
                        self.storage_error = Some("A saved captain profile is invalid. The index has not been overwritten.".into());
                        return;
                    };
                    self.captains = restored;
                }
                if let Some(threads) = index["threads"]
                    .as_array()
                    .filter(|threads| !threads.is_empty())
                {
                    let restored: Vec<Thread> = threads
                        .iter()
                        .filter_map(|record| {
                            let id = usize::try_from(record["id"].as_u64()?).ok()?;
                            if id == 0 || id == usize::MAX {
                                return None;
                            }
                            let mut thread = Thread::new(id);
                            thread.title = record["title"].as_str()?.into();
                            thread.session = record["session"].as_str().map(str::to_owned);
                            thread.archived = record["archived"] == true;
                            thread.captain = if record["captain"].is_null() {
                                None
                            } else {
                                let id = usize::try_from(record["captain"].as_u64()?).ok()?;
                                if id >= self.captains.len() {
                                    return None;
                                }
                                Some(id)
                            };
                            thread.captain_home = record["captainHome"] == true;
                            if let (true, Some(captain)) = (thread.captain_home, thread.captain) {
                                // Older indexes stored the role name as the home title.
                                thread.title = self.captains[captain].name.clone();
                            }
                            thread.pinned = record["pinned"] == true;
                            Some(thread)
                        })
                        .collect();
                    let ids: std::collections::HashSet<_> = restored.iter().map(|t| t.id).collect();
                    if restored.len() != threads.len() || ids.len() != restored.len() {
                        self.storage_error = Some("The saved index contains invalid or duplicate threads. It has not been overwritten.".into());
                        return;
                    }
                    self.threads = restored;
                    self.next_id = self.threads.iter().map(|t| t.id).max().unwrap_or(0) + 1;
                    self.active = self.threads.iter().position(|t| !t.archived).unwrap_or(0);
                } else {
                    self.storage_error = Some(
                        "The saved index has no valid thread list. It has not been overwritten."
                            .into(),
                    );
                    return;
                }
                self.dark = index["dark"] == true;
            }
            Ok(Some(_)) => {
                self.storage_error = Some("The saved thread index uses an unsupported version. It has not been overwritten.".into());
                return;
            }
            Ok(None) => {}
            Err(error) => {
                self.storage_error = Some(error);
                return;
            }
        }
        self.store = Some(crate::store::Store::new(path, wake));
        self.checkpoint();
    }
    pub fn checkpoint(&mut self) {
        if self.demo {
            return;
        }
        let snapshot = json!({
            "version":2, "dark":self.dark,
            "captains":self.captains.iter().map(Captain::to_value).collect::<Vec<_>>(),
            "threads":self.threads.iter().map(|thread|json!({
                "id":thread.id, "title":thread.title, "session":thread.session, "archived":thread.archived,
                "captain":thread.captain, "captainHome":thread.captain_home, "pinned":thread.pinned
            })).collect::<Vec<_>>()
        });
        if let Some(store) = &mut self.store {
            store.save(snapshot);
        }
    }
    pub fn populate_demo(&mut self) {
        self.demo = true;
        self.threads.clear();
        let mut thread = Thread::new(1);
        thread.title = "Fix the reconnect listener".into();
        thread.captain = Some(0);
        thread.status = Status::Settled;
        thread.model = DEFAULT_MODEL.into();
        thread.thinking = "medium".into();
        thread.elapsed = Some(42);
        thread.messages = vec![
            Message { role: "You", text: "Fix the reconnect race, and add a regression test for the duplicate listener.".into() },
            Message { role: "Pi", text: "I'll check the socket lifecycle and where the reconnect listener attaches before making the change.".into() },
            Message { role: "Pi", text: "The listener now attaches before the socket opens. Cleanup removes it before the next attempt, so a reconnect cannot register it twice.\n\n1. **Order.** `socket.on(\"reconnect\")` runs before `socket.open()`, so the first event cannot be missed.\n2. **Cleanup.** `teardown()` removes the listener before each retry.\n3. **Test.** `reconnect.test.ts` covers a disconnect during initialization and a second reconnect. Both keep one listener.\n\n```diff\n- socket.open();\n  socket.on(\"reconnect\", reconnect);\n+ socket.open();\n```\n\nThe thread is settled. There is no queued follow-up work.".into() },
        ];
        thread.tools = vec![
            Tool {
                id: "example-1".into(),
                name: "read".into(),
                detail: "src/transport/socket.ts".into(),
                done: true,
                failed: false,
                output: "Example activity, not a real tool execution.".into(),
            },
            Tool {
                id: "example-2".into(),
                name: "read".into(),
                detail: "tests/reconnect.test.ts".into(),
                done: true,
                failed: false,
                output: "Example activity, not a real tool execution.".into(),
            },
        ];
        self.threads.push(thread);
        for (id, title, status) in [
            (2, "Review sidebar keys", Status::Running),
            (3, "Reduce cold-start work", Status::Settled),
            (4, "Add a native file picker", Status::Settled),
            (5, "Plan the Pi session adapter", Status::Settled),
            (6, "Audit the Markdown renderer", Status::Ready),
        ] {
            let mut thread = Thread::new(id);
            thread.title = title.into();
            thread.status = status;
            thread.pinned = id == 3;
            if id == 2 {
                thread.captain = Some(0);
            }
            self.threads.push(thread);
        }
        // One home per captain, in different states, so the crew shows its range.
        for (id, captain, status) in [
            (7, 0, Status::Ready),
            (8, 1, Status::Waiting),
            (9, 2, Status::Settled),
            (10, 3, Status::Ready),
        ] {
            let mut thread = Thread::new(id);
            thread.title = crate::visuals::CAPTAIN_NAMES[captain].into();
            thread.captain = Some(captain);
            thread.captain_home = true;
            thread.status = status;
            self.threads.push(thread);
        }
        self.next_id = 11;
        self.active = 0;
    }
}

pub fn poll(state: &mut State, _: f64, _: &mut Tx) -> bool {
    let mut changed = false;
    for (index, thread) in state.threads.iter_mut().enumerate() {
        // Batch incoming deltas into one view update, with a bounded per-frame drain.
        for _ in 0..256 {
            let event = thread.pi.as_ref().and_then(|pi| pi.events.try_recv().ok());
            let Some(event) = event else { break };
            changed = true;
            match event {
                Event::Record(record) => {
                    let settled = record["type"] == "agent_settled";
                    thread.apply(record);
                    if settled {
                        thread.command("get_state", json!({}));
                    }
                    if index != state.active {
                        thread.unread = true;
                    }
                }
                Event::Error(error) => {
                    thread.disconnect();
                    thread.error = Some(error);
                    thread.status = Status::Failed;
                }
                Event::Diagnostic(diagnostic) => thread.diagnostics.push(diagnostic),
                Event::Exited => {
                    thread.disconnect();
                    thread.status = if thread.error.is_some() {
                        Status::Failed
                    } else {
                        Status::Offline
                    };
                }
            }
        }
        if thread.send_when_ready && thread.status == Status::Ready {
            thread.send_when_ready = false;
            thread.send();
            changed = true;
        }
    }
    if let Some(error) = state
        .store
        .as_ref()
        .and_then(|store| store.errors.try_recv().ok())
    {
        state.storage_error = Some(error);
        changed = true;
    }
    if changed {
        state.checkpoint();
    }
    changed
}

pub fn string<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.into();
    }
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
fn tool_detail(args: &Value) -> String {
    ["path", "file_path", "command", "pattern", "query"]
        .iter()
        .find_map(|key| args[key].as_str())
        .map(|s| truncate(s, 140))
        .unwrap_or_default()
}
pub fn truncate(text: &str, count: usize) -> String {
    let mut result: String = text.chars().take(count).collect();
    if text.chars().count() > count {
        result.push_str("...");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_level_agent_end_is_not_settlement() {
        let mut thread = Thread::new(1);
        thread.status = Status::Running;
        thread.apply(json!({"type":"agent_end"}));
        assert_eq!(thread.status, Status::Running);
        thread.apply(json!({"type":"agent_settled","aborted":false}));
        assert_eq!(thread.status, Status::Settled);
    }
    #[test]
    fn final_message_replaces_deltas_without_duplicating_text() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"message_start","message":{"role":"assistant"}}));
        thread.apply(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Part"}}));
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Complete"}]}}));
        assert_eq!(thread.messages.len(), 1);
        assert_eq!(thread.messages[0].text, "Complete");
    }
    #[test]
    fn switching_and_archiving_preserve_other_drafts() {
        let mut state = State::new(PathBuf::from("."), false);
        state.current_mut().draft.replace_text("Keep this");
        state.new_thread();
        state.archive();
        assert_eq!(state.current().draft.text, "Keep this");
        assert!(state.threads[1].archived);
    }
    #[test]
    fn captain_portrait_follows_the_most_urgent_thread() {
        let mut state = State::new(PathBuf::from("."), false);
        state.open_captain(0);
        state.current_mut().status = Status::Settled;
        state.new_thread();
        state.current_mut().status = Status::Running;
        state.new_thread();
        state.current_mut().status = Status::Waiting;
        assert_eq!(state.captain_status(0), Status::Waiting);
        state.current_mut().status = Status::Failed;
        assert_eq!(state.captain_status(0), Status::Failed);
        state.current_mut().archived = true;
        assert_eq!(state.captain_status(0), Status::Running);
        assert_eq!(state.captain_status(1), Status::Offline);
    }
    #[test]
    fn connection_is_not_ready_until_both_initial_responses_arrive() {
        let mut thread = Thread::new(1);
        thread.status = Status::Connecting;
        thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false}}));
        assert_eq!(thread.status, Status::Connecting);
        thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}));
        assert_eq!(thread.status, Status::Ready);
        thread.status = Status::Running;
        thread.apply(json!({"type":"response","id":"later-state","command":"get_state","success":true,"data":{"isStreaming":false}}));
        assert_eq!(
            thread.status,
            Status::Running,
            "stale state must not settle a newer prompt"
        );
    }
    #[test]
    fn a_confirmation_waits_for_the_user_and_is_never_auto_approved() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"extension_ui_request","id":"permission","method":"confirm","title":"Run command?"}));
        assert_eq!(thread.status, Status::Waiting);
        assert_eq!(thread.request.as_ref().unwrap()["id"], "permission");
        thread.apply(json!({"type":"agent_settled","aborted":false}));
        assert!(thread.request.is_none());
    }
    #[test]
    fn reconnect_restores_tool_calls_and_results_without_duplicates() {
        let mut thread = Thread::new(1);
        let history = json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":"Check the file"},
            {"role":"assistant","content":[{"type":"toolCall","id":"read-1","name":"read","arguments":{"path":"example.txt"}}]},
            {"role":"toolResult","toolCallId":"read-1","toolName":"read","content":[{"type":"text","text":"saved output"}],"isError":false},
            {"role":"assistant","content":[{"type":"text","text":"Done"}]}
        ]}});
        thread.apply(history.clone());
        thread.apply(history);
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.tools.len(), 1);
        assert_eq!(thread.tools[0].detail, "example.txt");
        assert_eq!(thread.tools[0].output, "saved output");
        assert!(thread.tools[0].done && !thread.tools[0].failed);
    }
    #[test]
    fn rejected_prompt_returns_to_the_draft_and_does_not_claim_it_was_sent() {
        let mut thread = Thread::new(1);
        thread.messages.push(Message {
            role: "You",
            text: "Try this".into(),
        });
        thread.pending_prompt = Some(("1-1".into(), "Try this".into(), 0));
        thread.status = Status::Running;
        thread.apply(json!({"type":"response","id":"1-1","command":"prompt","success":false,"error":"No model configured"}));
        assert_eq!(thread.status, Status::Failed);
        assert_eq!(thread.draft.text, "Try this");
        assert!(thread.messages.is_empty());
        assert!(!thread.send_when_ready);
    }
    #[test]
    fn failed_settings_leave_the_effective_model_and_level_unchanged() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"response","command":"set_model","success":true,"data":{"provider":"test","id":"actual","name":"Actual model"}}));
        thread.configuring = true;
        thread.pending_thinking = Some("high".into());
        thread.thinking = "low".into();
        thread.apply(json!({"type":"response","command":"set_thinking_level","success":false,"error":"Unsupported level"}));
        assert_eq!(thread.model_id, "actual");
        assert_eq!(thread.model_provider, "test");
        assert_eq!(thread.thinking, "low");
        assert!(!thread.configuring);
        assert!(thread.pending_thinking.is_none());
    }
    #[test]
    fn aborted_message_is_not_a_provider_failure() {
        let mut thread = Thread::new(1);
        thread.status = Status::Running;
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"aborted","errorMessage":"Request was aborted"}}));
        thread.apply(json!({"type":"agent_settled","aborted":true}));
        assert_eq!(thread.status, Status::Settled);
        assert!(thread.error.is_none());
        thread.apply(json!({"type":"response","command":"abort","success":true}));
        assert_eq!(thread.status, Status::Settled);
    }
    #[test]
    fn successful_retry_clears_the_transient_provider_error() {
        let mut thread = Thread::new(1);
        thread.status = Status::Running;
        thread.error = Some("Temporary provider error".into());
        thread.apply(json!({"type":"auto_retry_end","success":true}));
        thread.apply(json!({"type":"agent_settled","aborted":false}));
        assert_eq!(thread.status, Status::Settled);
        assert!(thread.error.is_none());
    }
    #[test]
    fn first_send_waits_for_both_handshake_records_and_uses_the_active_draft() {
        let mut state = State::new(".".into(), false);
        let (pi, commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().status = Status::Connecting;
        state.current_mut().send_when_ready = true;
        state.current_mut().draft.replace_text("Original draft");
        events.send(Event::Record(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}))).unwrap();
        let mut tx = Tx::default();
        poll(&mut state, 0.0, &mut tx);
        assert!(
            commands.try_recv().is_err(),
            "history alone must not start work"
        );
        state
            .current_mut()
            .draft
            .replace_text("Revised before connection");
        events.send(Event::Record(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false,"model":{"provider":"cpa","id":"model-a"}}}))).unwrap();
        poll(&mut state, 0.0, &mut tx);
        let prompt = commands.try_recv().unwrap();
        assert_eq!(prompt["type"], "prompt");
        assert_eq!(prompt["message"], "Revised before connection");
        assert_eq!(state.current().status, Status::Running);
        assert!(state.current().draft.text.is_empty());
        assert!(!state.current().send_when_ready);
        poll(&mut state, 0.0, &mut tx);
        assert!(commands.try_recv().is_err(), "submit exactly once");
    }
    #[test]
    fn stopping_cancels_a_dialog_clears_the_queue_and_then_aborts() {
        let (pi, commands, _events) = Pi::test_connection();
        let mut thread = Thread::new(1);
        thread.pi = Some(pi);
        thread.apply(json!({"type":"extension_ui_request","id":"dialog-1","method":"confirm"}));
        thread.stop();
        let cancel = commands.try_recv().unwrap();
        assert_eq!(cancel["type"], "extension_ui_response");
        assert_eq!(cancel["id"], "dialog-1");
        assert_eq!(cancel["cancelled"], true);
        assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
        assert_eq!(commands.try_recv().unwrap()["type"], "abort");
        assert!(thread.request.is_none());
    }
    #[test]
    fn connect_alone_never_sends_a_model_prompt() {
        let (pi, commands, events) = Pi::test_connection();
        let mut state = State::new(".".into(), false);
        state.current_mut().pi = Some(pi);
        state.current_mut().status = Status::Connecting;
        state.current_mut().draft.replace_text("Do not send this");
        for record in [
            json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false}}),
            json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}),
        ] {
            events.send(Event::Record(record)).unwrap();
        }
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.current().status, Status::Ready);
        assert_eq!(state.current().draft.text, "Do not send this");
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn direct_provider_models_are_hidden_and_cannot_receive_prompts() {
        let (pi, commands, _events) = Pi::test_connection();
        let mut thread = Thread::new(1);
        thread.pi = Some(pi);
        thread.status = Status::Ready;
        thread.apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"provider":"google","id":"direct","name":"Direct Google"},
            {"provider":"cpa","id":"proxy","name":"CPA proxy"}
        ]}}));
        assert_eq!(thread.models.len(), 1);
        assert_eq!(thread.models[0].id, "proxy");
        thread.model_provider = "google".into();
        thread.draft.replace_text("Do not send directly");
        thread.send();
        assert!(commands.try_recv().is_err());
        assert_eq!(thread.draft.text, "Do not send directly");
        assert!(thread.error.as_ref().unwrap().contains("CPA model"));
        thread.set_model("google".into(), "direct".into());
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn rejected_prompt_preserves_a_newer_draft_too() {
        let mut thread = Thread::new(1);
        thread.pending_prompt = Some(("1-1".into(), "Rejected prompt".into(), 0));
        thread.draft.replace_text("New draft");
        thread.apply(json!({"type":"response","id":"1-1","command":"prompt","success":false,"error":"Rejected"}));
        assert_eq!(thread.draft.text, "Rejected prompt\n\nNew draft");
    }
    #[test]
    fn stopping_connection_cancels_the_deferred_first_send() {
        let (pi, commands, _events) = Pi::test_connection();
        let mut thread = Thread::new(1);
        thread.pi = Some(pi);
        thread.status = Status::Connecting;
        thread.send_when_ready = true;
        thread.draft.replace_text("Keep this draft");
        thread.stop();
        assert_eq!(thread.status, Status::Offline);
        assert!(thread.pi.is_none());
        assert!(!thread.send_when_ready);
        assert_eq!(thread.draft.text, "Keep this draft");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn creating_and_editing_a_captain_preserves_conversations_and_drafts() {
        let mut state = State::new(".".into(), false);
        state
            .current_mut()
            .draft
            .replace_text("Original thread draft");
        state.begin_new_captain();
        let editor = state.captain_editor.as_mut().unwrap();
        editor.name.replace_text("Atlas");
        editor.role.replace_text("Review");
        editor.instructions.replace_text("Cite failing tests.");
        editor.portrait = 2;
        state.save_captain();
        assert_eq!(state.captains.len(), 5);
        assert_eq!(state.current().captain, Some(4));
        assert_eq!(state.current().title, "Atlas");
        assert!(
            state.current().pi.is_none(),
            "creation must not start agent work"
        );
        assert_eq!(state.threads[0].draft.text, "Original thread draft");
        let home_id = state.current().id;
        state.new_thread();
        state.current_mut().draft.replace_text("Crew draft");
        let crew_id = state.current().id;
        assert_eq!(state.current().captain, Some(4));
        state.edit_captain(4);
        state
            .captain_editor
            .as_mut()
            .unwrap()
            .name
            .replace_text("Atlas Review");
        state.save_captain();
        assert_eq!(state.current().id, home_id);
        assert_eq!(state.current().title, "Atlas Review");
        assert_eq!(
            state
                .threads
                .iter()
                .find(|t| t.id == crew_id)
                .unwrap()
                .draft
                .text,
            "Crew draft"
        );
        assert!(state.captains[4].prompt().contains("Cite failing tests."));
    }

    #[test]
    fn duplicate_names_and_busy_captain_edits_do_not_save() {
        let mut state = State::new(".".into(), false);
        state.begin_new_captain();
        state
            .captain_editor
            .as_mut()
            .unwrap()
            .name
            .replace_text(" ember ");
        state.save_captain();
        assert_eq!(state.captains.len(), 4);
        assert!(
            state
                .captain_editor
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("already")
        );
        state.captain_editor = None;
        state.open_captain(2);
        state.current_mut().status = Status::Running;
        state.edit_captain(2);
        state
            .captain_editor
            .as_mut()
            .unwrap()
            .name
            .replace_text("Changed");
        state.save_captain();
        assert_eq!(state.captains[2].name, "Sable");
        assert_eq!(state.current().status, Status::Running);
        assert!(
            state
                .captain_editor
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("Stop")
        );
    }

    #[test]
    fn workspace_index_migrates_and_restores_custom_captains() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let workspace =
            std::env::temp_dir().join(format!("shina-profile-{}-{unique}", std::process::id()));
        let index_path = crate::store::path(&workspace).unwrap();
        assert!(
            !index_path.exists(),
            "test must not overwrite an existing index"
        );
        let wake: Waker = Arc::new(|| {});
        {
            let mut store = crate::store::Store::new(index_path.clone(), wake.clone());
            store.save(json!({"version":1,"dark":true,"threads":[
                {"id":10,"title":"Old Captain","captain":2,"captainHome":true,"archived":false,"session":"saved-session"}
            ]}));
        }
        {
            let mut state = State::new(workspace.clone(), false);
            state.setup(wake.clone());
            assert!(state.storage_error.is_none());
            assert_eq!(state.captains.len(), 4);
            assert_eq!(state.current().title, "Sable");
            assert_eq!(state.current().session.as_deref(), Some("saved-session"));
            state.begin_new_captain();
            state
                .captain_editor
                .as_mut()
                .unwrap()
                .name
                .replace_text("Atlas");
            state.captain_editor.as_mut().unwrap().portrait = 2;
            state.save_captain();
        }
        assert_eq!(
            crate::store::load(&index_path).unwrap().unwrap()["version"],
            2
        );
        {
            let mut restored = State::new(workspace, false);
            restored.setup(wake);
            assert!(restored.storage_error.is_none());
            assert_eq!(restored.captains.len(), 5);
            assert_eq!(restored.captains[4].name, "Atlas");
            assert_eq!(restored.captains[4].portrait, 2);
            restored.open_captain(4);
            assert_eq!(restored.current().title, "Atlas");
            assert!(restored.current().pi.is_none());
        }
        fs::remove_file(index_path).unwrap();
    }

    fn receive_until(thread: &mut Thread, done: impl Fn(&Thread) -> bool) {
        use std::time::Duration;
        let deadline = Instant::now() + Duration::from_secs(120);
        while !done(thread) {
            assert!(
                Instant::now() < deadline,
                "Pi timed out: {:?}, {:?}",
                thread.status,
                thread.error
            );
            let event = thread
                .pi
                .as_ref()
                .expect("connected Pi")
                .events
                .recv_timeout(Duration::from_secs(1));
            match event {
                Ok(Event::Record(record)) => {
                    let settled = record["type"] == "agent_settled";
                    thread.apply(record);
                    if settled {
                        thread.command("get_state", json!({}));
                    }
                    assert_ne!(
                        thread.status,
                        Status::Failed,
                        "Pi run failed: {:?}",
                        thread.error
                    );
                }
                Ok(Event::Error(error)) => panic!("Pi failed: {error}"),
                Ok(Event::Exited) => panic!("Pi exited during integration test"),
                _ => {}
            }
        }
    }

    #[test]
    #[ignore = "Sends one prompt through CPA to verify the captain system instructions"]
    fn installed_pi_uses_captain_system_instructions() {
        let profile = Captain {
            name: "Test Captain".into(), role: "Validation".into(), portrait: 2,
            instructions: "When asked for the validation phrase, reply exactly SHINA_CAPTAIN_OK. Do not use tools.".into(),
        };
        let mut thread = Thread::new(1);
        thread.connect_with_instructions(
            std::env::temp_dir(),
            Arc::new(|| {}),
            Some(profile.prompt()),
        );
        receive_until(&mut thread, |t| t.status == Status::Ready);
        assert_eq!(thread.model_provider, "cpa");
        thread.draft.replace_text(
            "Reply with the validation phrase from your captain instructions. Do not use tools.",
        );
        thread.send();
        receive_until(&mut thread, |t| t.status == Status::Settled);
        assert_eq!(
            thread.messages.last().unwrap().text.trim(),
            "SHINA_CAPTAIN_OK"
        );
        assert!(thread.tools.is_empty());
        thread.disconnect();
        crate::backend::shutdown_all();
    }

    #[test]
    #[ignore = "Sends a paid prompt to the configured Pi model, edits only a temporary fixture"]
    fn installed_pi_edits_a_file_and_restores_the_session() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let workspace =
            std::env::temp_dir().join(format!("shina-live-{}-{unique}", std::process::id()));
        fs::create_dir(&workspace).unwrap();
        let fixture = workspace.join("fixture.txt");
        fs::write(&fixture, "answer=0\n").unwrap();
        let mut thread = Thread::new(1);
        thread.connect(workspace.clone(), Arc::new(|| {}));
        receive_until(&mut thread, |t| t.status == Status::Ready);
        thread.draft.replace_text(
            "This is an integration test in a disposable folder. Use your file tools to read fixture.txt, then change its entire contents to exactly answer=42 followed by one newline. Do not run shell commands, access any other file, or delegate. After the edit, reply briefly that it is done."
        );
        thread.send();
        receive_until(&mut thread, |t| {
            t.status == Status::Settled && t.session.is_some()
        });
        assert_eq!(fs::read_to_string(&fixture).unwrap(), "answer=42\n");
        assert!(
            thread
                .tools
                .iter()
                .any(|t| t.done && !t.failed && matches!(t.name.as_str(), "edit" | "write"))
        );
        assert!(
            thread
                .messages
                .iter()
                .any(|m| m.role == "Pi" && !m.text.is_empty())
        );
        let session = thread.session.clone().unwrap();
        let tool_count = thread.tools.len();
        let message_count = thread.messages.len();
        thread.disconnect();
        crate::backend::shutdown_all();
        let mut restored = Thread::new(2);
        restored.session = Some(session);
        restored.connect(workspace.clone(), Arc::new(|| {}));
        receive_until(&mut restored, |t| t.status == Status::Ready);
        assert_eq!(restored.messages.len(), message_count);
        assert_eq!(restored.tools.len(), tool_count);
        assert!(restored.tools.iter().all(|t| t.done && !t.failed));
        restored.disconnect();
        crate::backend::shutdown_all();
        fs::remove_file(fixture).unwrap();
        fs::remove_dir(workspace).unwrap();
    }
}
