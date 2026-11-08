use crate::backend::{Event, Pi};
use rune::{TextEdit, Tx, Waker};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Instant};

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
            model: "Pi default model".into(),
            thinking: String::new(),
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
        }
    }

    pub fn connect(&mut self, path: PathBuf, wake: Waker) {
        if self.pi.is_some() {
            return;
        }
        self.error = None;
        self.status = Status::Connecting;
        self.state_loaded = false;
        self.history_loaded = false;
        self.pi = Some(Pi::start(path, self.session.clone(), wake));
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
        if self.status.busy() || self.pi.is_none() || self.draft.text.trim().is_empty() {
            return;
        }
        let prompt = self.draft.text.trim().to_owned();
        if !self.command("prompt", json!({"message":prompt})) {
            return;
        }
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
    }

    pub fn stop(&mut self) {
        if self.status == Status::Running || self.status == Status::Waiting {
            self.command("clear_queue", json!({}));
            self.command("abort", json!({}));
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
        self.pi = None;
        self.status = Status::Offline;
        self.assistant = None;
        self.request = None;
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
                    if matches!(command, "prompt" | "get_state" | "get_messages") {
                        self.status = Status::Failed;
                    }
                    return;
                }
                match command {
                    "get_state" => {
                        let data = &record["data"];
                        if let Some(model) = data["model"]["name"]
                            .as_str()
                            .or(data["model"]["id"].as_str())
                        {
                            self.model = model.to_owned();
                        }
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
                    "get_messages" => {
                        if let Some(messages) = record["data"]["messages"].as_array() {
                            self.messages.clear();
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
                            }
                        }
                        self.history_loaded = true;
                        if self.state_loaded && self.status == Status::Connecting {
                            self.status = Status::Ready;
                        }
                    }
                    "prompt" if record["data"]["disposition"] == "handled" => {
                        self.status = Status::Ready
                    }
                    "prompt" => {
                        // Capture the newly created session path before a long first run finishes.
                        self.command("get_state", json!({}));
                    }
                    _ => {}
                }
            }
            "agent_start" => {
                self.status = Status::Running;
                self.assistant = None;
            }
            // agent_end is deliberately NOT settlement. Pi can still retry or compact.
            "agent_settled" => {
                self.request = None;
                self.status = if self.error.is_some() {
                    Status::Failed
                } else {
                    Status::Settled
                };
                self.elapsed = self.started.take().map(|start| start.elapsed().as_secs());
                self.assistant = None;
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
                }
            }
            "message_end" if record["message"]["role"] == "assistant" => {
                let message = &record["message"];
                let text = content_text(&message["content"]);
                if !text.is_empty() {
                    self.assistant().text = text;
                    self.error = None;
                }
                if let Some(error) = message["errorMessage"].as_str().filter(|s| !s.is_empty()) {
                    self.error = Some(error.to_owned());
                }
                self.assistant = None;
            }
            "tool_execution_start" => {
                self.tools.push(Tool {
                    id: string(&record, "toolCallId").into(),
                    name: string(&record, "toolName").into(),
                    detail: tool_detail(&record["args"]),
                    done: false,
                    failed: false,
                    output: String::new(),
                });
            }
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
    pub search: TextEdit,
    pub demo: bool,
    pub wake: Waker,
    pub store: Option<crate::store::Store>,
    pub storage_error: Option<String>,
    pub artwork: Arc<crate::visuals::Artwork>,
    pub captain_picker: bool,
    pub search_open: bool,
}

impl State {
    pub fn new(workspace: PathBuf, demo: bool) -> Self {
        let mut state = Self {
            workspace,
            threads: vec![Thread::new(1)],
            active: 0,
            next_id: 2,
            dark: false,
            show_activity: false,
            show_archive: false,
            sidebar: true,
            search: TextEdit::default(),
            demo,
            wake: Arc::new(|| {}),
            store: None,
            storage_error: None,
            artwork: crate::visuals::Artwork::load(),
            captain_picker: false,
            search_open: false,
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
        if captain >= crate::visuals::CAPTAIN_NAMES.len() {
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
            thread.title = crate::visuals::CAPTAIN_NAMES[captain].into();
            thread.captain = Some(captain);
            thread.captain_home = true;
            self.threads.push(thread);
            self.active = self.threads.len() - 1;
        }
        self.show_archive = false;
        self.captain_picker = false;
        self.checkpoint();
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
        self.current_mut().connect(path, wake);
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
            Ok(Some(index)) if index["version"] == 1 => {
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
                            thread.captain = record["captain"]
                                .as_u64()
                                .and_then(|id| usize::try_from(id).ok())
                                .filter(|&id| id < crate::visuals::CAPTAIN_NAMES.len());
                            thread.captain_home = record["captainHome"] == true;
                            if let (true, Some(captain)) = (thread.captain_home, thread.captain) {
                                // Older indexes stored the role name as the home title.
                                thread.title = crate::visuals::CAPTAIN_NAMES[captain].into();
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
            "version":1, "dark":self.dark,
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
        thread.model = "Pi".into();
        thread.thinking = "medium".into();
        thread.elapsed = Some(42);
        thread.messages = vec![
            Message { role: "You", text: "Fix the reconnect race, and add a regression test for the duplicate listener.".into() },
            Message { role: "Pi", text: "I'll check the socket lifecycle and where the reconnect listener attaches before making the change.".into() },
            Message { role: "Pi", text: "The listener now attaches before the socket opens. Cleanup removes it before the next attempt, so a reconnect cannot register it twice.\n\nThe example regression test covers a disconnect during initialization and a second reconnect. Both paths keep a single listener.\n\n```diff\n- socket.open();\n  socket.on(\"reconnect\", reconnect);\n+ socket.open();\n```\n\nThe thread is settled. There is no queued follow-up work.".into() },
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
            (2, "Review sidebar keyboard navigation", Status::Ready),
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
        // One home per captain, in three different states, so the crew shows its range.
        for (id, captain, status) in [
            (7, 0, Status::Running),
            (8, 1, Status::Waiting),
            (9, 2, Status::Settled),
        ] {
            let mut thread = Thread::new(id);
            thread.title = crate::visuals::CAPTAIN_NAMES[captain].into();
            thread.captain = Some(captain);
            thread.captain_home = true;
            thread.status = status;
            self.threads.push(thread);
        }
        self.next_id = 10;
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
                Event::Error(error) => thread.error = Some(error),
                Event::Diagnostic(diagnostic) => thread.diagnostics.push(diagnostic),
                Event::Exited => {
                    thread.pi = None;
                    thread.status = if thread.error.is_some() {
                        Status::Failed
                    } else {
                        Status::Offline
                    };
                }
            }
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
}
