use crate::backend::{Event, HostLink, Pi};
use crate::captains::{Captain, CaptainEditor};
use crate::projects::{Project, ProjectId};
use crate::session::{AgentModel, Catalog, IDLE_TIMEOUT, START_TIMEOUT};
use rune::{TextEdit, Tx, Waker};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// The label shown when a thread uses whatever model Pi is configured with.
pub const DEFAULT_MODEL: &str = "Default model";
const INJECTION_PREFIX: &str = "[Shina orchestration]\n";

struct Injection {
    text: String,
    shown: bool,
}

struct TaskWaiter {
    thread: usize,
    deadline: Instant,
    reply: mpsc::Sender<Result<Value, String>>,
    // Dropping this sender cancels the deadline wake when the task finishes early.
    _timer: mpsc::Sender<()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ready,
    Running,
    Waiting,
    Settled,
    Failed,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Running => "Working",
            Self::Waiting => "Needs you",
            Self::Settled => "Settled",
            Self::Failed => "Error",
        }
    }
    pub fn busy(self) -> bool {
        matches!(self, Self::Running | Self::Waiting)
    }
    /// Which state a captain's portrait should show when its threads disagree. Attention
    /// outranks work, and work outranks a ready conversation.
    pub fn urgency(self) -> u8 {
        match self {
            Self::Failed => 6,
            Self::Waiting => 5,
            Self::Running => 4,
            Self::Ready => 2,
            Self::Settled => 1,
        }
    }
}

pub struct Message {
    pub role: &'static str,
    pub text: String,
}

impl Message {
    pub fn display_text(&self) -> &str {
        if self.role == "Shina"
            && let Some(body) = self.text.strip_prefix(INJECTION_PREFIX)
        {
            // Delivery metadata lets reconnect distinguish identical results from separate runs.
            return body
                .split_once('\n')
                .filter(|(header, _)| header.starts_with("[delivery "))
                .map(|(_, text)| text)
                .unwrap_or(body);
        }
        &self.text
    }
}

pub struct Tool {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub done: bool,
    pub failed: bool,
    pub output: String,
    /// Position in the transcript, including tool calls between text blocks in one message.
    pub anchor: ToolAnchor,
    pub block: usize,
    pub expanded: bool,
    pub group_expanded: bool,
    pub started: Option<Instant>,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToolAnchor {
    pub message: usize,
    /// UTF-8 byte offset into the message text. Zero also anchors tool-only messages.
    pub offset: usize,
}

impl Tool {
    fn new(id: &str, name: &str, args: &Value, anchor: ToolAnchor) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            detail: tool_detail(args),
            done: false,
            failed: false,
            output: String::new(),
            anchor,
            block: usize::MAX,
            expanded: false,
            group_expanded: false,
            started: None,
            duration_ms: None,
        }
    }
}

pub struct Thought {
    pub anchor: ToolAnchor,
    pub block: usize,
    pub text: String,
    pub expanded: bool,
    pub done: bool,
    pub started: Option<Instant>,
    /// Pi saves thinking text, but does not save thinking timings.
    pub duration_ms: Option<u64>,
}

pub struct AssistantWork {
    pub start: usize,
    pub expanded: bool,
    pub elapsed: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsPicker {
    Model,
    Thinking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowTarget {
    Thread(usize),
    Project(ProjectId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectScope {
    All,
    Project(ProjectId),
    NoProject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectPicker {
    NewThread,
    Scope,
}

pub struct Thread {
    pub id: usize,
    pub project: Option<ProjectId>,
    pub machine: String,
    pub updated: u64,
    pub title: String,
    pub renamed: bool,
    pub status: Status,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub thoughts: Vec<Thought>,
    pub work: Vec<AssistantWork>,
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
    pub started: Option<Instant>,
    pub elapsed: Option<u64>,
    pub captain: Option<usize>,
    pub captain_home: bool,
    pub pinned: bool,
    /// Started by another thread through the Shina MCP. Delegated threads cannot delegate.
    pub delegated: bool,
    pub parent: Option<usize>,
    /// How this thread's Pi reaches the Shina MCP, set when it connects.
    pub host: Option<HostLink>,
    assistant: Option<usize>,
    serial: usize,
    state_loaded: bool,
    history_loaded: bool,
    initializing: bool,
    session_started: Option<Instant>,
    last_activity: Instant,
    prewarm_attempted: bool,
    pub selected_model: Option<(String, String)>,
    pub selected_thinking: Option<String>,
    startup_model_applied: bool,
    startup_thinking_applied: bool,
    queued_prompt: Option<(String, usize)>,
    retry_prompt: Option<String>,
    pending_thinking: Option<String>,
    pending_prompt: Option<(String, String, usize)>,
    pending_injections: VecDeque<Injection>,
    injection_id: Option<String>,
    task_result: Option<&'static str>,
    completion_notified: bool,
}

impl Thread {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            project: None,
            machine: "local".into(),
            updated: crate::projects::now(),
            title: "New thread".into(),
            renamed: false,
            status: Status::Ready,
            messages: vec![],
            tools: vec![],
            thoughts: vec![],
            work: vec![],
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
            started: None,
            elapsed: None,
            captain: None,
            captain_home: false,
            pinned: false,
            delegated: false,
            parent: None,
            host: None,
            assistant: None,
            serial: 0,
            state_loaded: false,
            history_loaded: false,
            initializing: false,
            session_started: None,
            last_activity: Instant::now(),
            prewarm_attempted: false,
            selected_model: None,
            selected_thinking: None,
            startup_model_applied: false,
            startup_thinking_applied: false,
            queued_prompt: None,
            retry_prompt: None,
            pending_thinking: None,
            pending_prompt: None,
            pending_injections: VecDeque::new(),
            injection_id: None,
            task_result: None,
            completion_notified: true,
        }
    }

    #[cfg(test)]
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
        let backend = match crate::backend::BackendHost::resolve(&self.machine) {
            Ok(host) => host,
            Err(error) => {
                self.fail_session(error);
                return;
            }
        };
        self.error = None;
        if !self.pending_injections.is_empty() && !self.status.busy() {
            self.arm_task();
            self.status = Status::Running;
            self.started = None;
            self.phase = "Working".into();
        } else if self.status == Status::Failed {
            self.status = Status::Ready;
        }
        self.initializing = true;
        self.prewarm_attempted = true;
        self.session_started = Some(Instant::now());
        self.last_activity = Instant::now();
        self.state_loaded = false;
        self.history_loaded = false;
        self.startup_model_applied = false;
        self.startup_thinking_applied = false;
        self.pi = Some(backend.start(
            path,
            self.session.clone(),
            wake,
            instructions,
            self.host.clone(),
        ));
    }

    pub fn set_model(&mut self, provider: String, model_id: String) {
        if provider != "cpa" {
            self.error = Some(
                "Shina uses CPA proxy models only. Choose a model from the CPA catalog.".into(),
            );
            return;
        }
        if self.status.busy() || self.configuring {
            return;
        }
        self.selected_model = Some((provider.clone(), model_id.clone()));
        if self.pi.is_none() || self.initializing {
            self.model_provider = provider;
            self.model_id = model_id;
            self.model = self
                .models
                .iter()
                .find(|m| m.id == self.model_id)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| self.model_id.clone());
            self.refresh_thinking_levels();
        } else if self.command("set_model", json!({"provider":provider,"modelId":model_id})) {
            self.configuring = true;
        }
    }

    pub fn set_thinking(&mut self, level: String) {
        if self.status.busy() || self.configuring {
            return;
        }
        self.selected_thinking = Some(level.clone());
        if self.pi.is_none() || self.initializing {
            self.thinking = level;
        } else if self.command("set_thinking_level", json!({"level":level})) {
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
            .ok_or_else(|| "The session is unavailable. Retry to resume it.".to_owned())
            .and_then(|pi| pi.send(data));
        if let Err(error) = result {
            self.error = Some(error);
            return false;
        }
        self.last_activity = Instant::now();
        true
    }

    pub fn send(&mut self) {
        if self.status.busy() || self.configuring || self.draft.text.trim().is_empty() {
            return;
        }
        if !self.model_provider.is_empty() && self.model_provider != "cpa" {
            self.error = Some("Choose a CPA model before sending. Shina will not send prompts to a direct provider.".into());
            return;
        }
        let prompt = self.draft.text.trim().to_owned();
        self.queued_prompt = Some((prompt.clone(), self.messages.len()));
        self.retry_prompt = Some(prompt.clone());
        self.arm_task();
        if self.messages.is_empty() && !self.captain_home && !self.renamed {
            self.title = prompt.chars().take(45).collect();
        }
        self.messages.push(Message {
            role: "You",
            text: prompt,
        });
        self.draft.replace_text("");
        self.status = Status::Running;
        self.updated = crate::projects::now();
        self.started = Some(Instant::now());
        self.elapsed = None;
        self.assistant = None;
        self.ensure_work();
        self.error = None;
        self.phase = "Working".into();
        self.last_activity = Instant::now();
        self.drive_session();
    }

    pub fn stop(&mut self) {
        let _ = self.stop_task();
    }

    fn stop_task(&mut self) -> Result<(), String> {
        self.pending_injections.clear();
        self.injection_id = None;
        if self.initializing || self.queued_prompt.is_some() {
            self.disconnect();
            self.complete_task("cancelled");
            return Ok(());
        }
        if self.status == Status::Running || self.status == Status::Waiting {
            // Release a blocking extension dialog before asking Pi to wait for idle.
            if self.request.is_some() {
                self.respond(json!({"cancelled":true}));
            }
            if !self.command("clear_queue", json!({})) || !self.command("abort", json!({})) {
                return Err(self
                    .error
                    .clone()
                    .unwrap_or_else(|| "Pi could not stop the task.".into()));
            }
            self.phase = "Stopping".into();
        } else {
            self.complete_task("cancelled");
        }
        Ok(())
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
            .ok_or_else(|| "The session is unavailable. Retry to resume it.".to_owned())
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
        self.finish_thoughts();
        if let Some((prompt, index)) = self.queued_prompt.take() {
            self.remove_optimistic_prompt(&prompt, index);
            self.restore_prompt(&prompt);
        }
        if let Some((_, prompt, index)) = self.pending_prompt.take() {
            self.remove_optimistic_prompt(&prompt, index);
            self.restore_prompt(&prompt);
        }
        self.pi = None;
        self.status = Status::Ready;
        self.assistant = None;
        self.request = None;
        self.initializing = false;
        self.session_started = None;
        self.configuring = false;
        self.pending_thinking = None;
        self.injection_id = None;
        self.started = None;
        self.phase.clear();
    }

    fn arm_task(&mut self) {
        if self.delegated {
            self.task_result = None;
            self.completion_notified = false;
        }
    }

    fn complete_task(&mut self, result: &'static str) {
        if self.delegated && !self.completion_notified && self.task_result.is_none() {
            self.task_result = Some(result);
        }
    }

    fn enqueue_injection(&mut self, text: String) {
        self.serial += 1;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.pending_injections.push_back(Injection {
            text: format!(
                "{INJECTION_PREFIX}[delivery {}-{}-{stamp}]\n{text}",
                self.id, self.serial
            ),
            shown: false,
        });
    }

    fn flush_injection(&mut self) -> bool {
        if self.pending_injections.is_empty()
            || self.pi.is_none()
            || self.initializing
            || self.queued_prompt.is_some()
            || self.injection_id.is_some()
            || self.configuring
            || self.phase == "Stopping"
            || !matches!(
                self.status,
                Status::Ready | Status::Settled | Status::Running | Status::Waiting
            )
        {
            return false;
        }
        if self.model_provider != "cpa" {
            self.error =
                Some("Choose a CPA model before Shina can deliver queued messages.".into());
            return false;
        }
        let text = self.pending_injections[0].text.clone();
        // prompt starts an idle Pi; followUp also covers a run starting before Pi reads it.
        if !self.command(
            "prompt",
            json!({"message":text,"streamingBehavior":"followUp"}),
        ) {
            return false;
        }
        self.injection_id = Some(format!("{}-{}", self.id, self.serial));
        if !self.pending_injections[0].shown {
            self.messages.push(Message {
                role: "Shina",
                text,
            });
            self.pending_injections[0].shown = true;
        }
        if !self.status.busy() {
            self.arm_task();
            self.status = Status::Running;
            self.started = Some(Instant::now());
            self.elapsed = None;
            self.assistant = None;
            self.error = None;
            self.phase = "Waiting for Pi".into();
            self.ensure_work();
        }
        true
    }

    fn remove_optimistic_prompt(&mut self, prompt: &str, index: usize) {
        if self
            .messages
            .get(index)
            .is_some_and(|m| m.role == "You" && m.text == prompt)
        {
            self.messages.remove(index);
        }
    }

    fn fail_session(&mut self, error: String) {
        self.disconnect();
        self.error = Some(error);
        self.status = Status::Failed;
        self.complete_task("failed");
    }

    /// Settings and history are transport details, never conversation states.
    fn drive_session(&mut self) {
        if self.pi.is_none() || self.configuring || self.status == Status::Failed {
            return;
        }
        if self.initializing {
            if !(self.state_loaded && self.history_loaded) {
                return;
            }
            if !self.startup_model_applied {
                self.startup_model_applied = true;
                if let Some((provider, id)) = self.selected_model.clone() {
                    self.configuring =
                        self.command("set_model", json!({"provider":provider,"modelId":id}));
                    if !self.configuring {
                        self.fail_session(self.error.clone().unwrap());
                    }
                    return;
                }
            }
            if !self.startup_thinking_applied {
                self.startup_thinking_applied = true;
                if let Some(level) = self.selected_thinking.clone() {
                    self.pending_thinking = Some(level.clone());
                    self.configuring = self.command("set_thinking_level", json!({"level":level}));
                    if !self.configuring {
                        self.fail_session(self.error.clone().unwrap());
                    }
                    return;
                }
            }
            self.initializing = false;
            self.session_started = None;
            if self.queued_prompt.is_none()
                && self.started.is_none()
                && self.status == Status::Running
            {
                self.status = Status::Ready;
                self.phase.clear();
            }
        }
        let Some((prompt, index)) = self.queued_prompt.take() else {
            return;
        };
        if self.model_provider != "cpa" {
            self.queued_prompt = Some((prompt, index));
            self.fail_session("Choose a CPA model before sending. Shina will not send prompts to a direct provider.".into());
            return;
        }
        if self.command("prompt", json!({"message":prompt})) {
            self.pending_prompt = Some((format!("{}-{}", self.id, self.serial), prompt, index));
        } else {
            self.queued_prompt = Some((prompt, index));
            self.fail_session(
                self.error
                    .clone()
                    .unwrap_or_else(|| "Pi did not accept the message. Retry to resume.".into()),
            );
        }
    }

    fn refresh_thinking_levels(&mut self) {
        if let Some(model) = self
            .models
            .iter()
            .find(|m| m.provider == self.model_provider && m.id == self.model_id)
        {
            self.thinking_levels = model.thinking_levels.clone();
        }
    }

    pub fn restoring_history(&self) -> bool {
        self.initializing && !self.history_loaded
    }

    fn reclaim_idle(&mut self, now: Instant) -> bool {
        if self.pi.is_some()
            && !self.initializing
            && !self.configuring
            && !self.status.busy()
            && self.pending_injections.is_empty()
            && self.injection_id.is_none()
            && now.duration_since(self.last_activity) >= IDLE_TIMEOUT
        {
            let status = self.status;
            self.disconnect();
            self.status = status;
            return true;
        }
        false
    }

    fn assistant(&mut self) -> &mut Message {
        self.ensure_work();
        let index = *self.assistant.get_or_insert_with(|| {
            self.messages.push(Message {
                role: "Pi",
                text: String::new(),
            });
            self.messages.len() - 1
        });
        &mut self.messages[index]
    }

    fn ensure_work(&mut self) -> &mut AssistantWork {
        let start = self
            .messages
            .iter()
            .rposition(|message| message.role != "Pi")
            .map_or(0, |index| index + 1);
        if self.work.last().is_none_or(|work| work.start != start) {
            self.work.push(AssistantWork {
                start,
                expanded: true,
                elapsed: None,
            });
        }
        self.work.last_mut().unwrap()
    }

    fn thought(&mut self, block: usize) -> &mut Thought {
        self.assistant();
        let message = self.assistant.unwrap();
        let index = self
            .thoughts
            .iter()
            .position(|thought| thought.anchor.message == message && thought.block == block)
            .unwrap_or_else(|| {
                self.thoughts.push(Thought {
                    anchor: ToolAnchor {
                        message,
                        offset: self.messages[message].text.len(),
                    },
                    block,
                    text: String::new(),
                    expanded: false,
                    done: false,
                    started: None,
                    duration_ms: None,
                });
                self.thoughts.len() - 1
            });
        &mut self.thoughts[index]
    }

    /// Reuse Pi's content-block order for both restored history and completed live messages.
    /// Keep Thread.tools flat for existing consumers and record its transcript positions.
    fn anchor_content_work(&mut self, content: &Value, message: usize, restored: bool) {
        let Some(blocks) = content.as_array() else {
            return;
        };
        let mut offset = 0;
        let mut texts = 0;
        for (block_index, block) in blocks.iter().enumerate() {
            if block["type"] == "text" {
                if let Some(text) = block["text"].as_str() {
                    offset += text.len() + usize::from(texts > 0);
                    texts += 1;
                }
            } else if block["type"] == "toolCall" {
                let anchor = ToolAnchor { message, offset };
                let id = string(block, "id");
                if !restored
                    && let Some(tool) = self
                        .tools
                        .iter_mut()
                        .rev()
                        .find(|tool| tool.id == id && tool.anchor.message == message)
                {
                    tool.anchor = anchor;
                    tool.block = block_index;
                } else {
                    let mut tool =
                        Tool::new(id, string(block, "name"), &block["arguments"], anchor);
                    tool.block = block_index;
                    if restored {
                        tool.done = true;
                        tool.failed = true;
                        tool.output = "No saved result for this tool call.".into();
                    }
                    self.tools.push(tool);
                }
            } else if block["type"] == "thinking" {
                let index = self
                    .thoughts
                    .iter()
                    .position(|thought| {
                        thought.anchor.message == message && thought.block == block_index
                    })
                    .unwrap_or_else(|| {
                        self.thoughts.push(Thought {
                            anchor: ToolAnchor { message, offset },
                            block: block_index,
                            text: String::new(),
                            expanded: false,
                            done: true,
                            started: None,
                            duration_ms: None,
                        });
                        self.thoughts.len() - 1
                    });
                let thought = &mut self.thoughts[index];
                thought.anchor = ToolAnchor { message, offset };
                thought.text = string(block, "thinking").into();
                thought.done = true;
                thought.duration_ms = thought.duration_ms.or_else(|| {
                    thought
                        .started
                        .map(|start| start.elapsed().as_millis() as u64)
                });
            }
        }
    }

    pub fn apply(&mut self, record: Value) {
        self.last_activity = Instant::now();
        match string(&record, "type") {
            "response" => {
                let command = string(&record, "command");
                let injection = command == "prompt"
                    && self
                        .injection_id
                        .as_ref()
                        .is_some_and(|id| record["id"] == *id);
                if injection {
                    self.injection_id = None;
                    if record["success"] != false {
                        self.pending_injections.pop_front();
                    }
                }
                if record["success"] == false {
                    self.error = Some(string(&record, "error").to_owned());
                    if self.initializing {
                        self.fail_session(self.error.clone().unwrap());
                        return;
                    }
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
                        self.remove_optimistic_prompt(&prompt, index);
                    }
                    if matches!(command, "prompt" | "get_state" | "get_messages") {
                        self.status = Status::Failed;
                        self.started = None;
                        self.complete_task("failed");
                    }
                    return;
                }
                match command {
                    "get_state" => {
                        let data = &record["data"];
                        if !self.initializing || self.selected_model.is_none() {
                            self.update_model(&data["model"]);
                        }
                        if !self.initializing || self.selected_thinking.is_none() {
                            self.thinking = string(data, "thinkingLevel").to_owned();
                        }
                        self.session = data["sessionFile"].as_str().map(str::to_owned);
                        if record["id"] == "state" {
                            self.state_loaded = true;
                            if data["isStreaming"] == true {
                                self.status = Status::Running;
                                self.started.get_or_insert_with(Instant::now);
                            }
                        }
                    }
                    "get_available_models" => {
                        self.models = record["data"]["models"]
                            .as_array()
                            .map(|models| {
                                models.iter().filter_map(AgentModel::from_value).collect()
                            })
                            .unwrap_or_default();
                        self.refresh_thinking_levels();
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
                        self.refresh_thinking_levels();
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
                            self.thoughts.clear();
                            self.work.clear();
                            self.assistant = None;
                            for message in messages {
                                let role = string(message, "role");
                                let index = self.messages.len();
                                if matches!(role, "user" | "assistant") {
                                    let text = content_text(&message["content"]);
                                    if role == "assistant" || !text.is_empty() {
                                        if role == "assistant" {
                                            self.ensure_work();
                                        }
                                        self.messages.push(Message {
                                            role: if role != "user" {
                                                "Pi"
                                            } else if text.starts_with(INJECTION_PREFIX) {
                                                "Shina"
                                            } else {
                                                "You"
                                            },
                                            text,
                                        });
                                    }
                                }
                                if role == "assistant" {
                                    self.anchor_content_work(&message["content"], index, true);
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
                                    tool.duration_ms = message["durationMs"].as_u64();
                                }
                            }
                            // A process can accept a prompt before its response reaches us.
                            // Do not replay it if its unique delivery marker is already saved.
                            self.pending_injections.retain_mut(|injection| {
                                injection.shown = false;
                                !self
                                    .messages
                                    .iter()
                                    .any(|m| m.role == "Shina" && m.text == injection.text)
                            });
                            self.injection_id = None;
                        }
                        if let Some((prompt, index)) = &mut self.queued_prompt {
                            *index = self.messages.len();
                            self.messages.push(Message {
                                role: "You",
                                text: prompt.clone(),
                            });
                        }
                        self.history_loaded = true;
                    }
                    "prompt" => {
                        if !injection
                            && self
                                .pending_prompt
                                .as_ref()
                                .is_some_and(|(id, _, _)| record["id"] == *id)
                        {
                            self.pending_prompt = None;
                        }
                        if record["data"]["disposition"] == "handled"
                            && self.status == Status::Running
                        {
                            self.status = Status::Ready;
                            self.started = None;
                            self.phase.clear();
                            self.complete_task("done");
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
                self.started.get_or_insert_with(Instant::now);
                self.ensure_work();
            }
            // agent_end is deliberately NOT settlement. Pi can still retry or compact.
            "agent_settled" => {
                self.request = None;
                let aborted = record["aborted"] == true;
                self.finish_run();
                // A stopped run is idle, not a finished task.
                if aborted && self.status == Status::Settled {
                    self.status = Status::Ready;
                }
                self.complete_task(if aborted {
                    "cancelled"
                } else {
                    status_word(self.status)
                });
            }
            "message_start" if record["message"]["role"] == "assistant" => {
                self.assistant = None;
                self.request = None;
                self.status = Status::Running;
                self.assistant();
            }
            "message_update" => {
                let update = &record["assistantMessageEvent"];
                if update["type"] == "text_delta" {
                    self.assistant().text.push_str(string(update, "delta"));
                    self.phase = "Writing".into();
                } else if matches!(
                    string(update, "type"),
                    "thinking_start" | "thinking_delta" | "thinking_end"
                ) {
                    let thought =
                        self.thought(update["contentIndex"].as_u64().unwrap_or(0) as usize);
                    thought.started.get_or_insert_with(Instant::now);
                    match string(update, "type") {
                        "thinking_delta" => thought.text.push_str(string(update, "delta")),
                        "thinking_end" => {
                            thought.text = string(update, "content").into();
                            thought.done = true;
                            thought.duration_ms = thought
                                .started
                                .map(|start| start.elapsed().as_millis() as u64);
                        }
                        _ => {}
                    }
                    self.phase = "Thinking".into();
                }
            }
            "message_end" if record["message"]["role"] == "assistant" => {
                let message = &record["message"];
                let text = content_text(&message["content"]);
                self.assistant();
                let index = self.assistant.unwrap();
                if !text.is_empty() {
                    self.assistant().text = text;
                    self.error = None;
                }
                self.anchor_content_work(&message["content"], index, false);
                if message["stopReason"] != "aborted"
                    && let Some(error) = message["errorMessage"].as_str().filter(|s| !s.is_empty())
                {
                    self.error = Some(error.to_owned());
                }
                self.assistant = None;
            }
            "tool_execution_start" => {
                self.phase = format!("Running {}", string(&record, "toolName"));
                let id = string(&record, "toolCallId");
                if let Some(tool) = self.tools.iter_mut().rev().find(|tool| tool.id == id) {
                    tool.started.get_or_insert_with(Instant::now);
                } else {
                    self.assistant();
                    let anchor = ToolAnchor {
                        message: self.assistant.unwrap_or(self.messages.len()),
                        offset: self.assistant.map_or(0, |i| self.messages[i].text.len()),
                    };
                    let mut tool =
                        Tool::new(id, string(&record, "toolName"), &record["args"], anchor);
                    tool.started = Some(Instant::now());
                    self.tools.push(tool);
                }
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
                    .rev()
                    .find(|tool| tool.id == string(&record, "toolCallId"))
                {
                    tool.done = ending;
                    tool.failed = record["isError"] == true;
                    if ending {
                        tool.duration_ms = record["durationMs"].as_u64().or_else(|| {
                            tool.started.map(|start| start.elapsed().as_millis() as u64)
                        });
                    }
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
        self.drive_session();
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
        self.ensure_work().elapsed = self.elapsed;
        self.finish_thoughts();
        self.assistant = None;
        self.phase.clear();
        if self.error.is_none() {
            self.retry_prompt = None;
        }
    }

    fn finish_thoughts(&mut self) {
        for thought in self.thoughts.iter_mut().filter(|thought| !thought.done) {
            thought.done = true;
            thought.duration_ms = thought
                .started
                .map(|start| start.elapsed().as_millis() as u64);
        }
    }
}

pub struct State {
    pub data_dir: PathBuf,
    launch_project: Option<PathBuf>,
    pub threads: Vec<Thread>,
    pub active: usize,
    pub tabs: Vec<usize>,
    pub next_id: usize,
    pub dark: bool,
    /// Whole-window zoom, changed with Ctrl+= / Ctrl+- and reset with Ctrl+0.
    pub zoom: f32,
    pub show_activity: bool,
    /// The open thread's "More actions" menu in the chat header.
    pub thread_menu: bool,
    pub row_menu: Option<RowTarget>,
    pub hovered_row: Option<RowTarget>,
    pub rename_thread: Option<usize>,
    pub rename_edit: TextEdit,
    pub delete_thread: Option<usize>,
    pub project_scope: ProjectScope,
    pub project_picker: Option<ProjectPicker>,
    pub activity_full: bool,
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
    pub updater: crate::update::Updater,
    /// Set by the update button; `poll` restarts once no thread is busy.
    pub restart_for_update: bool,
    task_waiters: Vec<TaskWaiter>,
    /// Recent projects, the switcher menu, and the clone sheet.
    pub projects: crate::projects::Projects,
    pub catalog: Catalog,
    runtime_enabled: bool,
}

impl State {
    pub fn new(workspace: PathBuf, demo: bool) -> Self {
        let workspace = if workspace.as_os_str().is_empty() {
            workspace
        } else {
            workspace.canonicalize().unwrap_or(workspace)
        };
        let artwork = crate::visuals::Artwork::load(&workspace);
        let mut state = Self {
            data_dir: crate::store::directory()
                .unwrap_or_else(|| std::env::temp_dir().join("Shina")),
            launch_project: (!workspace.as_os_str().is_empty()).then_some(workspace.clone()),
            threads: vec![Thread::new(1)],
            active: 0,
            tabs: vec![1],
            next_id: 2,
            dark: false,
            zoom: 1.0,
            show_activity: false,
            thread_menu: false,
            row_menu: None,
            hovered_row: None,
            rename_thread: None,
            rename_edit: TextEdit::default(),
            delete_thread: None,
            project_scope: ProjectScope::All,
            project_picker: None,
            activity_full: false,
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
            updater: Default::default(),
            restart_for_update: false,
            task_waiters: vec![],
            projects: Default::default(),
            catalog: Catalog::default(),
            runtime_enabled: false,
        };
        if !workspace.as_os_str().is_empty() {
            state.projects.entries.push(Project::new(1, workspace));
            state.projects.next_id = 2;
            state.threads[0].project = Some(1);
        } else {
            state.projects.next_id = 1;
        }
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
    pub fn select_thread(&mut self, index: usize) {
        if index >= self.threads.len() {
            return;
        }
        self.active = index;
        let id = self.current().id;
        if !self.tabs.contains(&id) {
            self.tabs.push(id);
        }
        self.current_mut().unread = false;
        self.checkpoint();
    }
    pub fn close_tab(&mut self, id: usize) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.retain(|&tab| tab != id);
        if self.current().id == id
            && let Some(index) = self
                .threads
                .iter()
                .position(|t| Some(&t.id) == self.tabs.last())
        {
            self.select_thread(index);
        }
        self.checkpoint();
    }
    pub fn project(&self, id: Option<ProjectId>) -> Option<&Project> {
        self.projects.entries.iter().find(|p| Some(p.id) == id)
    }
    pub fn is_captain_home(&self, thread: &Thread) -> bool {
        self.project(thread.project)
            .is_some_and(|p| p.captain_thread == Some(thread.id))
    }
    pub fn thread_directory(&self, index: usize) -> Result<PathBuf, String> {
        let thread = self.threads.get(index).ok_or("Thread does not exist.")?;
        crate::backend::BackendHost::resolve(&thread.machine)?;
        match thread.project {
            Some(id) => {
                let project = self
                    .project(Some(id))
                    .ok_or("Thread project does not exist.")?;
                if project.machine != thread.machine {
                    return Err("Thread and project machines do not match.".into());
                }
                Ok(project.path.clone())
            }
            None => Ok(self.data_dir.join("chats").join(thread.id.to_string())),
        }
    }
    pub fn working_directory(&self) -> PathBuf {
        self.thread_directory(self.active)
            .unwrap_or_else(|_| self.data_dir.clone())
    }
    pub fn sorted_projects(&self) -> Vec<ProjectId> {
        let mut projects: Vec<_> = self
            .projects
            .entries
            .iter()
            .filter(|p| !p.hidden || self.show_archive)
            .collect();
        projects.sort_by_key(|p| {
            std::cmp::Reverse((
                self.threads
                    .iter()
                    .filter(|t| t.project == Some(p.id))
                    .map(|t| t.updated)
                    .max()
                    .unwrap_or(p.updated)
                    .max(p.updated),
                p.id,
            ))
        });
        projects.into_iter().map(|p| p.id).collect()
    }
    pub fn grouped_threads(&self, project: Option<ProjectId>) -> Vec<usize> {
        let mut threads: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.project == project && t.archived == self.show_archive)
            .collect();
        threads.sort_by_key(|(_, t)| std::cmp::Reverse((t.pinned, t.updated, t.id)));
        threads.into_iter().map(|(i, _)| i).collect()
    }
    pub fn sidebar_threads(&self) -> Vec<usize> {
        let mut threads: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.archived == self.show_archive && (self.show_archive || !self.is_captain_home(t))
            })
            .filter(|(_, t)| match self.project_scope {
                ProjectScope::All => true,
                ProjectScope::Project(id) => t.project == Some(id),
                ProjectScope::NoProject => t.project.is_none(),
            })
            .collect();
        threads.sort_by_key(|(_, t)| std::cmp::Reverse((t.pinned, t.updated, t.id)));
        threads.into_iter().map(|(i, _)| i).collect()
    }
    pub fn remove_project(&mut self, id: ProjectId) {
        if let Some(project) = self.projects.entries.iter_mut().find(|p| p.id == id) {
            project.hidden = true;
        }
        self.checkpoint();
    }
    pub fn begin_rename(&mut self, id: usize) {
        if let Some(thread) = self.threads.iter().find(|t| t.id == id) {
            self.rename_edit.replace_text(&thread.title);
            self.rename_edit.focused = true;
            self.rename_thread = Some(id);
            self.row_menu = None;
            self.thread_menu = false;
        }
    }
    pub fn save_thread_name(&mut self) {
        let name = self.rename_edit.text.trim();
        if name.is_empty() || name.chars().any(char::is_control) {
            return;
        }
        if let Some(thread) = self
            .threads
            .iter_mut()
            .find(|t| Some(t.id) == self.rename_thread)
        {
            thread.title = name.chars().take(200).collect();
            thread.renamed = true;
        }
        self.rename_thread = None;
        self.checkpoint();
    }
    pub fn request_delete(&mut self, id: usize) {
        self.delete_thread = Some(id);
        self.row_menu = None;
        self.thread_menu = false;
    }
    pub fn deletion_ids(&self, id: usize) -> std::collections::HashSet<usize> {
        let mut ids = std::collections::HashSet::from([id]);
        loop {
            let before = ids.len();
            for thread in &self.threads {
                if thread.parent.is_some_and(|parent| ids.contains(&parent)) {
                    ids.insert(thread.id);
                }
            }
            if ids.len() == before {
                return ids;
            }
        }
    }
    /// Explicit confirmation deletes the thread and its delegated descendants, never files.
    pub fn confirm_delete(&mut self) {
        let Some(id) = self.delete_thread.take() else {
            return;
        };
        let ids = self.deletion_ids(id);
        let current = self.current().id;
        let mut parents = Vec::new();
        for thread in &mut self.threads {
            if ids.contains(&thread.id) {
                let _ = thread.stop_task();
                thread.disconnect();
                if let Some(parent) = thread.parent.filter(|id| !ids.contains(id)) {
                    parents.push((parent, thread.id));
                }
                if self.runtime_enabled
                    && let Ok(server) = crate::mcp::server(self.wake.clone())
                {
                    server.revoke(thread.id);
                }
            }
        }
        self.threads.retain(|t| !ids.contains(&t.id));
        self.tabs.retain(|id| !ids.contains(id));
        for project in &mut self.projects.entries {
            if project.captain_thread.is_some_and(|id| ids.contains(&id)) {
                project.captain_thread = None;
            }
        }
        for (parent, id) in parents {
            if let Some(thread) = self.threads.iter_mut().find(|t| t.id == parent) {
                thread.enqueue_injection(format!(
                    "Shina: task #{id} was deleted by the user. Status: cancelled."
                ));
            }
        }
        let waiters = std::mem::take(&mut self.task_waiters);
        for waiter in waiters {
            if ids.contains(&waiter.thread) {
                let _ = waiter
                    .reply
                    .send(Err("The task was deleted by the user.".into()));
            } else {
                self.task_waiters.push(waiter);
            }
        }
        self.active = self
            .threads
            .iter()
            .position(|t| t.id == current)
            .or_else(|| self.threads.iter().position(|t| self.tabs.contains(&t.id)))
            .unwrap_or(0);
        if self.threads.is_empty() {
            self.new_thread_in(None);
        } else {
            self.select_thread(self.active);
        }
        self.checkpoint();
    }
    pub fn pin_thread(&mut self, id: usize) {
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.pinned = !thread.pinned;
        }
        self.checkpoint();
    }
    pub fn archive_thread(&mut self, id: usize) {
        if let Some(index) = self.threads.iter().position(|t| t.id == id) {
            let previous = self.active;
            self.active = index;
            self.archive();
            if let Some(thread) = self.threads.get(previous)
                && !thread.archived
                && previous != index
            {
                self.active = previous;
            }
        }
        self.checkpoint();
    }
    pub fn choose_project_captain(&mut self, id: ProjectId) {
        if let Some(captain) = self.project(Some(id)).and_then(|p| p.captain) {
            self.projects.captain_target = Some(id);
            self.open_captain(captain);
        } else {
            self.projects.captain_target = Some(id);
            self.captain_picker = true;
        }
    }
    pub fn new_thread(&mut self) {
        self.new_thread_in(self.current().project);
    }
    pub fn new_thread_in(&mut self, project: Option<ProjectId>) {
        if project.is_some() && self.project(project).is_none() {
            return;
        }
        let mut thread = Thread::new(self.next_id);
        thread.project = project;
        if let Some(project) = self.project(project) {
            thread.captain = project.captain;
            thread.machine = project.machine.clone();
        }
        self.threads.push(thread);
        self.next_id += 1;
        self.active = self.threads.len() - 1;
        self.tabs.push(self.current().id);
        self.show_archive = false;
        self.project_picker = None;
        if match self.project_scope {
            ProjectScope::All => false,
            ProjectScope::Project(id) => project != Some(id),
            ProjectScope::NoProject => project.is_some(),
        } {
            self.project_scope = ProjectScope::All;
        }
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
        self.checkpoint();
    }
    pub fn open_captain(&mut self, captain: usize) {
        if captain >= self.captains.len() {
            return;
        }
        let Some(project_id) = self.projects.captain_target.or(self.current().project) else {
            self.captain_picker = false;
            return;
        };
        let Some(project) = self.project(Some(project_id)) else {
            return;
        };
        let captain = project.captain.unwrap_or(captain);
        let home = project.captain_thread;
        let machine = project.machine.clone();
        if let Some(index) = self
            .threads
            .iter()
            .position(|thread| Some(thread.id) == home)
        {
            self.active = index;
            self.current_mut().archived = false;
            self.current_mut().unread = false;
        } else {
            let mut thread = Thread::new(self.next_id);
            self.next_id += 1;
            thread.title = self.captains[captain].name.clone();
            thread.captain = Some(captain);
            thread.captain_home = true;
            thread.project = Some(project_id);
            thread.machine = machine;
            self.threads.push(thread);
            self.active = self.threads.len() - 1;
        }
        let home = self.current().id;
        let project = self
            .projects
            .entries
            .iter_mut()
            .find(|p| p.id == project_id)
            .unwrap();
        project.captain = Some(captain);
        project.captain_thread = Some(home);
        self.show_archive = false;
        self.captain_picker = false;
        self.projects.captain_target = None;
        self.select_thread(self.active);
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
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
                        "That captain name is already in the roster. Choose a different name."
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
                if thread.captain_home && !thread.renamed {
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
        self.project_captain_status(
            self.projects.captain_target.or(self.current().project),
            captain,
        )
    }
    pub fn project_captain_status(&self, project: Option<ProjectId>, captain: usize) -> Status {
        self.threads
            .iter()
            .filter(|thread| {
                thread.project == project && thread.captain == Some(captain) && !thread.archived
            })
            .map(|thread| thread.status)
            .max_by_key(|status| status.urgency())
            .unwrap_or(Status::Ready)
    }
    /// Steps the zoom one notch in (`1`), out (`-1`), or back to 100% (`0`).
    pub fn step_zoom(&mut self, direction: i32) {
        let index = ZOOM_STEPS
            .iter()
            .position(|&z| (z - self.zoom).abs() < 0.001)
            .unwrap_or(4);
        self.zoom = match direction {
            0 => 1.0,
            d if d > 0 => ZOOM_STEPS[(index + 1).min(ZOOM_STEPS.len() - 1)],
            _ => ZOOM_STEPS[index.saturating_sub(1)],
        };
        self.checkpoint();
    }
    /// Connects one thread's Pi with its captain's instructions and its Shina MCP link.
    pub fn connect_thread(&mut self, index: usize) {
        if self.demo || index >= self.threads.len() {
            return;
        }
        let path = match self.thread_directory(index) {
            Ok(path) => path,
            Err(error) => {
                self.threads[index].fail_session(error);
                return;
            }
        };
        if self.threads[index].project.is_none()
            && let Err(error) = std::fs::create_dir_all(&path)
        {
            self.threads[index].fail_session(format!("Could not create the chat folder: {error}"));
            return;
        }
        let wake = self.wake.clone();
        let thread_id = self.threads[index].id;
        let instructions = self.threads[index]
            .captain
            .and_then(|id| self.captains.get(id))
            .map(Captain::prompt);
        // Future project notes can augment this prompt using (thread.machine, thread.project).
        // Context storage is deliberately separate from persona and session persistence.
        // Shina's tools are an addition. If the MCP cannot start, Pi still works without them.
        let host = crate::mcp::server(wake.clone()).and_then(|server| {
            Ok(HostLink {
                url: server.url.clone(),
                token: server.token_for(thread_id),
                extension: crate::mcp::extension_path()?,
            })
        });
        let thread = &mut self.threads[index];
        match host {
            Ok(host) => thread.host = Some(host),
            Err(error) => {
                thread.host = None;
                thread.diagnostics.push(error);
            }
        }
        thread.connect_with_instructions(path, wake, instructions);
    }
    /// Answers a Shina MCP tool call on the UI thread. `caller` is the calling thread's id.
    fn mcp_call(&mut self, caller: usize, tool: &str, args: &Value) -> Result<Value, String> {
        if matches!(tool, "delegate_task" | "task_status") {
            wait_seconds(args)?;
        }
        let caller_index = self
            .threads
            .iter()
            .position(|t| t.id == caller)
            .ok_or("The calling thread no longer exists.")?;
        let find = |state: &Self| -> Result<usize, String> {
            let id = args["thread_id"]
                .as_u64()
                .ok_or("thread_id must be a thread id from list_threads.")?;
            state
                .threads
                .iter()
                .position(|t| t.id as u64 == id)
                .ok_or_else(|| format!("There is no thread {id}."))
        };
        let captain_name = |state: &Self, captain: Option<usize>| -> Value {
            captain
                .and_then(|id| state.captains.get(id))
                .map(|c| Value::from(c.name.clone()))
                .unwrap_or(Value::Null)
        };
        match tool {
            "list_threads" => {
                let archived = args["include_archived"] == true;
                let all = args["all_projects"] == true;
                let project = self.threads[caller_index].project;
                let threads: Vec<Value> = self
                    .threads
                    .iter()
                    .filter(|t| (archived || !t.archived) && (all || t.project == project))
                    .map(|t| {
                        json!({
                            "thread_id": t.id,
                            "title": t.title,
                            "project": self.project(t.project).map(Project::to_value),
                            "machine": t.machine,
                            "captain": captain_name(self, t.captain),
                            "captain_home": self.is_captain_home(t),
                            "status": status_word(t.status),
                            "archived": t.archived,
                            "delegated": t.delegated,
                            "parent": t.parent,
                            "is_caller": t.id == caller,
                        })
                    })
                    .collect();
                Ok(json!({ "threads": threads }))
            }
            "read_thread" => {
                let thread = &self.threads[find(self)?];
                let limit = args["max_messages"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
                let start = thread.messages.len().saturating_sub(limit);
                let messages: Vec<Value> = thread.messages[start..]
                    .iter()
                    .map(|m| {
                        json!({
                            "role": match m.role { "You" => "user", "Shina" => "system", _ => "assistant" },
                            "text": truncate(m.display_text(), 6000),
                        })
                    })
                    .collect();
                Ok(json!({
                    "thread_id": thread.id,
                    "title": thread.title,
                    "project": self.project(thread.project).map(Project::to_value),
                    "machine": thread.machine,
                    "status": status_word(thread.status),
                    "message_count": thread.messages.len(),
                    "messages": messages,
                }))
            }
            "list_captains" => {
                let captains: Vec<Value> = self
                    .captains
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        json!({
                            "name": c.name,
                            "role": c.role,
                            "status": status_word(self.project_captain_status(self.threads[caller_index].project, i)),
                            "assigned": self.project(self.threads[caller_index].project).is_some_and(|p| p.captain == Some(i)),
                        })
                    })
                    .collect();
                Ok(json!({ "captains": captains }))
            }
            "delegate_task" => {
                if self.threads[caller_index].delegated {
                    return Err(
                        "Delegated threads cannot delegate further. Do the work in this thread."
                            .into(),
                    );
                }
                let prompt = args["prompt"].as_str().map(str::trim).unwrap_or("");
                if prompt.is_empty() {
                    return Err("prompt must describe the task.".into());
                }
                let project = self.threads[caller_index].project;
                let assigned = self.project(project).and_then(|p| p.captain);
                let captain = match args["captain"]
                    .as_str()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                {
                    Some(name) => {
                        if !assigned
                            .is_some_and(|id| self.captains[id].name.eq_ignore_ascii_case(name))
                        {
                            return Err("Children inherit their project's captain. Choose a captain on the project first.".into());
                        }
                        assigned
                    }
                    None => assigned,
                };
                let mut thread = Thread::new(self.next_id);
                thread.project = project;
                thread.machine = self.threads[caller_index].machine.clone();
                self.next_id += 1;
                thread.title = args["title"]
                    .as_str()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .unwrap_or(prompt)
                    .chars()
                    .take(45)
                    .collect();
                thread.captain = captain;
                thread.delegated = true;
                thread.parent = Some(caller);
                thread.arm_task();
                thread.unread = true;
                thread.enqueue_injection(format!("Shina: task from parent #{caller}\n\n{prompt}"));
                let id = thread.id;
                self.threads.push(thread);
                let index = self.threads.len() - 1;
                self.connect_thread(index);
                self.checkpoint();
                self.mcp_call(caller, "task_status", &json!({"thread_id":id}))
            }
            "task_status" => {
                let thread = &self.threads[find(self)?];
                let reply = thread
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "Pi")
                    .map(|m| truncate(&m.text, 6000));
                let status = thread.task_result.unwrap_or_else(|| {
                    if !thread.status.busy() && !thread.pending_injections.is_empty() {
                        "queued"
                    } else {
                        status_word(thread.status)
                    }
                });
                Ok(json!({
                    "thread_id": thread.id,
                    "title": thread.title,
                    "project": self.project(thread.project).map(Project::to_value),
                    "machine": thread.machine,
                    "parent": thread.parent,
                    "captain": captain_name(self, thread.captain),
                    "status": status,
                    "finished": thread.task_result.is_some() || (thread.completion_notified && matches!(thread.status, Status::Settled | Status::Failed)),
                    "phase": thread.phase,
                    "error": thread.error,
                    "latest_reply": reply,
                }))
            }
            "task_cancel" | "task_send" => {
                let index = find(self)?;
                let thread = &mut self.threads[index];
                if !thread.delegated || thread.parent != Some(caller) {
                    return Err(format!(
                        "Only the parent that delegated thread {} may cancel or send to it.",
                        thread.id
                    ));
                }
                if tool == "task_cancel" {
                    thread.stop_task()?;
                } else {
                    let message = args["message"]
                        .as_str()
                        .map(str::trim)
                        .filter(|m| !m.is_empty())
                        .ok_or("message must contain a correction or follow-up brief.")?;
                    if !thread.status.busy() {
                        thread.arm_task();
                        if thread.status == Status::Failed && thread.pi.is_some() {
                            thread.status = Status::Ready;
                            thread.error = None;
                        }
                    }
                    thread.enqueue_injection(format!(
                        "Shina: message from parent #{caller}\n\n{message}"
                    ));
                    if thread.pi.is_none() {
                        self.connect_thread(index);
                    }
                }
                self.mcp_call(
                    caller,
                    "task_status",
                    &json!({"thread_id":self.threads[index].id}),
                )
            }
            _ => Err(format!("Shina has no tool named {tool}.")),
        }
    }

    fn mcp_request(&mut self, request: crate::mcp::Request) {
        let wait = if matches!(request.tool.as_str(), "delegate_task" | "task_status") {
            wait_seconds(&request.args)
        } else {
            Ok(0)
        };
        let result = wait.and_then(|seconds| {
            self.mcp_call(request.thread, &request.tool, &request.args)
                .map(|value| (seconds, value))
        });
        match result {
            Ok((seconds, value)) if seconds > 0 && value["finished"] == false => {
                let deadline = Instant::now() + Duration::from_secs(seconds);
                let (timer, cancel) = mpsc::channel();
                let wake = self.wake.clone();
                std::thread::spawn(move || {
                    if cancel.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        == Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        wake();
                    }
                });
                self.task_waiters.push(TaskWaiter {
                    thread: value["thread_id"].as_u64().expect("task thread id") as usize,
                    deadline,
                    reply: request.reply,
                    _timer: timer,
                });
            }
            Ok((_, value)) => {
                let _ = request.reply.send(Ok(value));
            }
            Err(error) => {
                let _ = request.reply.send(Err(error));
            }
        }
    }

    /// Called after all threads' events are drained, so simultaneous results cannot overwrite
    /// each other. Each finished run contributes one message to its parent's separate queue.
    fn poll_tasks(&mut self, now: Instant) -> bool {
        let mut changed = false;
        let mut completions = vec![];
        for thread in &mut self.threads {
            if !thread.completion_notified
                && let Some(result) = thread.task_result
            {
                thread.completion_notified = true;
                if let Some(parent) = thread.parent {
                    let reply = thread
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.role == "Pi")
                        .map(|m| truncate(&m.text, 6000))
                        .unwrap_or_else(|| "(no reply)".into());
                    completions.push((parent, format!(
                        "Shina: task #{} finished\nTitle: {}\nStatus: {result}\nLast reply:\n{reply}\nError: {}",
                        thread.id, thread.title, thread.error.as_deref().map(|e| truncate(e, 2000)).unwrap_or_else(|| "none".into())
                    )));
                }
            }
        }
        for (parent, text) in completions {
            if let Some(index) = self.threads.iter().position(|t| t.id == parent) {
                self.threads[index].enqueue_injection(text);
                self.threads[index].unread = index != self.active;
                if self.threads[index].pi.is_none() {
                    self.connect_thread(index);
                }
                changed = true;
            }
        }
        let waiters = std::mem::take(&mut self.task_waiters);
        for waiter in waiters {
            // Read through the same formatter, including cancellation's idle UI status.
            let result = self.mcp_call(
                waiter.thread,
                "task_status",
                &json!({"thread_id":waiter.thread}),
            );
            if result.as_ref().is_err()
                || result.as_ref().is_ok_and(|value| value["finished"] == true)
                || now >= waiter.deadline
            {
                let _ = waiter.reply.send(result);
            } else {
                self.task_waiters.push(waiter);
            }
        }
        for thread in &mut self.threads {
            changed |= thread.flush_injection();
        }
        changed
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
        self.current_mut().send();
        if self.current().pi.is_none() && self.current().queued_prompt.is_some() {
            self.connect_thread(self.active);
        }
        self.checkpoint();
    }

    pub fn retry(&mut self) {
        if self.demo || self.current().archived || self.current().status.busy() {
            return;
        }
        if self.current().draft.text.trim().is_empty()
            && let Some(prompt) = self.current().retry_prompt.clone()
        {
            self.current_mut().draft.replace_text(&prompt);
        }
        if self.current().draft.text.trim().is_empty() {
            self.current_mut().disconnect();
            self.current_mut().error = None;
            self.current_mut().status = Status::Ready;
            self.connect_thread(self.active);
        } else {
            self.send();
        }
    }

    pub fn refresh_catalog(&mut self) {
        if !self.demo && self.runtime_enabled {
            self.catalog
                .refresh(self.data_dir.clone(), self.wake.clone());
        }
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
    }

    fn apply_catalog(&mut self) {
        for thread in &mut self.threads {
            thread.models = self.catalog.models.clone();
            if thread.model_id.is_empty()
                && thread.session.is_none()
                && let Some(model) = &self.catalog.default_model
            {
                thread.model_provider = model.provider.clone();
                thread.model_id = model.id.clone();
                thread.model = model.name.clone();
                if thread.selected_thinking.is_none() {
                    thread.thinking = self.catalog.default_thinking.clone();
                }
            }
            if let Some(model) = thread.models.iter().find(|m| m.id == thread.model_id) {
                thread.model = model.name.clone();
            }
            thread.refresh_thinking_levels();
        }
    }

    fn prewarm_current(&mut self) -> bool {
        let thread = self.current();
        if self.runtime_enabled
            && !self.demo
            && !thread.archived
            && thread.pi.is_none()
            && !thread.prewarm_attempted
            && thread.status != Status::Failed
            && (!thread.draft.text.trim().is_empty()
                || (thread.session.is_some() && !thread.history_loaded))
        {
            self.connect_thread(self.active);
            return true;
        }
        false
    }
    pub fn add_project(&mut self, path: PathBuf) -> Result<ProjectId, String> {
        let path = match path.canonicalize() {
            Ok(path) if path.is_dir() => path,
            _ => return Err("That project folder could not be opened.".into()),
        };
        let id = if let Some(project) = self
            .projects
            .entries
            .iter_mut()
            .find(|p| p.path == path && p.machine == "local")
        {
            project.hidden = false;
            project.updated = crate::projects::now();
            project.id
        } else {
            let id = self.projects.next_id.max(1);
            self.projects.next_id = id + 1;
            self.projects.entries.push(Project::new(id, path));
            id
        };
        if let Some(index) = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.project == Some(id) && !t.archived)
            .max_by_key(|(_, t)| (t.updated, t.id))
            .map(|(i, _)| i)
        {
            self.active = index;
            self.current_mut().unread = false;
            self.select_thread(index);
        } else {
            self.new_thread_in(Some(id));
        }
        self.show_archive = false;
        self.checkpoint();
        Ok(id)
    }
    pub fn archive(&mut self) {
        if self.current().status.busy() {
            return;
        }
        let archived = !self.current().archived;
        self.current_mut().archived = archived;
        if archived {
            let id = self.current().id;
            self.tabs.retain(|&tab| tab != id);
            self.current_mut().disconnect();
            if let Some(index) = self.threads.iter().position(|thread| !thread.archived) {
                self.select_thread(index);
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
        if let Err(error) = std::fs::create_dir_all(&self.data_dir) {
            self.storage_error = Some(format!("Could not create Shina's data folder: {error}"));
            return;
        }
        self.load_global(wake.clone());
        self.runtime_enabled = true;
        self.refresh_catalog();
        self.updater.check(wake.clone());
        self.prewarm_current();
    }
    pub fn leave_demo(&mut self) {
        if !self.demo {
            return;
        }
        let mut live = Self::new(std::env::current_dir().unwrap_or_default(), false);
        live.setup(self.wake.clone());
        *self = live;
    }

    fn load_global(&mut self, wake: Waker) {
        let path = self.data_dir.join("index.json");
        match crate::store::load_global(&self.data_dir, self.launch_project.as_deref()) {
            Ok(Some(index)) if index["version"] == 3 => {
                if let Err(error) = self.restore_global(&index) {
                    self.storage_error =
                        Some(format!("{error} The index has not been overwritten."));
                    return;
                }
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
        if let Some(path) = self.launch_project.take()
            && let Err(error) = self.add_project(path)
        {
            self.current_mut().error = Some(error);
        }
        self.checkpoint();
    }

    fn restore_global(&mut self, index: &Value) -> Result<(), String> {
        let Some(profiles) = index["captains"].as_array().filter(|p| !p.is_empty()) else {
            return Err("The saved captain profiles are invalid.".into());
        };
        let captains: Option<Vec<Captain>> = profiles.iter().map(Captain::from_value).collect();
        let Some(captains) = captains else {
            return Err("A saved captain profile is invalid.".into());
        };
        let projects: Option<Vec<_>> = index["projects"]
            .as_array()
            .ok_or("Invalid project list.")?
            .iter()
            .map(Project::from_value)
            .collect();
        let projects = projects.ok_or("Invalid project record.")?;
        let project_ids: std::collections::HashSet<_> = projects.iter().map(|p| p.id).collect();
        if project_ids.len() != projects.len() {
            return Err("Duplicate project ids.".into());
        }
        if let Some(threads) = index["threads"].as_array() {
            let restored: Vec<Thread> = threads
                .iter()
                .filter_map(|record| {
                    let id = usize::try_from(record["id"].as_u64()?).ok()?;
                    if id == 0 || id == usize::MAX {
                        return None;
                    }
                    let mut thread = Thread::new(id);
                    thread.project = if record["project"].is_null() {
                        None
                    } else {
                        Some(usize::try_from(record["project"].as_u64()?).ok()?)
                    };
                    thread.machine = record["machine"].as_str()?.into();
                    thread.updated = record["updated"].as_u64().unwrap_or(0);
                    if thread.machine.is_empty()
                        || thread.project.is_some_and(|id| !project_ids.contains(&id))
                    {
                        return None;
                    }
                    thread.title = record["title"].as_str()?.into();
                    thread.renamed = record["renamed"] == true;
                    thread.session = record["session"].as_str().map(str::to_owned);
                    if let Some(model) = record["model"].as_str() {
                        // Only CPA choices survive restart, even in hand-edited indexes.
                        thread.selected_model = Some(("cpa".into(), model.into()));
                        thread.model_provider = "cpa".into();
                        thread.model_id = model.into();
                        thread.model = model.into();
                    }
                    thread.selected_thinking = record["thinking"].as_str().map(str::to_owned);
                    if let Some(level) = &thread.selected_thinking {
                        thread.thinking = level.clone();
                    }
                    thread.archived = record["archived"] == true;
                    thread.unread = record["unread"] == true;
                    thread.captain = if record["captain"].is_null() {
                        None
                    } else {
                        let id = usize::try_from(record["captain"].as_u64()?).ok()?;
                        if id >= captains.len() || thread.project.is_none() {
                            return None;
                        }
                        Some(id)
                    };
                    thread.captain_home = record["captainHome"] == true;
                    if thread.project.is_none() && thread.captain_home {
                        return None;
                    }
                    thread.pinned = record["pinned"] == true;
                    thread.delegated = record["delegated"] == true;
                    thread.parent = if record["parent"].is_null() {
                        None
                    } else {
                        Some(usize::try_from(record["parent"].as_u64()?).ok()?)
                    };
                    Some(thread)
                })
                .collect();
            let ids: std::collections::HashSet<_> = restored.iter().map(|t| t.id).collect();
            let parents_valid = restored.iter().all(|thread| {
                thread.parent.is_none_or(|parent| {
                    thread.delegated
                        && parent != thread.id
                        && restored.iter().any(|t| {
                            t.id == parent
                                && !t.delegated
                                && t.project == thread.project
                                && t.machine == thread.machine
                        })
                })
            });
            if restored.len() != threads.len() || ids.len() != restored.len() || !parents_valid {
                return Err("The saved index contains invalid or duplicate threads.".into());
            }
            for project in &projects {
                if project.captain.is_some_and(|id| id >= profiles.len())
                    || project.captain_thread.is_some_and(|id| {
                        !restored.iter().any(|t| {
                            t.id == id
                                && t.project == Some(project.id)
                                && t.captain == project.captain
                                && t.captain_home
                                && !t.delegated
                        })
                    })
                    || restored
                        .iter()
                        .any(|t| t.project == Some(project.id) && t.machine != project.machine)
                {
                    return Err("Invalid project captain or machine.".into());
                }
            }
            let next_thread = next_counter(
                &index["nextId"],
                restored.iter().map(|t| t.id).max().unwrap_or(0),
            )?;
            let next_project = next_counter(
                &index["nextProjectId"],
                projects.iter().map(|p| p.id).max().unwrap_or(0),
            )?;
            self.threads = restored;
            self.captains = captains;
            self.projects.entries = projects;
            self.projects.next_id = next_project;
            self.next_id = next_thread;
            self.active = self
                .threads
                .iter()
                .position(|t| Some(t.id as u64) == index["active"].as_u64())
                .or_else(|| self.threads.iter().position(|t| !t.archived))
                .unwrap_or(0);
            if self.threads.is_empty() {
                self.threads.push(Thread::new(self.next_id));
                self.next_id += 1;
            }
            self.tabs = index["tabs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .filter_map(|id| usize::try_from(id).ok())
                .filter(|id| self.threads.iter().any(|t| t.id == *id && !t.archived))
                .collect();
            let mut seen_tabs = std::collections::HashSet::new();
            self.tabs.retain(|id| seen_tabs.insert(*id));
            if !self.tabs.contains(&self.current().id) {
                self.tabs.push(self.current().id);
            }
        } else {
            return Err("The saved index has no valid thread list.".into());
        }
        self.dark = index["dark"] == true;
        if let Some(zoom) = index["zoom"].as_f64() {
            self.zoom = snap_zoom(zoom as f32);
        }
        self.projects.recents.clone_parent = index["cloneParent"].as_str().map(PathBuf::from);
        self.project_scope = match index["projectScope"].as_str() {
            Some("none") => ProjectScope::NoProject,
            _ => index["projectScope"]
                .as_u64()
                .and_then(|id| usize::try_from(id).ok())
                .filter(|id| project_ids.contains(id))
                .map(ProjectScope::Project)
                .unwrap_or(ProjectScope::All),
        };
        Ok(())
    }
    pub fn checkpoint(&mut self) {
        if self.demo {
            return;
        }
        let snapshot = self.snapshot();
        if let Some(store) = &mut self.store {
            store.save(snapshot);
        }
    }
    fn snapshot(&self) -> Value {
        json!({
            "version":3, "dark":self.dark, "zoom":self.zoom, "active":self.current().id, "tabs":self.tabs,
            "nextId":self.next_id, "nextProjectId":self.projects.next_id,
            "projectScope":match self.project_scope {
                ProjectScope::All => json!("all"),
                ProjectScope::NoProject => json!("none"),
                ProjectScope::Project(id) => json!(id),
            },
            "projects": self.projects.entries.iter().map(Project::to_value).collect::<Vec<_>>(),
            "cloneParent": self.projects.recents.clone_parent,
            "captains":self.captains.iter().map(Captain::to_value).collect::<Vec<_>>(),
            "threads":self.threads.iter().map(|thread|json!({
                "id":thread.id, "title":thread.title, "renamed":thread.renamed, "session":thread.session, "archived":thread.archived,
                "captain":thread.captain, "captainHome":thread.captain_home, "pinned":thread.pinned, "delegated":thread.delegated,
                "parent":thread.parent,
                "project":thread.project, "machine":thread.machine, "updated":thread.updated, "unread":thread.unread,
                "model":thread.selected_model.as_ref().map(|(_, id)|id), "thinking":thread.selected_thinking
            })).collect::<Vec<_>>()
        })
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
        thread.work.push(AssistantWork {
            start: 1,
            expanded: true,
            elapsed: Some(42),
        });
        thread.messages = vec![
            Message { role: "You", text: "Fix the reconnect race, and add a regression test for the duplicate listener.".into() },
            Message { role: "Pi", text: "I'll check the socket lifecycle and where the reconnect listener attaches before making the change.".into() },
            Message { role: "Pi", text: "The reconnect listener attaches too late. I'll move it before `socket.open()` and cover the initialization race.".into() },
            Message { role: "Pi", text: "The listener now attaches before the socket opens. Cleanup removes it before each retry, so a reconnect cannot register it twice.\n\n```diff\n- socket.open();\n  socket.on(\"reconnect\", reconnect);\n+ socket.open();\n```\n\nThe regression test passes. Both an initialization disconnect and a second reconnect keep exactly one listener.".into() },
        ];
        thread.tools = vec![
            Tool {
                id: "example-1".into(),
                name: "read".into(),
                detail: "src/transport/socket.ts".into(),
                done: true,
                failed: false,
                output: "Example activity, not a real tool execution.".into(),
                anchor: ToolAnchor { message: 2, offset: 0 },
                block: 0,
                expanded: false,
                group_expanded: false,
                started: None,
                duration_ms: Some(180),
            },
            Tool {
                id: "example-2".into(),
                name: "read".into(),
                detail: "tests/reconnect.test.ts".into(),
                done: true,
                failed: false,
                output: "Example activity, not a real tool execution.".into(),
                anchor: ToolAnchor { message: 2, offset: 0 },
                block: 1,
                expanded: false,
                group_expanded: false,
                started: None,
                duration_ms: Some(120),
            },
            Tool {
                id: "example-3".into(),
                name: "edit".into(),
                detail: "src/transport/socket.ts".into(),
                done: true,
                failed: false,
                output: "Example output\nReplaced 2 lines in src/transport/socket.ts.".into(),
                anchor: ToolAnchor { message: 3, offset: 0 },
                block: 0,
                expanded: false,
                group_expanded: true,
                started: None,
                duration_ms: Some(240),
            },
            Tool {
                id: "example-4".into(),
                name: "powershell".into(),
                detail: "npm test -- reconnect.test.ts".into(),
                done: true,
                failed: false,
                output: "Example output\nPASS tests/reconnect.test.ts\n  reconnect lifecycle\n    attaches before opening the socket\n    removes the listener before retrying\n\nTests: 2 passed, 2 total".into(),
                anchor: ToolAnchor { message: 3, offset: 0 },
                block: 1,
                expanded: false,
                group_expanded: false,
                started: None,
                duration_ms: Some(1100),
            },
        ];
        thread.thoughts.push(Thought {
            anchor: ToolAnchor { message: 2, offset: 0 },
            block: 2,
            text: "Example thinking, not a real model response.\n\nThe listener must attach before socket.open() can emit a reconnect. Cleanup must remove the old listener before another attempt.".into(),
            expanded: false,
            done: true,
            started: None,
            duration_ms: Some(18000),
        });
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
        self.projects = crate::projects::Projects::demo();
        self.projects.entries = ["shina", "rune", "ksgcode"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let mut project = Project::new(i + 1, PathBuf::from(format!(r"D:\AiStack\{name}")));
                project.captain = Some(i);
                project.captain_thread = Some(i + 7);
                project.updated = crate::projects::now().saturating_sub(i as u64 * 120);
                project
            })
            .collect();
        self.projects.next_id = 4;
        for thread in &mut self.threads {
            thread.project = match thread.id {
                1 | 2 | 7 => Some(1),
                3 | 4 | 8 => Some(2),
                5 | 9 => Some(3),
                _ => None,
            };
            thread.captain = thread.project.map(|id| id - 1);
            thread.captain_home = matches!(thread.id, 7..=9);
            thread.updated = crate::projects::now().saturating_sub(thread.id as u64 * 60);
        }
        self.threads[9].title = "Plan a weekend tool".into();
        self.threads[1].unread = true;
        self.tabs = vec![1, 3, 10];
    }
}

pub fn poll(state: &mut State, _: f64, tx: &mut Tx) -> bool {
    let mut changed = state.updater.poll();
    changed |= state.poll_projects();
    if state.catalog.poll() {
        state.apply_catalog();
        changed = true;
    }
    if let Some(remaining) = state.catalog.remaining() {
        tx.wake_after(remaining);
    }
    changed |= state.prewarm_current();
    if !state.demo
        && let Ok(server) = crate::mcp::server(state.wake.clone())
    {
        while let Some(request) = server.next_request() {
            state.mcp_request(request);
            changed = true;
        }
    }
    if state.restart_for_update {
        state.restart_for_update = false;
        changed = true;
        if state.threads.iter().any(|t| t.status.busy()) {
            state.current_mut().error =
                Some("Stop running threads before restarting to update.".into());
        } else {
            state.checkpoint();
            match state.updater.install_and_relaunch() {
                Ok(()) => tx.quit(),
                Err(error) => state.current_mut().error = Some(error),
            }
        }
    }
    for (index, thread) in state.threads.iter_mut().enumerate() {
        // Batch incoming deltas into one view update, with a bounded per-frame drain.
        for _ in 0..256 {
            let event = thread.pi.as_ref().and_then(|pi| pi.events.try_recv().ok());
            let Some(event) = event else { break };
            changed = true;
            thread.updated = crate::projects::now();
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
                    thread.fail_session(error);
                }
                Event::Diagnostic(diagnostic) => thread.diagnostics.push(diagnostic),
                Event::Exited => {
                    thread.fail_session(
                        "Pi stopped unexpectedly. Retry to resume the saved conversation.".into(),
                    );
                }
            }
        }
        thread.drive_session();
        if thread
            .session_started
            .is_some_and(|at| at.elapsed() >= START_TIMEOUT)
        {
            thread.fail_session(
                "Pi did not finish preparing the session. Retry to resume it.".into(),
            );
            changed = true;
        }
        changed |= thread.reclaim_idle(Instant::now());
        // A quiet RPC stream must not freeze the visible work/thinking clocks.
        if index == state.active && thread.status.busy() {
            tx.wake_after(Duration::from_secs(1));
            changed = true;
        }
        if thread.pi.is_some() {
            let remaining = if let Some(started) = thread.session_started {
                START_TIMEOUT.saturating_sub(started.elapsed())
            } else if !thread.status.busy() && !thread.configuring {
                IDLE_TIMEOUT.saturating_sub(thread.last_activity.elapsed())
            } else {
                IDLE_TIMEOUT
            };
            tx.wake_after(remaining);
        }
    }
    changed |= state.poll_tasks(Instant::now());
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

pub const ZOOM_STEPS: [f32; 10] = [0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

fn next_counter(saved: &Value, maximum: usize) -> Result<usize, String> {
    let minimum = maximum.checked_add(1).ok_or("Invalid id counter.")?;
    let next = if saved.is_null() {
        minimum
    } else {
        usize::try_from(saved.as_u64().ok_or("Invalid id counter.")?)
            .map_err(|_| "Invalid id counter.")?
            .max(minimum)
    };
    if next == usize::MAX {
        return Err("Thread or project ids exhausted.".into());
    }
    Ok(next)
}

fn wait_seconds(args: &Value) -> Result<u64, String> {
    match args.get("wait_seconds") {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|&seconds| seconds <= 600)
            .ok_or_else(|| "wait_seconds must be an integer from 0 to 600.".into()),
    }
}

/// The closest supported zoom step, so a hand-edited index cannot pick an odd scale.
pub fn snap_zoom(zoom: f32) -> f32 {
    ZOOM_STEPS
        .iter()
        .copied()
        .min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs()))
        .unwrap_or(1.0)
}

/// The words the Shina MCP uses for a thread's status.
fn status_word(status: Status) -> &'static str {
    match status {
        Status::Ready => "idle",
        Status::Running => "working",
        Status::Waiting => "needs input",
        Status::Failed => "failed",
        Status::Settled => "done",
    }
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

    fn fake_pi(
        thread: &mut Thread,
        status: Status,
    ) -> (mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
        let (pi, commands, events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.status = status;
        thread.model_provider = "cpa".into();
        (commands, events)
    }

    fn preparing_pi(thread: &mut Thread) -> (mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
        let result = fake_pi(thread, Status::Ready);
        thread.initializing = true;
        thread.state_loaded = false;
        thread.history_loaded = false;
        result
    }

    fn child(state: &mut State) -> (usize, mpsc::Receiver<Value>, mpsc::SyncSender<Event>) {
        let index = state.threads.len();
        let mut thread = Thread::new(state.next_id);
        state.next_id += 1;
        thread.title = format!("Child {}", thread.id);
        thread.delegated = true;
        thread.parent = Some(state.threads[0].id);
        thread.project = state.threads[0].project;
        thread.machine = state.threads[0].machine.clone();
        thread.arm_task();
        let (commands, events) = fake_pi(&mut thread, Status::Running);
        state.threads.push(thread);
        (index, commands, events)
    }

    fn acknowledge(thread: &mut Thread, prompt: &Value) {
        thread.apply(json!({"type":"response","id":prompt["id"],"command":"prompt","success":true,"data":{"disposition":"queued"}}));
    }

    #[test]
    fn simultaneous_completions_are_delivered_exactly_once_to_idle_or_busy_parents() {
        for status in [
            Status::Ready,
            Status::Settled,
            Status::Running,
            Status::Waiting,
        ] {
            let mut state = State::new(".".into(), false);
            let (commands, _parent_events) = fake_pi(&mut state.threads[0], status);
            state.threads[0].draft.replace_text("Unsent user draft");
            let (first, _first_commands, first_events) = child(&mut state);
            let (second, _second_commands, second_events) = child(&mut state);
            for events in [&first_events, &second_events] {
                events.send(Event::Record(json!({"type":"message_end","message":{"role":"assistant","content":"PONG"}}))).unwrap();
                events
                    .send(Event::Record(
                        json!({"type":"agent_settled","aborted":false}),
                    ))
                    .unwrap();
            }
            poll(&mut state, 0.0, &mut Tx::default());
            assert_eq!(state.threads[0].pending_injections.len(), 2);
            let first_prompt = commands.try_recv().unwrap();
            assert_eq!(first_prompt["type"], "prompt");
            assert_eq!(first_prompt["streamingBehavior"], "followUp");
            assert!(
                first_prompt["message"]
                    .as_str()
                    .unwrap()
                    .contains("task #2 finished")
            );
            assert!(first_prompt["message"].as_str().unwrap().contains("PONG"));
            assert!(
                commands.try_recv().is_err(),
                "await acceptance before the next injection"
            );
            acknowledge(&mut state.threads[0], &first_prompt);
            poll(&mut state, 0.0, &mut Tx::default());
            let second_prompt = commands.try_iter().find(|c| c["type"] == "prompt").unwrap();
            assert!(
                second_prompt["message"]
                    .as_str()
                    .unwrap()
                    .contains("task #3 finished")
            );
            acknowledge(&mut state.threads[0], &second_prompt);
            for index in [first, second] {
                state.threads[index].apply(json!({"type":"agent_settled","aborted":false}));
            }
            poll(&mut state, 0.0, &mut Tx::default());
            assert!(state.threads[0].pending_injections.is_empty());
            assert!(commands.try_iter().all(|c| c["type"] != "prompt"));
            assert_eq!(state.threads[0].draft.text, "Unsent user draft");
            assert_eq!(
                state.threads[0]
                    .messages
                    .iter()
                    .filter(|m| m.role == "Shina")
                    .count(),
                2
            );
        }
    }

    #[test]
    fn completion_waits_for_parent_handshake_without_sending_its_draft() {
        let mut state = State::new(".".into(), false);
        let (commands, events) = preparing_pi(&mut state.threads[0]);
        state.threads[0].draft.replace_text("Keep typing");
        let (index, _commands, _events) = child(&mut state);
        state.threads[index].apply(json!({"type":"agent_settled","aborted":false}));
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.threads[0].pending_injections.len(), 1);
        assert!(commands.try_recv().is_err());
        events.send(Event::Record(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}))).unwrap();
        events.send(Event::Record(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false,"model":{"provider":"cpa","id":"test"}}}))).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        let prompt = commands.try_recv().unwrap();
        assert!(
            prompt["message"]
                .as_str()
                .unwrap()
                .contains("task #2 finished")
        );
        assert_eq!(state.threads[0].draft.text, "Keep typing");
    }

    #[test]
    fn failures_and_error_exits_wake_once_with_truncated_results() {
        for exit in [false, true] {
            let mut state = State::new(".".into(), false);
            let (commands, _events) = fake_pi(&mut state.threads[0], Status::Ready);
            let (index, _commands, events) = child(&mut state);
            state.threads[index].messages.push(Message {
                role: "Pi",
                text: "界".repeat(6100),
            });
            if exit {
                events
                    .send(Event::Error("Pi exited with code 1".into()))
                    .unwrap();
                events.send(Event::Exited).unwrap();
            } else {
                events.send(Event::Record(json!({"type":"response","command":"prompt","success":false,"error":"Rejected"}))).unwrap();
            }
            poll(&mut state, 0.0, &mut Tx::default());
            let prompt = commands.try_recv().unwrap();
            let text = prompt["message"].as_str().unwrap();
            assert!(text.contains("Status: failed"));
            assert!(text.contains(if exit {
                "Pi exited with code 1"
            } else {
                "Rejected"
            }));
            assert!(text.contains(&format!("{}...", "界".repeat(6000))));
            acknowledge(&mut state.threads[0], &prompt);
            poll(&mut state, 0.0, &mut Tx::default());
            assert!(commands.try_iter().all(|c| c["type"] != "prompt"));
        }
    }

    #[test]
    fn task_waits_park_finish_and_timeout_without_blocking() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, _commands, events) = child(&mut state);
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":2,"wait_seconds":600}),
            reply,
        });
        assert_eq!(state.task_waiters.len(), 1);
        assert!(answer.try_recv().is_err());
        state.poll_tasks(Instant::now());
        assert!(answer.try_recv().is_err());
        events
            .send(Event::Record(
                json!({"type":"agent_settled","aborted":false}),
            ))
            .unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        let result = answer.try_recv().unwrap().unwrap();
        assert_eq!(result["finished"], true);
        assert_eq!(result["parent"], 1);
        assert!(state.task_waiters.is_empty());

        state.threads[index].arm_task();
        state.threads[index].status = Status::Running;
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":2,"wait_seconds":10}),
            reply,
        });
        let deadline = state.task_waiters[0].deadline;
        state.poll_tasks(deadline);
        assert_eq!(answer.try_recv().unwrap().unwrap()["finished"], false);
        assert!(state.task_waiters.is_empty());
        for value in [json!(-1), json!(601), json!(1.5), json!("1"), Value::Null] {
            assert!(
                state
                    .mcp_call(
                        1,
                        "delegate_task",
                        &json!({"prompt":"No task","wait_seconds":value})
                    )
                    .is_err()
            );
        }
        assert_eq!(
            state.threads.len(),
            2,
            "invalid waits must not start children"
        );
    }

    #[test]
    fn delegate_wait_parks_and_queues_the_result_for_a_disconnected_parent() {
        let mut state = State::new(".".into(), false);
        // Prevent a real subprocess in this unit test; connect_thread is tested live below.
        state.demo = true;
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "delegate_task".into(),
            args: json!({"prompt":"Reply PONG","title":"Ping","wait_seconds":600}),
            reply,
        });
        assert_eq!(state.threads[1].parent, Some(1));
        assert!(state.threads[1].draft.text.is_empty());
        assert_eq!(state.threads[1].pending_injections.len(), 1);
        assert!(answer.try_recv().is_err());
        assert_eq!(state.task_waiters.len(), 1);
        state.threads[1].apply(json!({"type":"agent_settled","aborted":false}));
        state.poll_tasks(Instant::now());
        assert_eq!(answer.try_recv().unwrap().unwrap()["finished"], true);
        assert_eq!(state.threads[0].pending_injections.len(), 1);
        let (commands, events) = preparing_pi(&mut state.threads[0]);
        events.send(Event::Record(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false,"model":{"provider":"cpa","id":"test"}}}))).unwrap();
        events.send(Event::Record(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}))).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        assert!(
            commands.try_recv().unwrap()["message"]
                .as_str()
                .unwrap()
                .contains("Ping")
        );
    }

    #[test]
    fn only_a_childs_parent_can_cancel_or_send_and_cancellation_finishes_waits() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, commands, events) = child(&mut state);
        let mut stranger = Thread::new(3);
        stranger.status = Status::Ready;
        state.threads.push(stranger);
        for caller in [2, 3] {
            for tool in ["task_cancel", "task_send"] {
                let error = state
                    .mcp_call(caller, tool, &json!({"thread_id":2,"message":"Correction"}))
                    .unwrap_err();
                assert!(error.contains("Only the parent"));
            }
        }
        assert!(
            state
                .mcp_call(1, "task_cancel", &json!({"thread_id":3}))
                .is_err()
        );
        assert!(commands.try_recv().is_err());
        state
            .mcp_call(
                1,
                "task_send",
                &json!({"thread_id":2,"message":"Correction"}),
            )
            .unwrap();
        state.poll_tasks(Instant::now());
        let prompt = commands.try_recv().unwrap();
        assert_eq!(prompt["streamingBehavior"], "followUp");
        assert!(prompt["message"].as_str().unwrap().contains("Correction"));
        acknowledge(&mut state.threads[index], &prompt);
        assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":2,"wait_seconds":600}),
            reply,
        });
        state
            .mcp_call(1, "task_cancel", &json!({"thread_id":2}))
            .unwrap();
        assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
        assert_eq!(commands.try_recv().unwrap()["type"], "abort");
        events
            .send(Event::Record(
                json!({"type":"agent_settled","aborted":true}),
            ))
            .unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        let result = answer.try_recv().unwrap().unwrap();
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result["finished"], true);
        assert_eq!(state.threads[index].status, Status::Ready);
        assert!(
            state
                .mcp_call(2, "delegate_task", &json!({"prompt":"Nested"}))
                .unwrap_err()
                .contains("cannot delegate")
        );
        state
            .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"Retry"}))
            .unwrap();
        state.poll_tasks(Instant::now());
        assert_eq!(state.threads[index].status, Status::Running);
        assert_eq!(
            state
                .mcp_call(1, "task_status", &json!({"thread_id":2}))
                .unwrap()["finished"],
            false
        );
        assert!(commands.try_iter().any(|c| c["type"] == "prompt"));
    }

    #[test]
    fn restored_injections_are_shina_messages_and_rejections_never_change_drafts() {
        let mut thread = Thread::new(1);
        let (commands, _events) = fake_pi(&mut thread, Status::Ready);
        thread.draft.replace_text("User draft");
        thread.enqueue_injection("Shina: task #2 finished".into());
        thread.flush_injection();
        let prompt = commands.try_recv().unwrap();
        thread.apply(json!({"type":"response","id":prompt["id"],"command":"prompt","success":false,"error":"Rejected"}));
        assert_eq!(thread.draft.text, "User draft");
        assert_eq!(thread.pending_injections.len(), 1);
        thread.status = Status::Ready;
        thread.flush_injection();
        assert_eq!(
            thread.messages.len(),
            1,
            "a retry must not duplicate its transcript entry"
        );
        let prompt = commands.try_recv().unwrap();
        acknowledge(&mut thread, &prompt);
        let history = json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":prompt["message"]},
            {"role":"assistant","content":"Acknowledged"}
        ]}});
        thread.apply(history);
        assert_eq!(thread.messages[0].role, "Shina");
        assert_eq!(thread.messages[1].role, "Pi");
        assert_eq!(thread.draft.text, "User draft");
    }

    #[test]
    fn reconnect_reconciles_an_unacknowledged_injection_against_saved_history() {
        for accepted in [true, false] {
            let mut thread = Thread::new(1);
            let (commands, _events) = fake_pi(&mut thread, Status::Ready);
            thread.draft.replace_text("Keep this draft");
            thread.enqueue_injection("Shina: task #2 finished".into());
            thread.flush_injection();
            let prompt = commands.try_recv().unwrap();
            thread.disconnect();
            let (commands, _events) = preparing_pi(&mut thread);
            let messages = if accepted {
                json!([{"role":"user","content":prompt["message"]}])
            } else {
                json!([])
            };
            thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":messages}}));
            thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false,"model":{"provider":"cpa","id":"test"}}}));
            assert_eq!(thread.flush_injection(), !accepted);
            assert_eq!(commands.try_recv().is_ok(), !accepted);
            assert_eq!(thread.messages.len(), 1);
            assert_eq!(thread.messages[0].display_text(), "Shina: task #2 finished");
            assert_eq!(thread.draft.text, "Keep this draft");
        }
    }

    #[test]
    fn a_wait_deadline_wakes_an_otherwise_idle_ui() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (_index, _commands, _events) = child(&mut state);
        let (wake, woken) = mpsc::channel();
        state.wake = Arc::new(move || {
            let _ = wake.send(());
        });
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":2,"wait_seconds":1}),
            reply,
        });
        assert!(answer.try_recv().is_err());
        woken
            .recv_timeout(Duration::from_secs(3))
            .expect("the deadline must wake the UI without Pi events");
        state.poll_tasks(Instant::now());
        assert_eq!(answer.try_recv().unwrap().unwrap()["finished"], false);
        assert!(state.task_waiters.is_empty());
    }

    #[test]
    fn cancelling_a_starting_child_discards_its_pending_brief() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, commands, _events) = child(&mut state);
        state.threads[index].initializing = true;
        state.threads[index].enqueue_injection("Shina: task brief".into());
        let result = state
            .mcp_call(1, "task_cancel", &json!({"thread_id":2}))
            .unwrap();
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result["finished"], true);
        assert!(state.threads[index].pi.is_none());
        assert!(state.threads[index].pending_injections.is_empty());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn sending_to_a_stopping_child_waits_for_abort_then_starts_a_fresh_run() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, commands, _events) = child(&mut state);
        state
            .mcp_call(1, "task_cancel", &json!({"thread_id":2}))
            .unwrap();
        assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
        assert_eq!(commands.try_recv().unwrap()["type"], "abort");
        state
            .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"New work"}))
            .unwrap();
        state.poll_tasks(Instant::now());
        assert!(
            commands.try_recv().is_err(),
            "new work must not enter the queue being cleared"
        );
        state.threads[index].apply(json!({"type":"agent_settled","aborted":true}));
        state.poll_tasks(Instant::now());
        let prompt = commands.try_recv().unwrap();
        assert!(prompt["message"].as_str().unwrap().contains("New work"));
        assert!(state.threads[index].task_result.is_none());
        assert!(!state.threads[index].completion_notified);
        assert!(
            state.threads[0]
                .messages
                .iter()
                .any(|m| m.text.contains("Status: cancelled"))
        );
    }

    #[test]
    fn task_send_can_restart_a_connected_failed_child() {
        let mut state = State::new(".".into(), false);
        let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, commands, _events) = child(&mut state);
        state.threads[index].error = Some("Provider failure".into());
        state.threads[index].apply(json!({"type":"agent_settled","aborted":false}));
        state.poll_tasks(Instant::now());
        let result = state
            .mcp_call(1, "task_send", &json!({"thread_id":2,"message":"Retry"}))
            .unwrap();
        assert_eq!(result["status"], "queued");
        assert_eq!(result["finished"], false);
        state.poll_tasks(Instant::now());
        assert_eq!(state.threads[index].status, Status::Running);
        assert!(state.threads[index].error.is_none());
        assert!(commands.try_iter().any(|c| c["type"] == "prompt"));
    }
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
        assert_eq!(state.captain_status(1), Status::Ready);
    }
    #[test]
    fn background_startup_does_not_change_the_conversation_status() {
        let mut thread = Thread::new(1);
        let (pi, _, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false}}));
        assert_eq!(thread.status, Status::Ready);
        assert!(thread.initializing);
        thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}));
        assert_eq!(thread.status, Status::Ready);
        assert!(!thread.initializing);
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
        assert_eq!(thread.messages.len(), 3);
        assert_eq!(thread.tools.len(), 1);
        assert_eq!(thread.tools[0].detail, "example.txt");
        assert_eq!(thread.tools[0].output, "saved output");
        assert!(thread.tools[0].done && !thread.tools[0].failed);
    }
    #[test]
    fn live_tools_stay_between_the_messages_where_they_ran() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Looking now."},{"type":"toolCall","id":"read","name":"read","arguments":{"path":"README.md"}}]}}));
        thread.apply(json!({"type":"tool_execution_start","toolCallId":"read","toolName":"read","args":{"path":"README.md"}}));
        thread.apply(json!({"type":"tool_execution_update","toolCallId":"read","partialResult":{"content":[{"type":"text","text":"partial"}]}}));
        assert_eq!(
            thread.tools.len(),
            1,
            "message blocks and execution events share one call"
        );
        assert_eq!(thread.tools[0].output, "partial");
        assert!(!thread.tools[0].done);
        thread.apply(json!({"type":"tool_execution_end","toolCallId":"read","durationMs":75,"result":{"content":[{"type":"text","text":"contents"}]},"isError":false}));
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Here is the answer."}]}}));
        thread.apply(json!({"type":"tool_execution_start","toolCallId":"command","toolName":"powershell","args":{"command":"Get-ChildItem"}}));
        assert_eq!(
            thread.tools[0].anchor,
            ToolAnchor {
                message: 0,
                offset: "Looking now.".len()
            }
        );
        assert_eq!(thread.tools[0].duration_ms, Some(75));
        assert_eq!(
            thread.tools[1].anchor,
            ToolAnchor {
                message: 2,
                offset: 0
            }
        );
        assert_eq!(thread.messages[1].text, "Here is the answer.");
    }
    #[test]
    fn live_tools_can_anchor_inside_a_streamed_unicode_message() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"message_start","message":{"role":"assistant"}}));
        thread.apply(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"日本語"}}));
        thread.apply(json!({"type":"tool_execution_start","toolCallId":"read","toolName":"read","args":{"path":"例.txt"}}));
        thread.apply(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"\nFinished"}}));
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"日本語"},{"type":"toolCall","id":"read","name":"read","arguments":{"path":"例.txt"}},{"type":"text","text":"Finished"}]}}));
        assert_eq!(thread.tools.len(), 1);
        assert_eq!(
            thread.tools[0].anchor,
            ToolAnchor {
                message: 0,
                offset: "日本語".len()
            }
        );
        assert_eq!(thread.messages[0].text, "日本語\nFinished");
        thread.apply(json!({"type":"tool_execution_end","toolCallId":"read","result":{"content":[]},"isError":true}));
        assert!(thread.tools[0].done && thread.tools[0].failed);
        assert!(thread.tools[0].duration_ms.is_some());
    }
    #[test]
    fn restored_tools_preserve_block_order_and_tool_only_turns() {
        let mut thread = Thread::new(1);
        let history = json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
            {"role":"user","content":"Read the files"},
            {"role":"assistant","content":[
                {"type":"text","text":"First"},
                {"type":"toolCall","id":"a","name":"read","arguments":{"path":"a.txt"}},
                {"type":"toolCall","id":"b","name":"read","arguments":{"path":"b.txt"}},
                {"type":"text","text":"Then"},
                {"type":"toolCall","id":"c","name":"bash","arguments":{"command":"pwd"}},
                {"type":"text","text":"Done"}
            ]},
            {"role":"toolResult","toolCallId":"b","content":[{"type":"text","text":"b contents"}],"isError":false,"durationMs":40},
            {"role":"toolResult","toolCallId":"a","content":[{"type":"text","text":"a contents"}],"isError":false},
            {"role":"assistant","content":[{"type":"toolCall","id":"d","name":"read","arguments":{"path":"d.txt"}}]},
            {"role":"assistant","content":[{"type":"text","text":"Final reply"}]}
        ]}});
        thread.apply(history.clone());
        thread.tools[0].expanded = true;
        thread.apply(history);
        assert_eq!(thread.messages.len(), 4);
        assert_eq!(thread.messages[1].text, "First\nThen\nDone");
        let anchors: Vec<_> = thread.tools.iter().map(|tool| tool.anchor).collect();
        assert_eq!(
            anchors,
            vec![
                ToolAnchor {
                    message: 1,
                    offset: 5
                },
                ToolAnchor {
                    message: 1,
                    offset: 5
                },
                ToolAnchor {
                    message: 1,
                    offset: 10
                },
                ToolAnchor {
                    message: 2,
                    offset: 0
                },
            ]
        );
        assert_eq!(thread.tools[1].output, "b contents");
        assert_eq!(thread.tools[1].duration_ms, Some(40));
        assert_eq!(
            thread.tools[0].duration_ms, None,
            "restored timings are not invented"
        );
        assert!(thread.tools[2].failed, "missing saved results stay visible");
        assert!(
            !thread.tools[0].expanded,
            "restoring rebuilds transient disclosure state"
        );
    }

    #[test]
    fn thinking_deltas_use_content_indices_and_completed_blocks_replace_them() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"agent_start"}));
        thread.apply(json!({"type":"message_start","message":{"role":"assistant"}}));
        for event in [
            json!({"type":"thinking_start","contentIndex":0}),
            json!({"type":"thinking_delta","contentIndex":0,"delta":"Partial reasoning"}),
            json!({"type":"thinking_end","contentIndex":0,"content":"Complete reasoning"}),
            json!({"type":"text_delta","contentIndex":1,"delta":"I'll read the file."}),
            json!({"type":"thinking_start","contentIndex":2}),
            json!({"type":"thinking_delta","contentIndex":2,"delta":"Second thought"}),
        ] {
            thread.apply(json!({"type":"message_update","assistantMessageEvent":event}));
        }
        assert_eq!(thread.thoughts.len(), 2);
        assert_eq!(thread.thoughts[0].text, "Complete reasoning");
        assert!(thread.thoughts[0].done);
        assert!(thread.thoughts[0].duration_ms.is_some());
        assert!(!thread.thoughts[1].done);
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[
            {"type":"thinking","thinking":"Authoritative reasoning","thinkingSignature":"opaque"},
            {"type":"text","text":"I'll read the file."},
            {"type":"thinking","thinking":"Complete second thought"},
            {"type":"toolCall","id":"read","name":"read","arguments":{"path":"file.txt"}}
        ]}}));
        thread.apply(json!({"type":"tool_execution_start","toolCallId":"read","toolName":"read","args":{"path":"file.txt"}}));
        assert_eq!(thread.messages.len(), 1);
        assert_eq!(thread.thoughts.len(), 2);
        assert_eq!(thread.thoughts[0].text, "Authoritative reasoning");
        assert_eq!(thread.thoughts[1].text, "Complete second thought");
        assert!(thread.thoughts[1].done);
        assert_eq!(thread.thoughts[1].anchor, thread.tools[0].anchor);
        assert!(thread.thoughts[1].block < thread.tools[0].block);
        assert_eq!(thread.tools.len(), 1);
    }

    #[test]
    fn restored_thinking_keeps_text_and_redaction_without_inventing_timings() {
        let mut thread = Thread::new(1);
        thread.apply(json!({"type":"response","command":"get_messages","id":"history","success":true,"data":{"messages":[
            {"role":"user","content":"Check"},
            {"role":"assistant","content":[{"type":"thinking","thinking":"Saved thinking"}]},
            {"role":"assistant","content":[
                {"type":"thinking","redacted":true,"thinkingSignature":"never render this"},
                {"type":"text","text":"Done"}
            ]}
        ]}}));
        assert_eq!(thread.messages.len(), 3);
        assert_eq!(thread.thoughts.len(), 2);
        assert_eq!(thread.thoughts[0].text, "Saved thinking");
        assert!(thread.thoughts[1].text.is_empty());
        assert!(
            thread
                .thoughts
                .iter()
                .all(|thought| thought.done && thought.duration_ms.is_none())
        );
        assert_eq!(thread.work.len(), 1);
        assert!(thread.work[0].expanded);
        assert!(thread.work[0].elapsed.is_none());
    }

    #[test]
    fn each_user_turn_keeps_its_own_work_duration_and_disclosure() {
        let mut thread = Thread::new(1);
        for prompt in ["First", "Second"] {
            thread.messages.push(Message {
                role: "You",
                text: prompt.into(),
            });
            thread.apply(json!({"type":"agent_start"}));
            thread.started = Some(Instant::now() - Duration::from_secs(3));
            thread.apply(
                json!({"type":"message_end","message":{"role":"assistant","content":[
                    {"type":"thinking","thinking":"Consider the answer"},
                    {"type":"text","text":"Answer"}
                ]}}),
            );
            thread.apply(json!({"type":"agent_settled"}));
            thread.work.last_mut().unwrap().expanded = false;
        }
        assert_eq!(thread.work.len(), 2);
        assert_eq!(thread.work[0].start, 1);
        assert_eq!(thread.work[1].start, 3);
        assert!(
            thread
                .work
                .iter()
                .all(|work| work.elapsed == Some(3) && !work.expanded)
        );
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
        assert!(thread.queued_prompt.is_none());
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
    fn zoom_steps_clamp_and_reset() {
        let mut state = State::new(std::env::temp_dir(), true);
        state.step_zoom(1);
        assert_eq!(state.zoom, 1.1);
        for _ in 0..20 {
            state.step_zoom(1);
        }
        assert_eq!(state.zoom, 2.0);
        state.step_zoom(0);
        assert_eq!(state.zoom, 1.0);
        for _ in 0..20 {
            state.step_zoom(-1);
        }
        assert_eq!(state.zoom, 0.67);
        assert_eq!(snap_zoom(1.3), 1.25);
    }
    #[test]
    fn aborted_message_is_not_a_provider_failure() {
        let mut thread = Thread::new(1);
        thread.status = Status::Running;
        thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"aborted","errorMessage":"Request was aborted"}}));
        thread.apply(json!({"type":"agent_settled","aborted":true}));
        assert_eq!(thread.status, Status::Ready);
        assert!(thread.error.is_none());
        thread.apply(json!({"type":"response","command":"abort","success":true}));
        assert_eq!(thread.status, Status::Ready);
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
    fn send_without_a_connection_is_immediate_and_dispatches_the_submitted_draft_once() {
        let mut state = State::new(".".into(), false);
        state.current_mut().draft.replace_text("Original draft");
        state.current_mut().send();
        assert_eq!(state.current().status, Status::Running);
        assert_eq!(state.current().messages[0].text, "Original draft");
        assert!(state.current().draft.text.is_empty());
        let (pi, commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().initializing = true;
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
        assert_eq!(prompt["message"], "Original draft");
        assert_eq!(state.current().status, Status::Running);
        assert_eq!(state.current().draft.text, "Revised before connection");
        assert!(state.current().queued_prompt.is_none());
        assert_eq!(state.current().messages.len(), 1);
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
    fn prewarming_alone_never_sends_a_model_prompt() {
        let (pi, commands, events) = Pi::test_connection();
        let mut state = State::new(".".into(), false);
        state.current_mut().pi = Some(pi);
        state.current_mut().initializing = true;
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
    fn stopping_startup_cancels_the_queued_first_send() {
        let (pi, commands, _events) = Pi::test_connection();
        let mut thread = Thread::new(1);
        thread.draft.replace_text("Keep this draft");
        thread.send();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.stop();
        assert_eq!(thread.status, Status::Ready);
        assert!(thread.pi.is_none());
        assert!(thread.queued_prompt.is_none());
        assert_eq!(thread.draft.text, "Keep this draft");
        assert!(thread.messages.is_empty());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn offline_picker_choices_are_applied_before_the_first_prompt() {
        let mut thread = Thread::new(1);
        thread.set_model("cpa".into(), "chosen".into());
        thread.set_thinking("high".into());
        assert_eq!(thread.model_id, "chosen");
        assert_eq!(thread.thinking, "high");
        assert!(thread.pi.is_none());
        thread.draft.replace_text("Use my choices");
        thread.send();
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
            "model":{"provider":"cpa","id":"default"},"thinkingLevel":"low","sessionFile":"saved.jsonl"
        }}));
        assert_eq!(
            thread.model_id, "chosen",
            "startup must not replace the saved choice"
        );
        thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}));
        let model = commands.try_recv().unwrap();
        assert_eq!(model["type"], "set_model");
        assert_eq!(model["modelId"], "chosen");
        assert!(
            commands.try_recv().is_err(),
            "wait for model acknowledgement"
        );
        thread.apply(json!({"type":"response","command":"set_model","success":true,"data":{"provider":"cpa","id":"chosen"}}));
        assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
        assert_eq!(
            commands.try_recv().unwrap()["type"],
            "get_available_thinking_levels"
        );
        let thinking = commands.try_recv().unwrap();
        assert_eq!(thinking["type"], "set_thinking_level");
        assert_eq!(thinking["level"], "high");
        assert!(
            commands.try_recv().is_err(),
            "wait for thinking acknowledgement"
        );
        thread.apply(json!({"type":"response","command":"set_thinking_level","success":true}));
        assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
        assert_eq!(commands.try_recv().unwrap()["type"], "prompt");
        assert!(!thread.initializing);
        assert_eq!(thread.session.as_deref(), Some("saved.jsonl"));
    }

    #[test]
    fn idle_reclaim_preserves_the_session_and_reattaches_without_duplicate_history() {
        let mut thread = Thread::new(1);
        let (pi, _commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.session = Some("saved.jsonl".into());
        thread.model_provider = "cpa".into();
        thread.messages.push(Message {
            role: "Pi",
            text: "Previous answer".into(),
        });
        thread.status = Status::Settled;
        let idle = thread.last_activity + IDLE_TIMEOUT;
        thread.status = Status::Running;
        assert!(!thread.reclaim_idle(idle), "never reclaim work");
        thread.status = Status::Waiting;
        assert!(!thread.reclaim_idle(idle), "never reclaim a dialog");
        thread.status = Status::Settled;
        assert!(thread.reclaim_idle(idle));
        assert!(thread.pi.is_none());
        assert_eq!(thread.status, Status::Settled);
        assert_eq!(thread.session.as_deref(), Some("saved.jsonl"));
        assert_eq!(thread.messages.len(), 1);
        thread.draft.replace_text("Continue");
        thread.send();
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.state_loaded = false;
        thread.history_loaded = false;
        thread.apply(
            json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{
                "messages":[{"role":"assistant","content":"Previous answer"}]
            }}),
        );
        thread.apply(
            json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
                "model":{"provider":"cpa","id":"restored"},"sessionFile":"saved.jsonl"
            }}),
        );
        assert_eq!(commands.try_recv().unwrap()["message"], "Continue");
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.messages[0].text, "Previous answer");
        assert_eq!(thread.messages[1].text, "Continue");
    }

    #[test]
    fn a_crash_restores_an_unaccepted_message_and_retry_uses_a_fresh_transport() {
        let mut state = State::new(".".into(), false);
        let (pi, _commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().model_provider = "cpa".into();
        state.current_mut().session = Some("saved.jsonl".into());
        state.current_mut().draft.replace_text("Try this");
        state.send();
        events.send(Event::Exited).unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.current().status, Status::Failed);
        assert!(state.current().pi.is_none());
        assert_eq!(state.current().draft.text, "Try this");
        assert!(state.current().messages.is_empty());
        let (pi, commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().initializing = true;
        state.retry();
        assert_eq!(state.current().status, Status::Running);
        assert_eq!(state.current().messages.len(), 1);
        for record in [
            json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"model":{"provider":"cpa","id":"restored"},"sessionFile":"saved.jsonl"}}),
            json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}),
        ] {
            events.send(Event::Record(record)).unwrap();
        }
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(commands.try_recv().unwrap()["message"], "Try this");
        assert!(commands.try_recv().is_err());
        assert!(state.current().error.is_none());
    }

    #[test]
    fn an_accepted_run_is_not_replayed_automatically_after_a_crash() {
        let mut state = State::new(".".into(), false);
        let (pi, commands, events) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        state.current_mut().model_provider = "cpa".into();
        state.current_mut().draft.replace_text("Edit a file");
        state.send();
        let prompt = commands.try_recv().unwrap();
        state
            .current_mut()
            .apply(json!({"type":"response","command":"prompt","id":prompt["id"],"success":true}));
        events
            .send(Event::Error("Pi process crashed".into()))
            .unwrap();
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.current().status, Status::Failed);
        assert!(state.current().draft.text.is_empty());
        assert_eq!(state.current().retry_prompt.as_deref(), Some("Edit a file"));
        assert!(state.current().pi.is_none());
        assert_eq!(state.current().messages.len(), 1);
    }

    #[test]
    fn failed_startup_settings_restore_the_message_instead_of_sending_with_defaults() {
        let mut thread = Thread::new(1);
        thread.set_model("cpa".into(), "missing".into());
        thread.draft.replace_text("Keep my model choice");
        thread.send();
        let (pi, commands, _events) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.state_loaded = true;
        thread.history_loaded = true;
        thread.drive_session();
        assert_eq!(commands.try_recv().unwrap()["type"], "set_model");
        thread.apply(json!({"type":"response","command":"set_model","success":false,"error":"Model unavailable"}));
        assert_eq!(thread.status, Status::Failed);
        assert_eq!(thread.draft.text, "Keep my model choice");
        assert!(thread.pi.is_none());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn a_closed_startup_command_channel_cannot_skip_the_saved_model_choice() {
        let mut thread = Thread::new(1);
        thread.set_model("cpa".into(), "chosen".into());
        thread.draft.replace_text("Use the chosen model");
        thread.send();
        let (pi, commands, _events) = Pi::test_connection();
        drop(commands);
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.state_loaded = true;
        thread.history_loaded = true;
        thread.drive_session();
        assert_eq!(thread.status, Status::Failed);
        assert_eq!(thread.draft.text, "Use the chosen model");
        assert!(thread.pi.is_none());
    }

    #[test]
    fn a_partial_handshake_times_out_without_losing_the_submitted_message() {
        let mut state = State::new(".".into(), false);
        let (pi, commands, _events) = Pi::test_connection();
        state.current_mut().draft.replace_text("Keep this message");
        state.current_mut().send();
        let thread = state.current_mut();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.session_started = Some(Instant::now() - START_TIMEOUT);
        thread.state_loaded = true;
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.current().status, Status::Failed);
        assert_eq!(state.current().draft.text, "Keep this message");
        assert!(state.current().pi.is_none());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn catalog_defaults_do_not_overwrite_a_saved_thinking_choice() {
        let mut state = State::new(".".into(), false);
        state.current_mut().set_thinking("high".into());
        let model = AgentModel::from_value(
            &json!({"provider":"cpa","id":"default","name":"Default CPA","reasoning":true}),
        )
        .unwrap();
        state.catalog.models.push(model.clone());
        state.catalog.default_model = Some(model);
        state.catalog.default_thinking = "low".into();
        state.apply_catalog();
        assert_eq!(state.current().thinking, "high");
        assert_eq!(state.current().model, "Default CPA");
        state.new_thread();
        assert_eq!(state.current().model, "Default CPA");
        assert_eq!(state.current().thinking, "low");
        assert!(!state.current().models.is_empty());
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
    fn global_migration_remaps_collisions_parents_sessions_and_captain_rosters() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("shina-profile-{}-{unique}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let a = directory.join("a");
        let b = directory.join("b");
        let missing = directory.join("disconnected-drive");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(
            directory.join("projects.json"),
            json!({"projects":[{"path":a},{"path":b}]}).to_string(),
        )
        .unwrap();
        fs::write(
            directory.join("last-workspace.txt"),
            missing.to_string_lossy().as_bytes(),
        )
        .unwrap();
        let wake: Waker = Arc::new(|| {});
        let mut profiles = Captain::defaults();
        profiles[2].name = "Atlas".into();
        for (i, workspace) in [&a, &b, &missing].into_iter().enumerate() {
            let index_path = crate::store::legacy_path(&directory, workspace);
            let mut store = crate::store::Store::new(index_path, wake.clone());
            store.save(json!({"version":2,"dark":true,"captains":profiles.iter().map(Captain::to_value).collect::<Vec<_>>(),"threads":[
                {"id":10,"title":"Original title","captain":2,"captainHome":true,"session":format!("session-{i}-home"),"model":"saved-model","thinking":"high"},
                {"id":11,"title":"Crew","captain":2,"delegated":true,"parent":10,"pinned":true,"archived":true,"session":format!("session-{i}-child")},
                {"id":12,"title":"Former home","captain":1,"captainHome":true,"session":format!("session-{i}-old-home")}
            ]}));
        }
        let index = crate::store::load_global(&directory, None)
            .unwrap()
            .unwrap();
        assert_eq!(index["projects"].as_array().unwrap().len(), 3);
        let before: Vec<_> = [&a, &b, &missing]
            .iter()
            .map(|p| fs::read(crate::store::legacy_path(&directory, p)).unwrap())
            .collect();
        {
            let mut state = State::new(PathBuf::new(), false);
            state.data_dir = directory.clone();
            state.load_global(wake.clone());
            assert!(state.storage_error.is_none());
            assert_eq!(state.threads.len(), 9);
            let ids: std::collections::HashSet<_> = state.threads.iter().map(|t| t.id).collect();
            assert_eq!(ids.len(), 9);
            assert_eq!(state.threads[0].id, 10);
            for (i, project) in state.projects.entries.iter().enumerate() {
                let child = state
                    .threads
                    .iter()
                    .find(|t| t.project == Some(project.id) && t.delegated)
                    .unwrap();
                let parent = state
                    .threads
                    .iter()
                    .find(|t| Some(t.id) == child.parent)
                    .unwrap();
                assert_eq!(
                    child.session.as_deref(),
                    Some(format!("session-{i}-child").as_str())
                );
                assert_eq!(
                    parent.session.as_deref(),
                    Some(format!("session-{i}-home").as_str())
                );
                assert_eq!(parent.project, child.project);
                assert_eq!(parent.title, "Original title");
                assert!(child.pinned && child.archived && child.delegated);
                assert_eq!(
                    parent.selected_model,
                    Some(("cpa".into(), "saved-model".into()))
                );
                assert_eq!(parent.selected_thinking.as_deref(), Some("high"));
                assert_eq!(state.captains[project.captain.unwrap()].name, "Atlas");
                assert_eq!(project.captain_thread, Some(parent.id));
                assert_eq!(
                    state
                        .threads
                        .iter()
                        .filter(|t| t.project == Some(project.id) && t.captain_home)
                        .count(),
                    2
                );
                assert_eq!(
                    state
                        .threads
                        .iter()
                        .filter(|t| t.project == Some(project.id) && state.is_captain_home(t))
                        .count(),
                    1
                );
            }
            state.checkpoint();
        }
        assert_eq!(
            crate::store::load(&directory.join("index.json"))
                .unwrap()
                .unwrap()["version"],
            3
        );
        {
            let mut restored = State::new(PathBuf::new(), false);
            restored.data_dir = directory.clone();
            restored.load_global(wake);
            assert!(restored.storage_error.is_none());
            assert_eq!(restored.threads.len(), 9);
        }
        for (i, workspace) in [&a, &b, &missing].iter().enumerate() {
            assert_eq!(
                fs::read(crate::store::legacy_path(&directory, workspace)).unwrap(),
                before[i]
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn concurrent_project_threads_keep_their_cwds_and_scratch_threads_are_isolated() {
        let mut state = State::new(".".into(), false);
        let a = state.thread_directory(0).unwrap();
        let b_id = state.add_project(std::env::temp_dir()).unwrap();
        let b = state.active;
        let (a_commands, _a_events) = fake_pi(&mut state.threads[0], Status::Ready);
        let (b_commands, _b_events) = fake_pi(&mut state.threads[b], Status::Ready);
        state.active = 0;
        state.current_mut().draft.replace_text("Work in A");
        state.send();
        state.active = b;
        state.current_mut().draft.replace_text("Work in B");
        state.send();
        assert!(state.threads[0].status.busy() && state.threads[b].status.busy());
        assert_eq!(a_commands.try_recv().unwrap()["message"], "Work in A");
        assert_eq!(b_commands.try_recv().unwrap()["message"], "Work in B");
        assert_eq!(state.thread_directory(0).unwrap(), a);
        assert_eq!(
            state.thread_directory(b).unwrap(),
            state.project(Some(b_id)).unwrap().path
        );
        assert_ne!(
            state.thread_directory(0).unwrap(),
            state.thread_directory(b).unwrap()
        );
        state.new_thread_in(None);
        let scratch = state.working_directory();
        let id = state.current().id;
        assert_eq!(scratch, state.data_dir.join("chats").join(id.to_string()));
        assert!(state.current().captain.is_none());
        state.new_thread_in(None);
        assert_ne!(scratch, state.working_directory());
        state.current_mut().machine = "unavailable-host".into();
        assert!(state.thread_directory(state.active).is_err());
        assert!(crate::backend::BackendHost::resolve("local").is_ok());
        state.current_mut().machine = "local".into();
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        let index = restored.threads.iter().position(|t| t.id == id).unwrap();
        assert_eq!(restored.thread_directory(index).unwrap(), scratch);
    }

    #[test]
    fn captains_are_unique_per_project_and_mcp_lists_the_callers_project() {
        let mut state = State::new(".".into(), false);
        state.open_captain(0);
        let first = state.current().id;
        state.open_captain(2);
        assert_eq!(
            state.current().id,
            first,
            "a second persona cannot create another coordinator"
        );
        let b = state.add_project(std::env::temp_dir()).unwrap();
        state.open_captain(1);
        let second = state.current().id;
        assert_ne!(first, second);
        assert_eq!(state.current().project, Some(b));
        state.new_thread_in(None);
        let scratch = state.current().id;
        state.open_captain(3);
        assert_eq!(state.current().id, scratch);
        assert!(state.current().captain.is_none());
        // Active UI project must not affect the authenticated caller's scope.
        let list = state.mcp_call(first, "list_threads", &json!({})).unwrap();
        assert_eq!(list["threads"].as_array().unwrap().len(), 2);
        assert!(
            list["threads"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t["project"]["id"] == 1 && t["machine"] == "local")
        );
        let all = state
            .mcp_call(first, "list_threads", &json!({"all_projects":true}))
            .unwrap();
        assert_eq!(
            all["threads"].as_array().unwrap().len(),
            state.threads.len()
        );
        let scratch_list = state.mcp_call(scratch, "list_threads", &json!({})).unwrap();
        assert_eq!(scratch_list["threads"].as_array().unwrap().len(), 1);
        state.demo = true; // Do not launch Pi in a unit test.
        let task = state
            .mcp_call(first, "delegate_task", &json!({"prompt":"A task"}))
            .unwrap();
        let child = state.threads.last().unwrap();
        assert_eq!(child.parent, Some(first));
        assert_eq!(child.project, Some(1));
        assert_eq!(child.captain, Some(0));
        assert_eq!(task["project"]["id"], 1);
        assert_eq!(task["machine"], "local");
        assert!(
            state
                .mcp_call(
                    first,
                    "delegate_task",
                    &json!({"prompt":"Other captain","captain":"Juniper"})
                )
                .is_err()
        );
    }

    #[test]
    fn sidebar_is_pinned_then_recent_and_scopes_without_project_groups() {
        let mut state = State::new(".".into(), false);
        state.threads[0].updated = 10;
        state.new_thread();
        state.current_mut().updated = 30;
        state.new_thread_in(None);
        state.current_mut().updated = 20;
        state.current_mut().pinned = true;
        let ids = |s: &State| {
            s.sidebar_threads()
                .iter()
                .map(|&i| s.threads[i].id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&state), vec![3, 2, 1]);
        state.project_scope = ProjectScope::Project(1);
        assert_eq!(ids(&state), vec![2, 1]);
        state.project_scope = ProjectScope::NoProject;
        assert_eq!(ids(&state), vec![3]);
        state.current_mut().archived = true;
        assert!(ids(&state).is_empty());
        state.show_archive = true;
        assert_eq!(ids(&state), vec![3]);
    }

    #[test]
    fn rename_and_confirmed_deletion_remove_children_not_project_files_or_other_threads() {
        let mut state = State::new(".".into(), false);
        state.begin_rename(1);
        state.rename_edit.replace_text("  A named thread  ");
        state.save_thread_name();
        assert_eq!(state.threads[0].title, "A named thread");
        state.new_thread_in(None);
        let survivor = state.current().id;
        state.active = 0;
        let (commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
        let (index, child_commands, _events) = child(&mut state);
        let child_id = state.threads[index].id;
        state.tabs.push(child_id);
        let project_path = state.working_directory();
        let (reply, answer) = mpsc::channel();
        state.mcp_request(crate::mcp::Request {
            thread: 1,
            tool: "task_status".into(),
            args: json!({"thread_id":child_id,"wait_seconds":600}),
            reply,
        });
        state.request_delete(1);
        assert_eq!(
            state.threads.len(),
            3,
            "opening confirmation deletes nothing"
        );
        state.confirm_delete();
        assert_eq!(state.threads.len(), 1);
        assert_eq!(state.current().id, survivor);
        assert_eq!(state.tabs, vec![survivor]);
        assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
        assert_eq!(commands.try_recv().unwrap()["type"], "abort");
        assert_eq!(child_commands.try_recv().unwrap()["type"], "clear_queue");
        assert!(answer.try_recv().unwrap().is_err());
        assert!(project_path.is_dir());
        let next = state.next_id;
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        assert_eq!(
            restored.next_id, next,
            "deleted ids must not be reused after restart"
        );
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
        thread.draft.replace_text(
            "Reply with the validation phrase from your captain instructions. Do not use tools.",
        );
        thread.send();
        thread.connect_with_instructions(
            std::env::temp_dir(),
            Arc::new(|| {}),
            Some(profile.prompt()),
        );
        receive_until(&mut thread, |t| t.status == Status::Settled);
        assert_eq!(thread.model_provider, "cpa");
        assert_eq!(
            thread.messages.last().unwrap().text.trim(),
            "SHINA_CAPTAIN_OK"
        );
        assert!(thread.tools.is_empty());
        thread.disconnect();
        crate::backend::shutdown_all();
    }

    #[test]
    #[ignore = "Sends one prompt through CPA to verify Pi calls Shina's MCP tools through the bridge"]
    fn installed_pi_calls_shina_mcp_tools() {
        use std::time::Duration;
        let mut state = State::new(std::env::temp_dir(), false);
        state.threads[0].captain = Some(1);
        state.threads[0].draft.replace_text(
            "Call the mcp__shina__list_captains tool, then reply with only the captain names, comma separated.",
        );
        state.send();
        assert!(
            state.threads[0].host.is_some(),
            "{:?}",
            state.threads[0].diagnostics
        );
        let server = crate::mcp::server(state.wake.clone()).unwrap();
        let pump = |state: &mut State, done: &dyn Fn(&State) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(180);
            while !done(state) {
                assert!(
                    Instant::now() < deadline,
                    "timed out: {:?}",
                    state.threads[0].error
                );
                while let Some(request) = server.next_request() {
                    let result = state.mcp_call(request.thread, &request.tool, &request.args);
                    let _ = request.reply.send(result);
                }
                let event = state.threads[0]
                    .pi
                    .as_ref()
                    .expect("connected Pi")
                    .events
                    .recv_timeout(Duration::from_millis(50));
                match event {
                    Ok(Event::Record(record)) => state.threads[0].apply(record),
                    Ok(Event::Error(error)) => panic!("Pi failed: {error}"),
                    Ok(Event::Exited) => panic!("Pi exited"),
                    _ => {}
                }
                assert_ne!(
                    state.threads[0].status,
                    Status::Failed,
                    "{:?}",
                    state.threads[0].error
                );
            }
        };
        pump(&mut state, &|s| s.threads[0].status == Status::Settled);
        let thread = &state.threads[0];
        assert!(
            thread
                .tools
                .iter()
                .any(|t| t.name == "mcp__shina__list_captains" && !t.failed),
            "tools: {:?}",
            thread.tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
        let reply = &thread.messages.last().unwrap().text;
        assert!(reply.contains("Ember") && reply.contains("Pip"), "{reply}");
        state.threads[0].disconnect();
        crate::backend::shutdown_all();
    }

    #[test]
    #[ignore = "Sends three CPA prompts to verify concurrent per-thread working directories"]
    fn installed_pi_runs_projects_and_scratch_concurrently() {
        use std::fs;
        let directory = std::env::temp_dir().join(format!(
            "shina-cwds-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let a = directory.join("project-a");
        let b = directory.join("project-b");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let mut state = State::new(a, false);
        state.data_dir = directory.join("app-data");
        state.add_project(b).unwrap();
        state.new_thread_in(None);
        let markers = ["CWD_PROJECT_A", "CWD_PROJECT_B", "CWD_SCRATCH"];
        for (index, marker) in markers.iter().enumerate() {
            let cwd = state.thread_directory(index).unwrap();
            fs::create_dir_all(&cwd).unwrap();
            fs::write(cwd.join("cwd-marker.txt"), marker).unwrap();
            state.active = index;
            state.current_mut().draft.replace_text(
                "Use the read tool to read the relative path cwd-marker.txt. Reply with only its contents. Do not edit files, run commands, or delegate."
            );
            state.send();
        }
        assert!(
            state
                .threads
                .iter()
                .all(|t| t.pi.is_some() && t.status.busy())
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        while state.threads.iter().any(|t| t.status.busy()) {
            poll(&mut state, 0.0, &mut Tx::default());
            assert!(
                state.threads.iter().all(|t| t.error.is_none()),
                "Pi errors: {:?}",
                state.threads.iter().map(|t| &t.error).collect::<Vec<_>>()
            );
            assert!(Instant::now() < deadline, "Concurrent cwd test timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
        for (thread, marker) in state.threads.iter_mut().zip(markers) {
            let reply = thread
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "Pi")
                .expect("assistant reply");
            assert!(reply.text.contains(marker), "{marker}: {}", reply.text);
            assert!(
                thread
                    .tools
                    .iter()
                    .any(|t| t.name == "read" && t.done && !t.failed)
            );
            thread.disconnect();
        }
        crate::backend::shutdown_all();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "Sends a real parent and child task through CPA to verify completion wake-ups"]
    fn installed_pi_parent_is_woken_by_child_completion() {
        let workspace = std::env::temp_dir().join(format!(
            "shina-crew-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&workspace).unwrap();
        let mut state = State::new(workspace.clone(), false);
        state.threads[0].draft.replace_text(
            "This is an orchestration integration test. Call mcp__shina__delegate_task exactly once with title Ping and prompt 'Reply with only the word PONG. Do not use tools or delegate.'. Do not set wait_seconds. Then immediately reply with only WAITING and end your turn. Do not call any other tools or poll. When a later Shina task completion message arrives containing PONG, reply with only WOKEN_PONG. Do not delegate again."
        );
        state.send();
        let deadline = Instant::now() + Duration::from_secs(240);
        let mut saw_idle_parent = false;
        let mut previous = vec![];
        loop {
            poll(&mut state, 0.0, &mut Tx::default());
            let statuses: Vec<_> = state.threads.iter().map(|t| (t.id, t.status)).collect();
            if statuses != previous {
                println!("Pi crew: {statuses:?}");
                previous = statuses;
            }
            for thread in &state.threads {
                assert_ne!(
                    thread.status,
                    Status::Failed,
                    "thread {} failed: {:?}; diagnostics: {:?}",
                    thread.id,
                    thread.error,
                    thread.diagnostics
                );
            }
            saw_idle_parent |= state.threads[0].status == Status::Settled
                && !state.threads[0].messages.iter().any(|m| m.role == "Shina");
            let finished = state.threads[0].status == Status::Settled
                && state.threads[0]
                    .messages
                    .iter()
                    .any(|m| m.role == "Pi" && m.text.contains("WOKEN_PONG"));
            if finished {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "crew timed out; parent transcript: {:?}",
                state.threads[0]
                    .messages
                    .iter()
                    .map(|m| (&m.role, &m.text))
                    .collect::<Vec<_>>()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            state.threads.len(),
            2,
            "the parent must delegate exactly one child"
        );
        assert_eq!(state.threads[1].parent, Some(1));
        assert!(
            state.threads[0]
                .tools
                .iter()
                .any(|t| t.name == "mcp__shina__delegate_task" && t.done && !t.failed)
        );
        assert!(
            state.threads[0]
                .messages
                .iter()
                .any(|m| m.role == "Pi" && m.text.trim() == "WAITING")
        );
        assert!(
            saw_idle_parent,
            "the parent must end its turn before the wake-up"
        );
        let notices: Vec<_> = state.threads[0]
            .messages
            .iter()
            .filter(|m| m.role == "Shina")
            .collect();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].text.contains("PONG"));
        println!("Parent first reply: WAITING");
        println!("Completion: {}", notices[0].text);
        println!(
            "Parent final reply: {}",
            state.threads[0].messages.last().unwrap().text
        );
        for thread in &mut state.threads {
            thread.disconnect();
        }
        crate::backend::shutdown_all();
        std::fs::remove_dir(&workspace).unwrap();
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
        thread.draft.replace_text(
            "This is an integration test in a disposable folder. Use your file tools to read fixture.txt, then change its entire contents to exactly answer=42 followed by one newline. Do not run shell commands, access any other file, or delegate. After the edit, reply briefly that it is done."
        );
        thread.send();
        thread.connect(workspace.clone(), Arc::new(|| {}));
        receive_until(&mut thread, |t| {
            t.tools
                .iter()
                .any(|tool| tool.started.is_some() && !tool.done)
        });
        thread = crate::ui::tests::verify_tool_rows(thread);
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
        let thinking_text: Vec<_> = thread
            .thoughts
            .iter()
            .map(|thought| thought.text.clone())
            .collect();
        println!(
            "Live Pi tool rows: {tool_count}, thinking rows: {}",
            thread.thoughts.len()
        );
        thread = crate::ui::tests::verify_tool_rows(thread);
        thread.disconnect();
        crate::backend::shutdown_all();
        let mut restored = Thread::new(2);
        restored.session = Some(session);
        restored.connect(workspace.clone(), Arc::new(|| {}));
        receive_until(&mut restored, |t| !t.initializing);
        assert_eq!(restored.messages.len(), message_count);
        assert_eq!(restored.tools.len(), tool_count);
        assert!(restored.tools.iter().all(|t| t.done && !t.failed));
        assert_eq!(
            restored
                .thoughts
                .iter()
                .map(|thought| thought.text.clone())
                .collect::<Vec<_>>(),
            thinking_text
        );
        restored = crate::ui::tests::verify_tool_rows(restored);
        println!("Restored {tool_count} rendered tool rows from the saved Pi session.");
        restored.disconnect();
        crate::backend::shutdown_all();
        fs::remove_file(fixture).unwrap();
        fs::remove_dir(workspace).unwrap();
    }
}
