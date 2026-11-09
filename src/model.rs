use crate::backend::{Event, HostLink, Pi};
use crate::captains::{Captain, CaptainEditor};
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
            title: "New thread".into(),
            status: Status::Ready,
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
        self.pi = Some(Pi::start_with_instructions(
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
                            self.assistant = None;
                            for message in messages {
                                let role = string(message, "role");
                                if matches!(role, "user" | "assistant") {
                                    let text = content_text(&message["content"]);
                                    if !text.is_empty() {
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
        self.assistant = None;
        self.phase.clear();
        if self.error.is_none() {
            self.retry_prompt = None;
        }
    }
}

pub struct State {
    pub workspace: PathBuf,
    pub threads: Vec<Thread>,
    pub active: usize,
    pub next_id: usize,
    pub dark: bool,
    /// Whole-window zoom, changed with Ctrl+= / Ctrl+- and reset with Ctrl+0.
    pub zoom: f32,
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
        let artwork = crate::visuals::Artwork::load(&workspace);
        let mut state = Self {
            workspace,
            threads: vec![Thread::new(1)],
            active: 0,
            next_id: 2,
            dark: false,
            zoom: 1.0,
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
            updater: Default::default(),
            restart_for_update: false,
            task_waiters: vec![],
            projects: Default::default(),
            catalog: Catalog::default(),
            runtime_enabled: false,
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
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
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
    pub fn toggle_pin(&mut self) {
        self.current_mut().pinned = !self.current().pinned;
        self.checkpoint();
    }
    /// Connects one thread's Pi with its captain's instructions and its Shina MCP link.
    pub fn connect_thread(&mut self, index: usize) {
        if self.demo || index >= self.threads.len() {
            return;
        }
        let path = self.workspace.clone();
        let wake = self.wake.clone();
        let thread_id = self.threads[index].id;
        let instructions = self.threads[index]
            .captain
            .and_then(|id| self.captains.get(id))
            .map(Captain::prompt);
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
                let threads: Vec<Value> = self
                    .threads
                    .iter()
                    .filter(|t| archived || !t.archived)
                    .map(|t| {
                        json!({
                            "thread_id": t.id,
                            "title": t.title,
                            "captain": captain_name(self, t.captain),
                            "captain_home": t.captain_home,
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
                            "status": status_word(self.captain_status(i)),
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
                let captain = match args["captain"]
                    .as_str()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                {
                    Some(name) => Some(
                        self.captains
                            .iter()
                            .position(|c| c.name.eq_ignore_ascii_case(name))
                            .ok_or_else(|| format!("There is no captain named {name}."))?,
                    ),
                    None => self.threads[caller_index].captain,
                };
                let mut thread = Thread::new(self.next_id);
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
                .refresh(self.workspace.clone(), self.wake.clone());
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
    pub fn switch_workspace(&mut self, path: PathBuf) {
        if let Err(error) = self.try_switch_workspace(path) {
            self.current_mut().error = Some(error);
        }
    }
    /// Rebuilds the state for another folder, or says which thread keeps the current one open.
    pub fn try_switch_workspace(&mut self, path: PathBuf) -> Result<(), String> {
        if let Some(thread) = self.threads.iter().find(|thread| thread.status.busy()) {
            return Err(format!(
                "\"{}\" is {}. Stop it before opening another folder. Your current workspace has not changed.",
                thread.title,
                match thread.status {
                    Status::Waiting => "waiting for your reply",
                    _ => "still working",
                }
            ));
        }
        if let Some(thread) = self.threads.iter().find(|t| !t.draft.text.is_empty()) {
            return Err(format!(
                "\"{}\" has an unsent draft. Send or clear it before opening another folder. Your current workspace has not changed.",
                thread.title
            ));
        }
        let path = match path.canonicalize() {
            Ok(path) if path.is_dir() => path,
            _ => return Err("That workspace folder could not be opened.".into()),
        };
        self.checkpoint();
        let mut next = State::new(path, false);
        next.setup(self.wake.clone());
        *self = next;
        Ok(())
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
        self.projects.recents = crate::projects::record(&self.workspace);
        self.runtime_enabled = true;
        self.refresh_catalog();
        self.updater.check(wake.clone());
        // Thread ids restart per workspace, so tokens from a previous workspace must not carry over.
        if let Ok(server) = crate::mcp::server(wake.clone()) {
            server.reset();
        }
        self.load_workspace(wake);
        self.prewarm_current();
    }

    fn load_workspace(&mut self, wake: Waker) {
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
                            if let Some(model) = record["model"].as_str() {
                                // Only CPA choices survive restart, even in hand-edited indexes.
                                thread.selected_model = Some(("cpa".into(), model.into()));
                                thread.model_provider = "cpa".into();
                                thread.model_id = model.into();
                                thread.model = model.into();
                            }
                            thread.selected_thinking =
                                record["thinking"].as_str().map(str::to_owned);
                            if let Some(level) = &thread.selected_thinking {
                                thread.thinking = level.clone();
                            }
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
                                && restored.iter().any(|t| t.id == parent && !t.delegated)
                        })
                    });
                    if restored.len() != threads.len()
                        || ids.len() != restored.len()
                        || !parents_valid
                    {
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
                if let Some(zoom) = index["zoom"].as_f64() {
                    self.zoom = snap_zoom(zoom as f32);
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
        self.checkpoint();
    }
    pub fn checkpoint(&mut self) {
        if self.demo {
            return;
        }
        let snapshot = json!({
            "version":2, "dark":self.dark, "zoom":self.zoom,
            "captains":self.captains.iter().map(Captain::to_value).collect::<Vec<_>>(),
            "threads":self.threads.iter().map(|thread|json!({
                "id":thread.id, "title":thread.title, "session":thread.session, "archived":thread.archived,
                "captain":thread.captain, "captainHome":thread.captain_home, "pinned":thread.pinned, "delegated":thread.delegated,
                "parent":thread.parent,
                "model":thread.selected_model.as_ref().map(|(_, id)|id), "thinking":thread.selected_thinking
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
        self.projects = crate::projects::Projects::demo();
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
            state.load_workspace(wake.clone());
            assert!(state.storage_error.is_none());
            assert_eq!(state.captains.len(), 4);
            assert_eq!(state.current().title, "Sable");
            assert_eq!(state.current().session.as_deref(), Some("saved-session"));
            state
                .current_mut()
                .set_model("cpa".into(), "saved-model".into());
            state.current_mut().set_thinking("high".into());
            state.begin_new_captain();
            state
                .captain_editor
                .as_mut()
                .unwrap()
                .name
                .replace_text("Atlas");
            state.captain_editor.as_mut().unwrap().portrait = 2;
            state.save_captain();
            let parent = state.current().id;
            state.new_thread();
            state.current_mut().delegated = true;
            state.current_mut().parent = Some(parent);
            state.checkpoint();
        }
        assert_eq!(
            crate::store::load(&index_path).unwrap().unwrap()["version"],
            2
        );
        {
            let mut restored = State::new(workspace, false);
            restored.load_workspace(wake);
            assert!(restored.storage_error.is_none());
            assert_eq!(restored.captains.len(), 5);
            assert_eq!(restored.captains[4].name, "Atlas");
            assert_eq!(restored.captains[4].portrait, 2);
            let child = restored.threads.iter().find(|t| t.delegated).unwrap();
            let parent = restored
                .threads
                .iter()
                .find(|t| t.id == child.parent.unwrap())
                .unwrap();
            assert!(parent.captain_home);
            assert_eq!(parent.captain, Some(4));
            let parent_id = parent.id;
            let listed = restored.mcp_call(10, "list_threads", &json!({})).unwrap();
            assert!(
                listed["threads"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["parent"] == parent_id)
            );
            assert_eq!(restored.current().model_id, "saved-model");
            assert_eq!(
                restored.current().selected_model,
                Some(("cpa".into(), "saved-model".into()))
            );
            assert_eq!(
                restored.current().selected_thinking.as_deref(),
                Some("high")
            );
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
        receive_until(&mut restored, |t| !t.initializing);
        assert_eq!(restored.messages.len(), message_count);
        assert_eq!(restored.tools.len(), tool_count);
        assert!(restored.tools.iter().all(|t| t.done && !t.failed));
        restored.disconnect();
        crate::backend::shutdown_all();
        fs::remove_file(fixture).unwrap();
        fs::remove_dir(workspace).unwrap();
    }
}
