//! Codex app-server JSONL transport. Ported from KSG Code's MIT-licensed
//! effect-codex-app-server/client.ts and replay.ts, and CodexAdapterV2.ts.
//! Codex owns its provider configuration, authentication and session files.
use crate::{
    backend::{self, Event, Pi, SessionOwner, SessionSource},
    harness::Start,
};
use rune::Waker;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub fn process() -> Result<Command, String> {
    let mut candidates = vec![];
    if let Some(path) = std::env::var_os("SHINA_CODEX_CLI") {
        candidates.push(path.to_string_lossy().into_owned());
    } else {
        candidates.push("codex".into());
        if cfg!(windows)
            && let Some(local) = std::env::var_os("LOCALAPPDATA")
        {
            candidates.push(
                PathBuf::from(local)
                    .join("Programs/OpenAI/Codex/bin/codex.exe")
                    .to_string_lossy()
                    .into_owned(),
            );
            candidates.push(
                PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
                    .join("AppData/Local/Programs/OpenAI/Codex/bin/codex.exe")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    for command in candidates {
        if let Ok((executable, script)) = crate::harness::executable(&command) {
            let mut command = Command::new(executable);
            if let Some(script) = script {
                command.arg(script);
            }
            // Preserve Codex's own configured provider environment, like KSG. Do not
            // propagate another Shina thread's bearer token or Pi bridge settings.
            command
                .arg("app-server")
                .env_remove(crate::mcp::URL_ENV)
                .env_remove(crate::mcp::TOKEN_ENV)
                .env_remove("SHINA_RUNTIME_MODE")
                .env_remove("SHINA_PI_BRIDGE_PATH");
            return Ok(command);
        }
    }
    Err("Codex was not found. Install and configure Codex outside Shina, or set SHINA_CODEX_CLI to its executable or JavaScript entry point.".into())
}

pub fn start(start: Start, wake: Waker) -> Result<Pi, String> {
    start_process(process()?, start, wake)
}

fn start_process(command: Command, start: Start, wake: Waker) -> Result<Pi, String> {
    if matches!(start.source, SessionSource::Fork(_)) {
        return Err("Codex conversation forks are not implemented in Shina.".into());
    }
    if start.link.is_none() {
        return Err("Codex requires Shina's thread-scoped MCP endpoint.".into());
    }
    let key = match &start.source {
        SessionSource::Resume(id) => Some(PathBuf::from(format!("codex:{id}"))),
        _ => None,
    };
    let owner = start.owner.clone();
    #[cfg(test)]
    let profile = crate::store::directory().expect("isolated profile");
    Ok(Pi::worker(
        wake,
        owner,
        key,
        move |input, events, wake, stopped, owner| {
            let action = || run(command, start, input, events, wake, stopped, owner);
            #[cfg(test)]
            {
                crate::store::with_directory(profile, action)
            }
            #[cfg(not(test))]
            {
                action()
            }
        },
    ))
}

/// KSG's runtime modes, with a non-escalating read-only mode for captain homes.
fn policy(access: &str) -> (&'static str, &'static str, Value) {
    match access {
        "full-access" => (
            "never",
            "danger-full-access",
            json!({"type":"dangerFullAccess"}),
        ),
        "auto-accept-edits" => (
            "on-request",
            "workspace-write",
            json!({"type":"workspaceWrite"}),
        ),
        "read-only" => ("never", "read-only", json!({"type":"readOnly"})),
        _ => ("untrusted", "read-only", json!({"type":"readOnly"})),
    }
}

struct Pending {
    request: Value,
    deadline: Instant,
}

struct Approval {
    id: Value,
    method: String,
    params: Value,
}

struct Client {
    start: Start,
    session: Option<String>,
    turn: Option<String>,
    ready: bool,
    running: bool,
    cancelling: bool,
    interrupt_sent: bool,
    serial: usize,
    pending: HashMap<String, Pending>,
    approvals: HashMap<String, Approval>,
    queued: VecDeque<Value>,
    models: Vec<Value>,
    raw_models: Vec<Value>,
    model: Value,
    effort: String,
    usage: Value,
    history: Vec<Value>,
    items: HashMap<String, Value>,
    text: HashMap<String, String>,
    reasoning_indexes: HashMap<String, usize>,
    active_message: Option<String>,
    completed: HashSet<String>,
    wire: Vec<Value>,
    events: Vec<Value>,
}

impl Client {
    fn new(start: Start) -> Self {
        Self {
            start,
            session: None,
            turn: None,
            ready: false,
            running: false,
            cancelling: false,
            interrupt_sent: false,
            serial: 0,
            pending: HashMap::new(),
            approvals: HashMap::new(),
            queued: VecDeque::new(),
            models: vec![],
            raw_models: vec![],
            model: Value::Null,
            effort: String::new(),
            usage: Value::Null,
            history: vec![],
            items: HashMap::new(),
            text: HashMap::new(),
            reasoning_indexes: HashMap::new(),
            active_message: None,
            completed: HashSet::new(),
            wire: vec![],
            events: vec![],
        }
    }

    fn access(&self) -> &str {
        self.start
            .link
            .as_ref()
            .map_or("read-only", |link| link.access_mode.as_str())
    }

    fn request(&mut self, method: &str, params: Value, request: Value) {
        self.serial += 1;
        let id = format!("shina-codex-{}", self.serial);
        self.pending.insert(
            id.clone(),
            Pending {
                request,
                deadline: Instant::now() + Duration::from_secs(30),
            },
        );
        // app-server omits the JSON-RPC version, exactly as KSG's patched protocol does.
        self.wire
            .push(json!({"id":id,"method":method,"params":params}));
    }

    fn initialize(&mut self) {
        self.request(
            "initialize",
            json!({"clientInfo":{"name":"shina","title":"Shina","version":env!("CARGO_PKG_VERSION")},
                "capabilities":{"experimentalApi":true}}),
            json!({"type":"initialize"}),
        );
    }

    /// Ported from KSG's codexThreadRuntimeParams and CodexDeveloperInstructions.
    /// Supply this on BOTH start and resume. Never overwrite baseInstructions or login.
    fn session_params(&self) -> Value {
        let link = self.start.link.as_ref().expect("MCP required at boundary");
        let (approval, sandbox, _) = policy(self.access());
        json!({
            "cwd":self.start.workspace,
            "approvalPolicy":approval,"approvalsReviewer":"user","sandbox":sandbox,
            "developerInstructions":developer_instructions(&self.start),
            "config":{
                "tools.update_plan.enabled":true,
                "mcp_optional_startup_grace_ms":10000,
                "features.concurrent_reasoning_summaries":true,
                "mcp_servers.shina":{
                    "url":link.url,"http_headers":{"Authorization":format!("Bearer {}",link.token)},
                    "tool_timeout_sec":660,"required":true
                }
            }
        })
    }

    fn state(&self) -> Value {
        json!({"sessionFile":self.session,"isStreaming":self.running,
            "model":self.model,"thinkingLevel":self.effort})
    }

    fn reply(&mut self, request: &Value, data: Value) {
        self.events
            .push(json!({"type":"response","id":request["id"],
            "command":request["type"],"success":true,"data":data}));
    }

    fn reject(&mut self, request: &Value, error: impl Into<String>) {
        self.events
            .push(json!({"type":"response","id":request["id"],
            "command":request["type"],"success":false,"error":error.into()}));
    }

    fn incoming(&mut self, record: Value) -> Result<(), String> {
        if let Some(method) = record["method"].as_str() {
            let params = &record["params"];
            if let Some(id) = params["threadId"].as_str()
                && self.session.as_deref().is_some_and(|session| session != id)
            {
                return Err("Codex sent an event for another thread.".into());
            }
            if record.get("id").is_some() {
                self.server_request(method, &record);
            } else {
                self.notification(method, params)?;
            }
            return Ok(());
        }
        let id = record["id"]
            .as_str()
            .ok_or("Codex response lacks a request id.")?;
        let pending = self
            .pending
            .remove(id)
            .ok_or("Codex answered an unknown request.")?;
        let request = pending.request;
        let kind = request["type"].as_str().unwrap_or("");
        if !record["error"].is_null() {
            let error = self.redact(
                record["error"]["message"]
                    .as_str()
                    .unwrap_or("Unknown error"),
            );
            let error = format!(
                "Codex {kind} failed: {error}. Configure providers or sign in outside Shina if required, then retry."
            );
            if matches!(kind, "initialize" | "start" | "models" | "prompt") {
                // Never retry an accepted turn, or attempt login after a denial.
                return Err(error);
            }
            self.reject(&request, error);
            return Ok(());
        }
        let result = &record["result"];
        match kind {
            "initialize" => {
                self.wire.push(json!({"method":"initialized"}));
                self.request("model/list", json!({"limit":100}), json!({"type":"models"}));
            }
            "models" => {
                let page = result["data"]
                    .as_array()
                    .ok_or("Codex returned an invalid model list.")?;
                self.raw_models.extend(page.iter().cloned());
                if self.raw_models.len() > 4096 {
                    return Err("Codex model list exceeds Shina's limit.".into());
                }
                if let Some(cursor) = result["nextCursor"].as_str().filter(|s| !s.is_empty()) {
                    self.request(
                        "model/list",
                        json!({"limit":100,"cursor":cursor}),
                        json!({"type":"models"}),
                    );
                } else {
                    self.models = self
                        .raw_models
                        .iter()
                        .filter(|m| m["hidden"] != true)
                        .filter_map(model_value)
                        .collect();
                    let mut params = self.session_params();
                    let method = match &self.start.source {
                        SessionSource::New => "thread/start",
                        SessionSource::Resume(id) => {
                            params["threadId"] = json!(id);
                            "thread/resume"
                        }
                        SessionSource::Fork(_) => return Err("Codex forks are unsupported.".into()),
                    };
                    self.request(method, params, json!({"type":"start"}));
                }
            }
            "start" => {
                let id = result["thread"]["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or("Codex did not return a thread id.")?;
                if let SessionSource::Resume(expected) = &self.start.source
                    && id != expected
                {
                    return Err("Codex resumed a different thread.".into());
                }
                self.session = Some(id.into());
                let model = result["model"]
                    .as_str()
                    .or_else(|| result["thread"]["model"].as_str());
                self.model = model
                    .and_then(|id| self.models.iter().find(|m| m["id"] == id).cloned())
                    .or_else(|| model.map(|id| json!({"provider":"codex","id":id,"name":id})))
                    .unwrap_or(Value::Null);
                self.effort = result["reasoningEffort"]
                    .as_str()
                    .or_else(|| result["thread"]["reasoningEffort"].as_str())
                    .or_else(|| {
                        self.raw_models
                            .iter()
                            .find(|m| Some(m["model"].as_str().unwrap_or("")) == model)
                            .and_then(|m| m["defaultReasoningEffort"].as_str())
                    })
                    .unwrap_or("")
                    .into();
                for turn in result["thread"]["turns"].as_array().into_iter().flatten() {
                    self.replay(turn);
                }
                if let Some(active) = result["thread"]["turns"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|t| t["status"] == "inProgress")
                {
                    self.running = true;
                    self.turn = active["id"].as_str().map(str::to_owned);
                }
                self.ready = true;
                self.reply(
                    &json!({"type":"get_available_models","id":"models"}),
                    json!({"models":self.models}),
                );
                self.reply(
                    &json!({"type":"get_available_thinking_levels","id":"levels"}),
                    json!({"levels":self.levels()}),
                );
                self.reply(&json!({"type":"get_state","id":"state"}), self.state());
                self.reply(
                    &json!({"type":"get_messages","id":"history"}),
                    json!({"messages":self.history}),
                );
                self.reply(
                    &json!({"type":"get_commands","id":"commands"}),
                    json!({"commands":[]}),
                );
                self.events
                    .push(json!({"type":"harness_update","field":"awareness","value":true}));
            }
            "prompt" => {
                if self.turn.is_none() && self.running {
                    self.turn = result["turn"]["id"].as_str().map(str::to_owned);
                    if self.cancelling && !self.interrupt_sent {
                        self.interrupt_sent = true;
                        self.request(
                            "turn/interrupt",
                            json!({"threadId":self.session,"turnId":self.turn}),
                            json!({"type":"abort"}),
                        );
                    }
                }
                self.reply(&request, json!({}));
            }
            "abort" => self.reply(&request, json!({})),
            "steer" => self.reply(&request["original"], json!({})),
            _ => self.reply(&request, result.clone()),
        }
        Ok(())
    }

    fn redact(&self, message: &str) -> String {
        let message = if let Some(link) = &self.start.link {
            message.replace(&link.token, "[redacted]")
        } else {
            message.to_owned()
        };
        crate::update::redact(&crate::projects::redact(&message))
    }

    fn levels(&self) -> Value {
        self.model
            .get("thinkingLevels")
            .cloned()
            .unwrap_or_else(|| json!([]))
    }

    fn command(&mut self, record: Value) -> Result<(), String> {
        let kind = record["type"].as_str().unwrap_or("");
        if kind == "extension_ui_response" {
            if let Some(approval) = record["id"]
                .as_str()
                .and_then(|id| self.approvals.remove(id))
            {
                if approval.method == "item/tool/requestUserInput" {
                    let supplied: Value = record["value"]
                        .as_str()
                        .and_then(|text| serde_json::from_str(text).ok())
                        .unwrap_or(Value::Null);
                    let mut answers = serde_json::Map::new();
                    if record["cancelled"] != true {
                        for question in approval.params["questions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                        {
                            if let Some(id) = question["id"].as_str()
                                && let Some(value) = supplied.get(id)
                            {
                                let values = value
                                    .as_array()
                                    .cloned()
                                    .unwrap_or_else(|| vec![value.clone()]);
                                if values.iter().all(Value::is_string) {
                                    answers.insert(id.into(), json!({"answers":values}));
                                }
                            }
                        }
                    }
                    self.wire
                        .push(json!({"id":approval.id,"result":{"answers":answers}}));
                    return Ok(());
                }
                let allow = record["confirmed"] == true
                    && record["cancelled"] != true
                    && self.access() != "read-only";
                self.answer_approval(approval, allow);
            }
            return Ok(());
        }
        if !self.ready {
            return Err("Codex session is not initialized.".into());
        }
        match kind {
            "get_state" => self.reply(&record, self.state()),
            "get_messages" | "get_entries" => {
                let mut record = record;
                record["type"] = json!("get_messages");
                self.reply(&record, json!({"messages":self.history}));
            }
            "get_available_models" => self.reply(&record, json!({"models":self.models})),
            "get_available_thinking_levels" => self.reply(&record, json!({"levels":self.levels()})),
            "get_session_stats" => self.reply(&record, json!({"contextUsage":self.usage})),
            "get_commands" => self.reply(&record, json!({"commands":[]})),
            "set_session_name" => self.reply(&record, json!({})),
            "clear_queue" => {
                let texts: Vec<_> = self
                    .queued
                    .drain(..)
                    .map(|r| r["message"].clone())
                    .collect();
                self.reply(&record, json!({"steering":[],"followUp":texts}));
                self.queue_update();
            }
            "abort" => {
                self.cancelling = self.running;
                for (_, approval) in std::mem::take(&mut self.approvals) {
                    self.answer_approval(approval, false);
                }
                if let Some(turn) = &self.turn {
                    self.interrupt_sent = true;
                    self.request(
                        "turn/interrupt",
                        json!({"threadId":self.session,"turnId":turn}),
                        record,
                    );
                } else if self.running {
                    // turn/start is in flight. Interrupt as soon as its notification supplies an id.
                    self.reply(&record, json!({}));
                } else {
                    self.reply(&record, json!({}));
                    self.events
                        .push(json!({"type":"agent_settled","aborted":true}));
                }
            }
            "prompt" if self.running => {
                // KSG uses turn/steer with expectedTurnId. Follow-ups retain FIFO ordering.
                if record["streamingBehavior"] == "steer"
                    && let Some(turn) = &self.turn
                {
                    let input = codex_input(&record)?;
                    self.request(
                        "turn/steer",
                        json!({"threadId":self.session,"expectedTurnId":turn,"input":input}),
                        json!({"type":"steer","original":record}),
                    );
                } else if self.queued.len() < 32 {
                    self.queued.push_back(record.clone());
                    self.reply(&record, json!({"disposition":"queued"}));
                    self.queue_update();
                } else {
                    self.reject(&record, "Codex follow-up queue is full.");
                }
            }
            "prompt" => self.prompt(record)?,
            "set_model" => {
                if self.running {
                    self.reject(&record, "Wait for Codex to finish before changing models.");
                } else if let Some(model) = self
                    .models
                    .iter()
                    .find(|m| m["id"] == record["modelId"])
                    .cloned()
                {
                    self.model = model;
                    if !self
                        .levels()
                        .as_array()
                        .is_some_and(|levels| levels.contains(&json!(self.effort)))
                    {
                        self.effort = self
                            .raw_models
                            .iter()
                            .find(|m| m["model"] == self.model["id"])
                            .and_then(|m| m["defaultReasoningEffort"].as_str())
                            .unwrap_or("")
                            .into();
                    }
                    self.usage = Value::Null;
                    self.reply(&record, self.model.clone());
                } else {
                    self.reject(&record, "Choose a model returned by Codex model/list.");
                }
            }
            "set_thinking_level" => {
                if self.running
                    || !self
                        .levels()
                        .as_array()
                        .is_some_and(|levels| levels.contains(&record["level"]))
                {
                    self.reject(
                        &record,
                        "Choose an advertised Codex reasoning effort while idle.",
                    );
                } else {
                    self.effort = record["level"].as_str().unwrap_or("").into();
                    self.reply(&record, json!({"thinkingLevel":self.effort}));
                }
            }
            _ => self.reject(&record, format!("Codex does not support {kind} in Shina.")),
        }
        Ok(())
    }

    fn prompt(&mut self, record: Value) -> Result<(), String> {
        let input = codex_input(&record)?;
        let (approval, _, sandbox) = policy(self.access());
        let mut params = json!({"threadId":self.session,"cwd":self.start.workspace,
            "input":input,"approvalPolicy":approval,"approvalsReviewer":"user",
            "sandboxPolicy":sandbox,"summary":"detailed"});
        if let Some(model) = self.model["id"].as_str() {
            params["model"] = json!(model);
        }
        if !self.effort.is_empty() {
            params["effort"] = json!(self.effort);
        }
        self.running = true;
        self.cancelling = false;
        self.interrupt_sent = false;
        self.turn = None;
        self.events.push(json!({"type":"agent_start"}));
        self.request("turn/start", params, record);
        Ok(())
    }

    fn queue_update(&mut self) {
        let texts: Vec<_> = self.queued.iter().map(|r| r["message"].clone()).collect();
        self.events
            .push(json!({"type":"queue_update","steering":[],"followUp":texts}));
    }

    fn server_request(&mut self, method: &str, record: &Value) {
        if method == "item/tool/requestUserInput" {
            let questions = record["params"]["questions"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if questions.is_empty()
                || questions.len() > 32
                || questions.iter().any(|q| q["isSecret"] == true)
            {
                self.wire
                    .push(json!({"id":record["id"],"error":{"code":-32602,
                    "message":"Shina supports up to 32 non-secret questions."}}));
                return;
            }
            let questions: Vec<_> = questions
                .into_iter()
                .map(|mut q| {
                    if !q["options"].is_array() {
                        q["options"] = json!([]);
                    }
                    q
                })
                .collect();
            if self.cancelling || self.approvals.len() >= 32 {
                self.wire
                    .push(json!({"id":record["id"],"result":{"answers":{}}}));
            } else {
                self.serial += 1;
                let key = format!("codex-question-{}", self.serial);
                self.approvals.insert(
                    key.clone(),
                    Approval {
                        id: record["id"].clone(),
                        method: method.into(),
                        params: record["params"].clone(),
                    },
                );
                self.events.push(
                    json!({"type":"extension_ui_request","id":key,"method":"input",
                    "title":"Codex needs your input","questions":questions}),
                );
            }
            return;
        }
        if matches!(
            method,
            "item/commandExecution/requestApproval"
                | "item/fileChange/requestApproval"
                | "item/permissions/requestApproval"
        ) {
            let approval = Approval {
                id: record["id"].clone(),
                method: method.into(),
                params: record["params"].clone(),
            };
            if self.access() == "read-only" || self.cancelling || self.approvals.len() >= 32 {
                self.answer_approval(approval, false);
            } else if self.access() == "full-access"
                || self.access() == "auto-accept-edits"
                    && method == "item/fileChange/requestApproval"
            {
                self.answer_approval(approval, true);
            } else {
                self.serial += 1;
                let key = format!("codex-approval-{}", self.serial);
                let params = &record["params"];
                self.events.push(json!({"type":"extension_ui_request","id":key,"method":"confirm",
                    "title":if method == "item/fileChange/requestApproval" {"Allow Codex file changes?"} else {"Allow Codex execution or permissions?"},
                    "message":self.redact(&format!("{}\n{}\n{}",params["reason"].as_str().unwrap_or(""),
                        params["command"].as_str().unwrap_or(""),params))}));
                self.approvals.insert(key, approval);
            }
        } else {
            // Never silently grant unknown requests, including dynamic tools and user input.
            self.wire
                .push(json!({"id":record["id"],"error":{"code":-32601,
                "message":format!("Shina does not support Codex request {method}.")}}));
            self.events
                .push(json!({"type":"harness_update","field":"unsupportedRequest","value":method}));
        }
    }

    fn answer_approval(&mut self, approval: Approval, allow: bool) {
        let result = if approval.method == "item/tool/requestUserInput" {
            json!({"answers":{}})
        } else if approval.method == "item/permissions/requestApproval" {
            json!({"permissions":if allow {approval.params["permissions"].clone()} else {json!({})},"scope":"turn"})
        } else {
            json!({"decision":if allow {"accept"} else {"decline"}})
        };
        self.wire.push(json!({"id":approval.id,"result":result}));
    }

    fn notification(&mut self, method: &str, params: &Value) -> Result<(), String> {
        match method {
            "turn/started" => {
                self.running = true;
                self.turn = params["turn"]["id"].as_str().map(str::to_owned);
                if self.cancelling && !self.interrupt_sent {
                    self.interrupt_sent = true;
                    self.request(
                        "turn/interrupt",
                        json!({"threadId":self.session,"turnId":self.turn}),
                        json!({"type":"abort"}),
                    );
                }
            }
            "item/started" | "item/completed" => {
                self.item(&params["item"], method == "item/completed", true)
            }
            "item/agentMessage/delta" => self.delta(params, false),
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                self.delta(params, true)
            }
            "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
                let id = params["itemId"].as_str().unwrap_or("");
                let output = self.text.entry(format!("output:{id}")).or_default();
                output.push_str(params["delta"].as_str().unwrap_or(""));
                self.events
                    .push(json!({"type":"tool_execution_update","toolCallId":id,
                    "partialResult":{"content":[{"type":"text","text":output}]}}));
            }
            "turn/plan/updated" => self
                .events
                .push(json!({"type":"harness_update","field":"plan","value":params})),
            "thread/tokenUsage/updated" => {
                // KSG uses last, not total. total is cumulative across turns.
                let usage = &params["tokenUsage"];
                let window = usage["modelContextWindow"].as_u64().filter(|n| *n > 0);
                let tokens = usage["last"]["totalTokens"].as_u64();
                self.usage = json!({"tokens":tokens,"contextWindow":window});
                if let Some(window) = window {
                    self.model["contextWindow"] = json!(window);
                }
            }
            "turn/completed" => {
                if !self.running {
                    return Ok(());
                }
                let turn = &params["turn"];
                if let Some(id) = turn["id"].as_str()
                    && self.turn.as_deref().is_some_and(|active| active != id)
                {
                    return Err("Codex completed a different active turn.".into());
                }
                for item in turn["items"].as_array().into_iter().flatten() {
                    self.item(item, true, true);
                }
                let failed = turn["status"] == "failed";
                self.finish_history_turn(turn);
                if failed {
                    self.events.push(json!({"type":"message_end","message":{"role":"assistant","stopReason":"error",
                        "errorMessage":self.redact(turn["error"]["message"].as_str().unwrap_or("Codex turn failed."))}}));
                }
                let aborted = self.cancelling || turn["status"] == "interrupted";
                self.running = false;
                self.turn = None;
                self.cancelling = false;
                self.events
                    .push(json!({"type":"agent_settled","aborted":aborted}));
                if !aborted
                    && !failed
                    && let Some(record) = self.queued.pop_front()
                {
                    self.queue_update();
                    self.prompt(record)?;
                }
            }
            "error" if params["willRetry"] != true => {
                return Err(self.redact(
                    params["error"]["message"]
                        .as_str()
                        .unwrap_or("Codex failed."),
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn delta(&mut self, params: &Value, thinking: bool) {
        let id = params["itemId"].as_str().unwrap_or("");
        if !thinking {
            self.begin_message(id);
        }
        let key = if thinking {
            format!(
                "reasoning:{id}:{}:{}",
                if params.get("summaryIndex").is_some() {
                    "summary"
                } else {
                    "text"
                },
                params["summaryIndex"]
                    .as_u64()
                    .or_else(|| params["contentIndex"].as_u64())
                    .unwrap_or(0)
            )
        } else {
            format!("message:{id}")
        };
        let new = !self.text.contains_key(&key);
        self.text
            .entry(key.clone())
            .or_default()
            .push_str(params["delta"].as_str().unwrap_or(""));
        // Use distinct indexes for reasoning sections, without duplicating completed item text.
        let next = self.reasoning_indexes.len();
        let index = *self.reasoning_indexes.entry(key).or_insert(next);
        self.events
            .push(json!({"type":"message_update","assistantMessageEvent":{
                "type":if thinking {"thinking_delta"} else {"text_delta"},
                "delta":params["delta"],"contentIndex":if thinking {index} else {0}
            }}));
        if new && !thinking {
            self.items
                .entry(id.into())
                .or_insert_with(|| json!({"type":"agentMessage","id":id}));
        }
    }

    fn item(&mut self, item: &Value, done: bool, emit: bool) {
        let Some(id) = item["id"].as_str() else {
            return;
        };
        if self.completed.contains(id) {
            return;
        }
        let new = !self.items.contains_key(id);
        self.items.insert(id.into(), item.clone());
        let kind = item["type"].as_str().unwrap_or("");
        match kind {
            "userMessage" if done => {
                let content = replay_input(&item["content"]);
                self.history.push(json!({"role":"user","content":content}));
                if emit {
                    self.events.push(
                        json!({"type":"message_start","message":{"role":"user","content":content}}),
                    );
                }
            }
            "agentMessage" if done => {
                if emit {
                    self.begin_message(id);
                }
                let text = item["text"].as_str().unwrap_or("");
                let key = format!("message:{id}");
                let streamed = self.text.remove(&key).unwrap_or_default();
                if emit && let Some(tail) = text.strip_prefix(&streamed).filter(|s| !s.is_empty()) {
                    self.events.push(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":tail,"contentIndex":0}}));
                }
                self.history
                    .push(json!({"role":"assistant","content":[{"type":"text","text":text}]}));
            }
            "reasoning" if done => {
                for (stream, field) in [("summary", "summary"), ("text", "content")] {
                    for (index, part) in item[field].as_array().into_iter().flatten().enumerate() {
                        let text = part
                            .as_str()
                            .or_else(|| part["text"].as_str())
                            .unwrap_or("");
                        let key = format!("reasoning:{id}:{stream}:{index}");
                        let next = self.reasoning_indexes.len();
                        let index = *self.reasoning_indexes.entry(key.clone()).or_insert(next);
                        let streamed = self.text.remove(&key).unwrap_or_default();
                        if emit
                            && let Some(tail) =
                                text.strip_prefix(&streamed).filter(|s| !s.is_empty())
                        {
                            self.events.push(json!({"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":tail,"contentIndex":index}}));
                        }
                        if !text.is_empty() {
                            self.history.push(json!({"role":"assistant","content":[{"type":"thinking","thinking":text}]}));
                        }
                    }
                }
            }
            "plan" if done => self
                .events
                .push(json!({"type":"harness_update","field":"plan","value":item})),
            "userMessage" | "agentMessage" | "reasoning" | "plan" => {}
            _ => {
                let name = match kind {
                    "commandExecution" => "command",
                    "fileChange" => "patch",
                    "mcpToolCall" | "dynamicToolCall" => {
                        item["tool"].as_str().unwrap_or("Codex tool")
                    }
                    _ => kind,
                };
                if new {
                    self.history.push(json!({"role":"assistant","content":[{"type":"toolCall","id":id,"name":name,"arguments":item}]}));
                    if emit {
                        self.events.push(json!({"type":"tool_execution_start","toolCallId":id,"toolName":name,"args":item}));
                    }
                }
                if done {
                    let output = item["aggregatedOutput"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| item.to_string());
                    let failed = matches!(item["status"].as_str(), Some("failed" | "declined"))
                        || item["exitCode"].as_i64().is_some_and(|code| code != 0);
                    let content = json!([{"type":"text","text":output}]);
                    self.history.push(json!({"role":"toolResult","toolCallId":id,"content":content,"isError":failed}));
                    if emit {
                        self.events
                            .push(json!({"type":"tool_execution_end","toolCallId":id,
                        "result":{"content":content},"isError":failed}));
                    }
                }
            }
        }
        if done {
            self.completed.insert(id.into());
            self.items.remove(id);
            self.text.remove(&format!("output:{id}"));
        }
    }

    fn replay(&mut self, turn: &Value) {
        for item in turn["items"].as_array().into_iter().flatten() {
            self.item(item, true, false);
        }
        self.finish_history_turn(turn);
    }

    fn begin_message(&mut self, id: &str) {
        if self.active_message.as_deref() != Some(id) {
            self.active_message = Some(id.into());
            self.events
                .push(json!({"type":"message_start","message":{"role":"assistant"}}));
        }
    }

    fn finish_history_turn(&mut self, turn: &Value) {
        // Only a successfully settled turn proves a final reply. Commentary before an
        // interrupt or crash must never make durable task recovery report success.
        if let Some(message) = self.history.last_mut().filter(|m| m["role"] == "assistant") {
            message["stopReason"] = json!(match turn["status"].as_str() {
                Some("completed") => "stop",
                Some("failed") => "error",
                _ => "aborted",
            });
        }
        self.active_message = None;
        self.reasoning_indexes.clear();
    }
}

fn model_value(model: &Value) -> Option<Value> {
    let id = model["model"].as_str().or_else(|| model["id"].as_str())?;
    let levels: Vec<_> = model["supportedReasoningEfforts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|level| level["reasoningEffort"].as_str())
        .collect();
    Some(
        json!({"provider":"codex","id":id,"name":model["displayName"].as_str().unwrap_or(id),
        "thinkingLevels":levels,"input":model.get("inputModalities").cloned().unwrap_or_else(||json!(["text","image"])),
        "contextWindow":model["contextWindow"]}),
    )
}

pub(crate) fn developer_instructions(start: &Start) -> String {
    format!(
        "{}\n\n{}",
        crate::mcp::INSTRUCTIONS,
        start.instructions.as_deref().unwrap_or("")
    )
}

/// KSG's toCodexInput uses text with absolute attachment paths and data-URL images.
fn codex_input(record: &Value) -> Result<Vec<Value>, String> {
    let mut text = record["message"].as_str().unwrap_or("").to_owned();
    let files: Vec<_> = record["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| {
            crate::attachments::Attachment::from_value(v)
                .ok_or("Invalid Codex attachment.".to_owned())
        })
        .collect::<Result<_, _>>()?;
    crate::attachments::validate_prompt_images(&files)?;
    for file in files {
        let path = file
            .path
            .as_ref()
            .ok_or("Codex attachment lacks an absolute path.")?;
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!(
            "[Attached file \"{}\" is saved at: {}]",
            file.name,
            path.display()
        ));
    }
    let mut input = vec![];
    if !text.is_empty() {
        input.push(json!({"type":"text","text":text}));
    }
    for image in record["images"].as_array().into_iter().flatten() {
        let mime = image["mimeType"]
            .as_str()
            .ok_or("Codex image lacks a MIME type.")?;
        let data = image["data"].as_str().ok_or("Codex image lacks data.")?;
        input.push(json!({"type":"image","url":format!("data:{mime};base64,{data}")}));
    }
    if input.is_empty() {
        return Err("Codex turn requires text or attachments.".into());
    }
    Ok(input)
}

fn replay_input(content: &Value) -> Value {
    json!(
        content
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| {
                match item["type"].as_str()? {
                    "text" => Some(json!({"type":"text","text":item["text"]})),
                    "image" => {
                        let url = item["url"].as_str()?;
                        let (mime, data) = url.strip_prefix("data:")?.split_once(";base64,")?;
                        Some(json!({"type":"image","mimeType":mime,"data":data}))
                    }
                    "localImage" => {
                        let path = std::path::Path::new(item["path"].as_str()?);
                        let image = crate::attachments::attach_file(path).ok()?;
                        image
                            .is_image()
                            .then(|| json!({"type":"image","shinaAttachment":image.to_value()}))
                    }
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
    )
}

fn run(
    mut command: Command,
    start: Start,
    input: Receiver<Value>,
    events: &SyncSender<Event>,
    wake: &Waker,
    stopped: &AtomicBool,
    owner: &SessionOwner,
) -> Result<(), String> {
    command
        .current_dir(&start.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = crate::process::Process::spawn(&mut command)
        .map_err(|e| format!("Could not start Codex: {e}"))?;
    let mut stdin = process.stdin.take().unwrap();
    let stdout = process.stdout.take().unwrap();
    let stderr = process.stderr.take().unwrap();
    let (received, records) = mpsc::sync_channel(256);
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let record = backend::read_record(&mut reader).map_err(|e| e.replace("Pi", "Codex"));
            let done = !matches!(record, Ok(Some(_)));
            if received.send(record).is_err() || done {
                break;
            }
        }
    });
    // Drain stderr without retaining it. Codex can print provider tokens and config details.
    let errors = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        loop {
            let mut bytes = vec![];
            match Read::by_ref(&mut reader)
                .take(8192)
                .read_until(b'\n', &mut bytes)
            {
                Ok(0) | Err(_) => break,
                _ => {}
            }
        }
    });
    let (writes, outgoing) = mpsc::sync_channel(32);
    let (result_tx, written) = mpsc::channel();
    let writing = Arc::new(std::sync::Mutex::new(None::<Instant>));
    let clock = writing.clone();
    let writer = thread::spawn(move || {
        let result = (|| {
            for record in outgoing {
                *clock.lock().unwrap() = Some(Instant::now());
                backend::write_record(&mut stdin, &record).map_err(|e| e.replace("Pi", "Codex"))?;
                *clock.lock().unwrap() = None;
            }
            Ok::<_, String>(())
        })();
        let _ = result_tx.send(result);
    });
    let mut client = Client::new(start);
    client.initialize();
    let result = (|| {
        loop {
            if stopped.load(Ordering::Acquire) {
                return Ok(());
            }
            if let Ok(result) = written.try_recv() {
                return result;
            }
            if writing
                .lock()
                .unwrap()
                .is_some_and(|at| at.elapsed() >= Duration::from_secs(30))
            {
                return Err("Codex stopped reading commands.".into());
            }
            if !client.approvals.is_empty() {
                for pending in client.pending.values_mut() {
                    pending.deadline = Instant::now() + Duration::from_secs(30);
                }
            }
            if client
                .pending
                .values()
                .any(|p| Instant::now() >= p.deadline)
            {
                return Err("Codex did not acknowledge an operation within 30 seconds.".into());
            }
            for record in std::mem::take(&mut client.wire) {
                writes
                    .try_send(record)
                    .map_err(|_| "Codex stopped accepting commands.")?;
            }
            for mut record in std::mem::take(&mut client.events) {
                crate::attachments::import_history(&mut record);
                if !backend::publish(events, wake, Event::Record(record)) {
                    return Ok(());
                }
            }
            for _ in 0..64 {
                match records.try_recv() {
                    Ok(Ok(Some(record))) => client.incoming(record)?,
                    Ok(Ok(None)) => {
                        return Err(
                            "Codex closed its output. Retry to resume its saved thread.".into()
                        );
                    }
                    Ok(Err(error)) => return Err(error),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("Codex output reader stopped.".into());
                    }
                }
            }
            if !client.wire.is_empty() || !client.events.is_empty() {
                continue;
            }
            match input.recv_timeout(Duration::from_millis(20)) {
                Ok(record) => client.command(record)?,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(status) = process.try_wait().map_err(|e| e.to_string())? {
                return Err(format!(
                    "Codex exited with {status}. Retry to resume its saved thread."
                ));
            }
        }
    })();
    for mut record in std::mem::take(&mut client.events) {
        crate::attachments::import_history(&mut record);
        if !backend::publish(events, wake, Event::Record(record)) {
            break;
        }
    }
    drop(writes);
    drop(records);
    process.terminate();
    if let Err(error) = process.reap(Duration::from_secs(1)) {
        owner.quarantined.store(true, Ordering::Release);
        return Err(format!(
            "Could not reap Codex. Restart Shina before reconnecting: {error}"
        ));
    }
    crate::process::join_for(writer, Duration::from_millis(200));
    crate::process::join_for(reader, Duration::from_millis(200));
    crate::process::join_for(errors, Duration::from_millis(200));
    result
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
