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

pub fn discover(workspace: PathBuf, wake: Waker, models: bool) -> Result<Pi, String> {
    let command = process()?;
    let start = Start::discovery("codex", workspace);
    Ok(Pi::worker(
        wake,
        start.owner.clone(),
        None,
        move |input, events, wake, stopped, owner| {
            run_mode(
                command,
                start,
                input,
                events,
                wake,
                stopped,
                owner,
                Some(models),
            )
        },
    ))
}

fn start_process(command: Command, start: Start, wake: Waker) -> Result<Pi, String> {
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
fn policy(access: &str) -> (&'static str, &'static str, &'static str, Value) {
    match access {
        "full-access" => (
            "never",
            "user",
            "danger-full-access",
            json!({"type":"dangerFullAccess"}),
        ),
        "auto" => (
            "on-request",
            "auto_review",
            "workspace-write",
            json!({"type":"workspaceWrite"}),
        ),
        "auto-accept-edits" => (
            "on-request",
            "user",
            "workspace-write",
            json!({"type":"workspaceWrite"}),
        ),
        "read-only" => ("never", "user", "read-only", json!({"type":"readOnly"})),
        _ => ("untrusted", "user", "read-only", json!({"type":"readOnly"})),
    }
}

const PROMPT_SUGGESTION_PROMPT: &str = "Predict the next message the user will send in this conversation. Reply with only that message, written as the user would type it, in under 20 words. Do not use tools. If there is no clear next step, reply with nothing.";

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
    retry_attempt: u64,
    retrying: bool,
    cancelling: bool,
    interrupt_sent: bool,
    compact_request: Option<Value>,
    compacting: bool,
    compaction_error: Option<String>,
    compactions: Vec<Value>,
    compacted_turns: HashSet<String>,
    serial: usize,
    pending: HashMap<String, Pending>,
    approvals: HashMap<String, Approval>,
    queued: VecDeque<Value>,
    models: Vec<Value>,
    raw_models: Vec<Value>,
    model_cursors: HashSet<String>,
    skill_commands: Vec<Value>,
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
    probe: Option<bool>,
    skills_loaded: bool,
    native_extensions: bool,
    service_tier: Option<String>,
    turns: Vec<Value>,
    turn_items: Vec<Value>,
    usage_limits: Value,
    usage_limits_raw: Value,
    usage_account: Value,
    history_mode: String,
    hydration: Option<(Value, Value)>,
    history_cursors: HashSet<String>,
    child_parents: HashMap<String, String>,
    goal: Value,
    prompt_suggestions: bool,
    active_run_id: Option<String>,
    settled_run_id: Option<String>,
    turn_serial: u64,
}

impl Client {
    fn new(start: Start) -> Self {
        let prompt_suggestions = start.prompt_suggestions;
        Self {
            start,
            session: None,
            turn: None,
            ready: false,
            running: false,
            retry_attempt: 0,
            retrying: false,
            cancelling: false,
            interrupt_sent: false,
            compact_request: None,
            compacting: false,
            compaction_error: None,
            compactions: vec![],
            compacted_turns: HashSet::new(),
            serial: 0,
            pending: HashMap::new(),
            approvals: HashMap::new(),
            queued: VecDeque::new(),
            models: vec![],
            raw_models: vec![],
            model_cursors: HashSet::new(),
            skill_commands: vec![],
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
            probe: None,
            skills_loaded: false,
            native_extensions: false,
            service_tier: None,
            turns: vec![],
            turn_items: vec![],
            usage_limits: Value::Null,
            usage_limits_raw: Value::Null,
            usage_account: Value::Null,
            history_mode: "legacy".into(),
            hydration: None,
            history_cursors: HashSet::new(),
            child_parents: HashMap::new(),
            goal: Value::Null,
            prompt_suggestions,
            active_run_id: None,
            settled_run_id: None,
            turn_serial: 0,
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
        let timeout = if matches!(request["type"].as_str(), Some("rateLimits" | "account")) {
            Duration::from_secs(5)
        } else {
            Duration::from_secs(30)
        };
        self.pending.insert(
            id.clone(),
            Pending {
                request,
                deadline: Instant::now() + timeout,
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
        let (approval, reviewer, sandbox, _) = policy(self.access());
        json!({
            "cwd":self.start.workspace,
            "approvalPolicy":approval,"approvalsReviewer":reviewer,"sandbox":sandbox,
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
            "turnCapabilities":TURN_CAPABILITIES,
            "isCompacting":self.compacting || self.compact_request.is_some(),
            "model":self.model,"thinkingLevel":self.effort,"serviceTier":self.service_tier})
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
                if let Some(parent) = self.child_parents.get(id).cloned() {
                    if record.get("id").is_some() {
                        self.server_request(method, &record);
                    } else {
                        self.child_notification(method, params, &parent)?;
                    }
                    return Ok(());
                }
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
            let error = auth_guidance(&error);
            if matches!(kind, "rateLimits" | "account") {
                // Usage and account metadata are optional. An older CLI or API-key account has none.
                return Ok(());
            }
            if kind == "skills" {
                let lower = error.to_ascii_lowercase();
                if matches!(record["error"]["code"].as_i64(), Some(401 | 403))
                    || [
                        "authentication",
                        "unauthorized",
                        "unauthorised",
                        "invalid api key",
                        "not logged in",
                    ]
                    .iter()
                    .any(|text| lower.contains(text))
                {
                    return Err(error);
                }
                self.skills_loaded = true;
                if self.probe.is_some() {
                    self.reply(
                        &json!({"type":"get_commands","id":"commands"}),
                        json!({"commands":self.commands()}),
                    );
                    return Ok(());
                }
                self.reject(&json!({"type":"get_commands","id":request["id"]}), error);
                return Ok(());
            }
            if matches!(
                kind,
                "initialize" | "start" | "models" | "prompt" | "goal_stop"
            ) {
                // Never retry an accepted turn, or attempt login after a denial.
                return Err(error);
            }
            if kind == "compact" {
                self.end_compaction(json!({"errorMessage":error}), true);
                self.compact_request = None;
                self.running = false;
                self.turn = None;
            }
            if kind == "steer" {
                if (self.compacting || self.compact_request.is_some()) && self.queued.len() < 32 {
                    self.queued.push_back(request["original"].clone());
                    self.reply(&request["original"], json!({"disposition":"queued"}));
                    self.queue_update();
                    return Ok(());
                }
                self.reject(&request["original"], error);
                return Ok(());
            }
            if kind == "prompt_suggestion" {
                return Ok(());
            }
            self.reject(&request, error);
            return Ok(());
        }
        let result = &record["result"];
        match kind {
            "initialize" => {
                self.native_extensions = supports_extensions(result);
                self.events.push(
                    json!({"type":"harness_update","field":"codexNativeExtensions",
                    "value":self.native_extensions}),
                );
                self.wire.push(json!({"method":"initialized"}));
                self.request(
                    "skills/list",
                    json!({"cwds":[self.start.workspace]}),
                    json!({"type":"skills","id":"commands"}),
                );
                if self.probe != Some(false) {
                    self.request("account/read", json!({}), json!({"type":"account"}));
                    self.request(
                        "account/rateLimits/read",
                        Value::Null,
                        json!({"type":"rateLimits"}),
                    );
                }
                if self.probe != Some(false) {
                    self.request("model/list", json!({"limit":100}), json!({"type":"models"}));
                } else {
                    self.ready = true;
                }
            }
            "account" => {
                let mut acct = result.clone();
                crate::usage_limits::conceal_account_json(&mut acct);
                self.usage_account = acct;
                self.events.push(json!({"type":"harness_update","field":"usageAccount","value":self.usage_account}));
            }
            "rateLimits" => {
                let read = &result["rateLimitsByLimitId"]["codex"];
                let read = if read.is_null() {
                    &result["rateLimits"]
                } else {
                    read
                };
                self.usage_limits = crate::usage_limits::codex(read, &self.usage_limits);
                self.events.push(json!({"type":"harness_update","field":"usageLimits","value":self.usage_limits}));
                let mut raw = json!({
                    "checkedAtMs": crate::lifecycle::now(),
                    "snapshot": result["rateLimits"],
                });
                if !result["rateLimitsByLimitId"].is_null() {
                    raw["rateLimitsByLimitId"] = result["rateLimitsByLimitId"].clone();
                }
                if !result["rateLimitResetCredits"].is_null() {
                    raw["rateLimitResetCredits"] = result["rateLimitResetCredits"].clone();
                }
                self.usage_limits_raw = raw;
                self.events.push(json!({"type":"harness_update","field":"usageLimitsRaw","value":self.usage_limits_raw}));
            }
            "consume_reset_credit" => {
                self.reply(&request, result.clone());
                self.request(
                    "account/rateLimits/read",
                    Value::Null,
                    json!({"type":"rateLimits"}),
                );
            }
            "skills" => {
                self.skill_commands =
                    crate::composer_skills::codex_commands(result, &self.start.workspace);
                self.skills_loaded = true;
                // Discovery may beat thread/start. Do not expose readiness before a session exists.
                if self.ready {
                    self.reply(
                        &json!({"type":"get_commands","id":request["id"]}),
                        json!({"commands":self.commands()}),
                    );
                }
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
                    if !self.model_cursors.insert(cursor.into()) {
                        return Err("Codex model list repeated a pagination cursor.".into());
                    }
                    self.request("model/list", json!({"limit":100,"cursor":cursor}), request);
                } else {
                    self.models = self
                        .raw_models
                        .iter()
                        .filter(|m| m["hidden"] != true)
                        .filter_map(model_value)
                        .collect();
                    let preferred = ["gpt-6-astra", "gpt-5.6-sol", "gpt-5.6-terra"]
                        .iter()
                        .find_map(|target| {
                            self.models
                                .iter()
                                .find(|m| {
                                    let id = m["id"].as_str().unwrap_or("");
                                    let fam = id
                                        .strip_prefix("openai.")
                                        .filter(|s| s.starts_with("gpt-"))
                                        .unwrap_or(id);
                                    fam == *target
                                })
                                .and_then(|m| m["id"].as_str())
                                .map(str::to_owned)
                        });
                    if let Some(preferred_slug) = preferred {
                        for m in &mut self.models {
                            m["isDefault"] = json!(m["id"] == preferred_slug);
                        }
                    }
                    if self.ready && self.probe.is_none() {
                        if let Some(model) = self
                            .models
                            .iter()
                            .find(|model| model["id"] == self.model["id"])
                        {
                            self.model = model.clone();
                        }
                        self.reply(
                            &json!({"type":"get_available_models","id":request["id"]}),
                            json!({"models":self.models}),
                        );
                        self.reply(
                            &json!({"type":"get_available_thinking_levels","id":"levels"}),
                            json!({"levels":self.levels()}),
                        );
                        return Ok(());
                    }
                    if self.probe.is_some() {
                        self.reply(
                            &json!({"type":"get_available_models","id":"models"}),
                            json!({"models":self.models}),
                        );
                        self.ready = true;
                        if self.skills_loaded {
                            self.reply(
                                &json!({"type":"get_commands","id":"commands"}),
                                json!({"commands":self.commands()}),
                            );
                        }
                        return Ok(());
                    }
                    let mut params = self.session_params();
                    let method = match &self.start.source {
                        SessionSource::New => {
                            if self.native_extensions {
                                params["historyMode"] = json!("paginated");
                            }
                            "thread/start"
                        }
                        SessionSource::Resume(id) => {
                            params["threadId"] = json!(id);
                            "thread/resume"
                        }
                        SessionSource::Fork(id) => {
                            if !self.native_extensions {
                                return Err("This Codex version has no verified conversation fork protocol. Update Codex outside Shina.".into());
                            }
                            params["threadId"] = json!(id);
                            params["deferGoalContinuation"] = json!(true);
                            "thread/fork"
                        }
                    };
                    self.request(method, params, json!({"type":"start"}));
                }
            }
            "start" => {
                self.history_mode = result["thread"]["historyMode"]
                    .as_str()
                    .unwrap_or("legacy")
                    .into();
                if self.native_extensions
                    && self.history_mode == "paginated"
                    && !matches!(self.start.source, SessionSource::New)
                {
                    self.begin_hydration(result.clone(), request)?;
                } else {
                    self.started(result)?;
                }
            }
            "hydrate" => {
                let page = result["data"]
                    .as_array()
                    .ok_or("Codex returned invalid turn history.")?;
                let (snapshot, _) = self
                    .hydration
                    .as_mut()
                    .ok_or("Codex history was not requested.")?;
                let turns = snapshot["thread"]["turns"].as_array_mut().unwrap();
                turns.extend(page.iter().cloned());
                if turns.len() > 100_000 {
                    return Err("Codex history exceeds Shina's turn limit.".into());
                }
                if let Some(cursor) = result["nextCursor"]
                    .as_str()
                    .filter(|cursor| !cursor.is_empty())
                {
                    if !self.history_cursors.insert(cursor.into()) {
                        return Err("Codex turn history repeated a pagination cursor.".into());
                    }
                    self.request_history(Some(cursor.into()));
                } else {
                    let (snapshot, original) = self.hydration.take().unwrap();
                    if original["type"] == "fork" {
                        self.reset_history(&snapshot["thread"]);
                        self.reply(&original, json!({}));
                    } else {
                        self.started(&snapshot)?;
                    }
                }
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
            // thread/compact/start returns {} before the compact turn finishes.
            "compact" => {}
            "goal_settings" => {
                if request["cancelled"] == true {
                    self.reply(
                        &json!({"type":"native_goal","id":request["id"]}),
                        json!({"cancelled":true}),
                    );
                } else {
                    self.request(
                        "thread/goal/set",
                        request["params"].clone(),
                        json!({"type":"native_goal","id":request["id"]}),
                    );
                }
            }
            "goal_state" | "goal_stop" => {
                self.goal = result["goal"].clone();
                self.events
                    .push(json!({"type":"harness_update","field":"goal","value":self.goal}));
            }
            "native_goal" => {
                self.goal = result["goal"].clone();
                self.events
                    .push(json!({"type":"harness_update","field":"goal","value":result["goal"]}));
                self.events.push(json!({"type":"extension_ui_request","method":"notify",
                    "message":if result["goal"].is_null() {"No goal is set.".into()} else {result["goal"].to_string()}}));
                self.reply(&request, result.clone());
            }
            "native_feedback" => {
                self.events
                    .push(json!({"type":"extension_ui_request","method":"notify",
                    "message":"Codex feedback was sent to OpenAI."}));
                self.reply(&request, result.clone());
            }
            "fork" => {
                self.begin_hydration(result.clone(), request)?;
            }
            "steer" => self.reply(&request["original"], json!({})),
            "prompt_suggestion" => {
                let req_run_id = request["runId"].as_str();
                if !self.running
                    && self.settled_run_id.is_some()
                    && req_run_id == self.settled_run_id.as_deref()
                {
                    let text = result["text"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    self.events.push(json!({
                        "type": "prompt_suggestion",
                        "runId": req_run_id,
                        "text": text,
                    }));
                }
            }
            _ => self.reply(&request, result.clone()),
        }
        Ok(())
    }

    fn begin_hydration(&mut self, mut result: Value, request: Value) -> Result<(), String> {
        let id = result["thread"]["id"]
            .as_str()
            .ok_or("Codex history lacks a thread id.")?;
        if request["type"] == "fork" && self.session.as_deref() != Some(id) {
            return Err("Codex reverted a different thread.".into());
        }
        if let SessionSource::Resume(expected) = &self.start.source
            && id != expected
        {
            return Err("Codex resumed a different thread.".into());
        }
        self.session = Some(id.into());
        result["thread"]["turns"] = json!([]);
        self.hydration = Some((result, request));
        self.history_cursors.clear();
        self.request_history(None);
        Ok(())
    }

    fn request_history(&mut self, cursor: Option<String>) {
        self.request("thread/turns/list",
            json!({"threadId":self.session,"limit":100,"sortDirection":"asc","itemsView":"full","cursor":cursor}),
            json!({"type":"hydrate"}));
    }

    fn started(&mut self, result: &Value) -> Result<(), String> {
        let id = result["thread"]["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Codex did not return a thread id.")?;
        if let SessionSource::Resume(expected) = &self.start.source
            && id != expected
        {
            return Err("Codex resumed a different thread.".into());
        }
        if let SessionSource::Fork(source) = &self.start.source
            && id == source
        {
            return Err(
                "Codex did not create a distinct fork. The source was left unchanged.".into(),
            );
        }
        self.session = Some(id.into());
        self.service_tier = result["serviceTier"].as_str().map(str::to_owned);
        self.events.push(json!({"type":"harness_update","field":"effectiveServiceTier","value":self.service_tier}));
        self.events.push(
            json!({"type":"harness_update","field":"codexHistoryMode","value":self.history_mode}),
        );
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
            self.turns.push(turn.clone());
        }
        if let Some(active) = self
            .turns
            .iter()
            .find(|turn| turn["status"] == "inProgress")
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
        self.reply(&json!({"type":"get_messages","id":"history"}),
            json!({"messages":self.history,"compactions":self.compactions,"forkMessages":self.fork_messages()}));
        if self.compacting {
            self.events.push(json!({"type":"compaction_start"}));
        }
        self.reply(
            &json!({"type":"get_commands","id":"commands"}),
            json!({"commands":self.commands()}),
        );
        self.events
            .push(json!({"type":"harness_update","field":"awareness","value":true}));
        if self.native_extensions {
            self.request(
                "thread/goal/get",
                json!({"threadId":self.session}),
                json!({"type":"goal_state"}),
            );
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

    fn commands(&self) -> Vec<Value> {
        let mut commands = self.skill_commands.clone();
        if self.native_extensions {
            commands.extend([
                json!({"name":"goal","description":"Set a goal Codex keeps working toward until it is done. Objective, or pause, resume, clear.","source":"Codex"}),
                json!({"name":"feedback","description":"Send this thread and Codex logs to OpenAI. Describe the issue, optionally.","source":"Codex"}),
            ]);
        }
        commands
    }

    fn cancel_goal_activation(&mut self) {
        for pending in self
            .pending
            .values_mut()
            .filter(|pending| pending.request["type"] == "goal_settings")
        {
            pending.request["cancelled"] = json!(true);
        }
    }

    fn fork_messages(&self) -> Vec<Value> {
        self.turns
            .iter()
            .flat_map(|turn| {
                turn["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|item| item["type"] == "userMessage")
                    .map(|item| {
                        let content = replay_input(&item["content"]);
                        let text = content
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        json!({"entryId":turn["id"],"text":text,"content":content})
                    })
            })
            .collect()
    }

    fn reset_history(&mut self, thread: &Value) {
        self.history.clear();
        self.items.clear();
        self.completed.clear();
        self.compactions.clear();
        self.compacted_turns.clear();
        self.text.clear();
        self.child_parents.clear();
        self.turns = thread["turns"].as_array().cloned().unwrap_or_default();
        for turn in self.turns.clone() {
            self.replay(&turn);
        }
        self.invalidate_usage();
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
        if kind == "prompt" {
            let mut words = record["message"]
                .as_str()
                .unwrap_or("")
                .trim()
                .splitn(2, char::is_whitespace);
            if let Some(name @ ("/goal" | "/feedback")) = words.next() {
                if !self.native_extensions {
                    self.reject(
                        &record,
                        "This Codex version has no verified native command protocol.",
                    );
                    return Ok(());
                }
                return self.command(json!({"type":"native_command","id":record["id"],
                    "name":name.trim_start_matches('/'),"arguments":words.next().unwrap_or("")}));
            }
        }
        match kind {
            "get_state" => self.reply(&record, self.state()),
            "get_messages" | "get_entries" => {
                let mut record = record;
                record["type"] = json!("get_messages");
                self.reply(
                    &record,
                    json!({"messages":self.history,"compactions":self.compactions,"forkMessages":self.fork_messages()}),
                );
            }
            "get_available_models" => {
                if self
                    .pending
                    .values()
                    .any(|pending| pending.request["type"] == "models")
                {
                    self.reply(&record, json!({"models":self.models}));
                    return Ok(());
                }
                self.raw_models.clear();
                self.model_cursors.clear();
                self.request(
                    "model/list",
                    json!({"limit":100}),
                    json!({"type":"models","id":record["id"]}),
                );
            }
            "get_available_thinking_levels" => self.reply(&record, json!({"levels":self.levels()})),
            "get_session_stats" => self.reply(&record, json!({"contextUsage":self.usage})),
            "get_commands" => self.request(
                "skills/list",
                json!({"cwds":[self.start.workspace],"forceReload":true}),
                json!({"type":"skills","id":record["id"]}),
            ),
            "set_session_name" => self.reply(&record, json!({})),
            "set_service_tier" => {
                if self.running
                    || !service_tiers(&self.model)
                        .iter()
                        .any(|tier| tier["id"] == record["tier"])
                {
                    self.reject(
                        &record,
                        "Choose an advertised Codex service tier while idle.",
                    );
                } else {
                    self.service_tier = record["tier"].as_str().map(str::to_owned);
                    self.reply(&record, json!({"serviceTier":self.service_tier}));
                }
            }
            "native_command" if self.native_extensions => {
                let text = record["arguments"].as_str().unwrap_or("").trim();
                if self.running
                    && record["name"] == "goal"
                    && !matches!(text, "" | "pause" | "clear")
                {
                    self.reject(
                        &record,
                        "Wait for Codex to finish before running this command.",
                    );
                    return Ok(());
                }
                match record["name"].as_str() {
                    Some("feedback") => self.request(
                        "feedback/upload",
                        json!({"threadId":self.session,"classification":"bug","includeLogs":true,
                            "reason":if text.is_empty() {Value::Null} else {json!(text)}}),
                        json!({"type":"native_feedback","id":record["id"]}),
                    ),
                    Some("goal") => {
                        if matches!(text, "pause" | "clear") {
                            self.cancel_goal_activation();
                        }
                        let mut params = json!({"threadId":self.session});
                        let method = match text {
                            "" => "thread/goal/get",
                            "clear" => "thread/goal/clear",
                            "pause" | "resume" => {
                                params["status"] =
                                    json!(if text == "pause" { "paused" } else { "active" });
                                params["origin"] = json!("user");
                                "thread/goal/set"
                            }
                            objective => {
                                params["objective"] = json!(objective);
                                params["status"] = json!("active");
                                params["origin"] = json!("user");
                                "thread/goal/set"
                            }
                        };
                        if params["status"] == "active" {
                            // Codex starts a goal turn itself. Apply the current policy/model
                            // first, natively, so it cannot reuse a stale, broader turn policy.
                            let (approval, reviewer, _, sandbox) = policy(self.access());
                            let mut settings = json!({"threadId":self.session,"cwd":self.start.workspace,
                                "approvalPolicy":approval,"approvalsReviewer":reviewer,"sandboxPolicy":sandbox});
                            if let Some(model) = self.model["id"].as_str() {
                                settings["model"] = json!(model);
                            }
                            if !self.effort.is_empty() {
                                settings["effort"] = json!(self.effort);
                            }
                            self.request(
                                "thread/settings/update",
                                settings,
                                json!({"type":"goal_settings","id":record["id"],"params":params}),
                            );
                        } else {
                            self.request(
                                method,
                                params,
                                json!({"type":"native_goal","id":record["id"]}),
                            );
                        }
                    }
                    _ => self.reject(&record, "Unknown native Codex command."),
                }
            }
            "fork" if self.native_extensions => {
                let entry = record["entryId"].as_str().unwrap_or("");
                if self.history_mode != "paginated" {
                    self.reject(&record, "Codex cannot rewind a legacy-history thread. Start a new paginated Codex thread.");
                } else if self.running
                    || !self
                        .fork_messages()
                        .iter()
                        .any(|point| point["entryId"] == entry)
                {
                    self.reject(&record, "Choose a Codex user turn while idle.");
                } else {
                    // Rewind replaces conversation history only, never files on disk.
                    self.request(
                        "thread/revert",
                        json!({"threadId":self.session,"beforeTurnId":entry}),
                        record,
                    );
                }
            }
            "compact" => {
                if self.running {
                    self.reject(
                        &record,
                        "Wait for Codex to finish before compacting context.",
                    );
                } else {
                    if record["customInstructions"]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty())
                    {
                        self.events.push(json!({"type":"extension_ui_request","method":"notify",
                            "message":"Codex native compaction does not accept custom instructions. The instructions were ignored."}));
                    }
                    self.running = true;
                    self.cancelling = false;
                    self.interrupt_sent = false;
                    self.turn = None;
                    self.compact_request = Some(record.clone());
                    self.begin_compaction(true);
                    // Ported from CodexAdapterV2.compactThread. The generated
                    // ThreadCompactStartParams has only threadId, never input.
                    self.request(
                        "thread/compact/start",
                        json!({"threadId":self.session}),
                        record,
                    );
                }
            }
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
                self.cancel_goal_activation();
                if self.native_extensions && self.goal["status"] == "active" {
                    self.request(
                        "thread/goal/set",
                        json!({"threadId":self.session,"status":"paused","origin":"user"}),
                        json!({"type":"goal_stop"}),
                    );
                }
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
            "set_prompt_suggestions" => {
                self.prompt_suggestions = record["enabled"] == true;
                self.reply(&record, json!({}));
            }
            "prompt" if self.running => {
                // KSG uses turn/steer with expectedTurnId. Follow-ups retain FIFO ordering.
                if record["streamingBehavior"] == "steer"
                    && self.compact_request.is_none()
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
                    self.service_tier = None;
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
            "set_option" => match record["optionId"].as_str() {
                Some("reasoningEffort") => {
                    let mut synth = record.clone();
                    synth["type"] = json!("set_thinking_level");
                    synth["level"] = record["value"].clone();
                    return self.command(synth);
                }
                Some("serviceTier") => {
                    let mut synth = record.clone();
                    synth["type"] = json!("set_service_tier");
                    synth["tier"] = record["value"].clone();
                    return self.command(synth);
                }
                _ => self.reply(&record, json!({})),
            },
            "consume_reset_credit" => {
                let idempotency_key = record["idempotencyKey"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                self.request(
                    "account/rateLimitResetCredit/consume",
                    json!({"idempotencyKey": idempotency_key}),
                    record,
                );
            }
            _ => self.reject(&record, format!("Codex does not support {kind} in Shina.")),
        }
        Ok(())
    }

    fn prompt(&mut self, record: Value) -> Result<(), String> {
        if let Some(tier) = record["serviceTier"].as_str() {
            if !service_tiers(&self.model)
                .iter()
                .any(|option| option["id"] == tier)
            {
                return Err("The chosen Codex service tier is no longer advertised.".into());
            }
            self.service_tier = Some(tier.into());
        }
        let input = codex_input(&record)?;
        let (approval, reviewer, _, sandbox) = policy(self.access());
        let mut params = json!({"threadId":self.session,"cwd":self.start.workspace,
            "input":input,"approvalPolicy":approval,"approvalsReviewer":reviewer,
            "sandboxPolicy":sandbox,"summary":"detailed"});
        if let Some(model) = self.model["id"].as_str() {
            params["model"] = json!(model);
        }
        if !self.effort.is_empty() {
            params["effort"] = json!(self.effort);
        }
        if let Some(tier) = &self.service_tier {
            params["serviceTierForTurn"] = json!(tier);
        }
        self.running = true;
        self.cancelling = false;
        self.interrupt_sent = false;
        self.turn = None;
        self.turn_serial += 1;
        self.active_run_id = Some(
            record["runId"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("codex-turn-{}", self.turn_serial)),
        );
        self.settled_run_id = None;
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
                if !self.running {
                    self.events.push(json!({"type":"agent_start"}));
                }
                self.retry_attempt = 0;
                self.retrying = false;
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
            "turn/plan/updated" => {
                let turn = params["turnId"]
                    .as_str()
                    .or(self.turn.as_deref())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        self.history
                            .iter()
                            .filter(|m| m["role"] == "user")
                            .count()
                            .to_string()
                    });
                let id = params["planId"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("codex-plan-{turn}"));
                self.plan(params, &id, true);
            }
            "thread/settings/updated" => {
                if let Some(cwd) = params["threadSettings"]["cwd"].as_str() {
                    let cwd = PathBuf::from(cwd);
                    if !cwd.is_absolute() {
                        return Err("Codex reported a non-absolute working directory.".into());
                    }
                    if cwd != self.start.workspace {
                        self.start.workspace = cwd;
                        self.skill_commands.clear();
                        self.skills_loaded = false;
                        self.reply(
                            &json!({"type":"get_commands","id":"commands"}),
                            json!({"commands":self.commands()}),
                        );
                        self.request(
                            "skills/list",
                            json!({"cwds":[self.start.workspace],"forceReload":true}),
                            json!({"type":"skills","id":"commands"}),
                        );
                    }
                }
            }
            "account/rateLimits/updated" => {
                self.usage_limits =
                    crate::usage_limits::codex(&params["rateLimits"], &self.usage_limits);
                self.events.push(json!({"type":"harness_update","field":"usageLimits","value":self.usage_limits}));
                let update = &params["rateLimits"];
                if update["limitId"].as_str().is_none_or(|id| id == "codex") {
                    if !self.usage_limits_raw.is_object() {
                        self.usage_limits_raw = json!({"snapshot": {}});
                    }
                    self.usage_limits_raw["checkedAtMs"] = json!(crate::lifecycle::now());
                    if let Some(target) = self.usage_limits_raw["snapshot"].as_object_mut()
                        && let Some(src) = update.as_object()
                    {
                        for (k, v) in src {
                            target.insert(k.clone(), v.clone());
                        }
                    } else {
                        self.usage_limits_raw["snapshot"] = update.clone();
                    }
                    self.events.push(json!({"type":"harness_update","field":"usageLimitsRaw","value":self.usage_limits_raw}));
                }
            }
            "thread/goal/updated" => {
                self.goal = params["goal"].clone();
                self.events
                    .push(json!({"type":"harness_update","field":"goal","value":self.goal}));
            }
            "thread/goal/cleared" => {
                self.goal = Value::Null;
                self.events
                    .push(json!({"type":"harness_update","field":"goal","value":null}));
            }
            "thread/tokenUsage/updated" => {
                if self.compacting {
                    return Ok(());
                }
                // KSG uses last, not total. total is cumulative across turns.
                let usage = &params["tokenUsage"];
                let window = usage["modelContextWindow"].as_u64().filter(|n| *n > 0);
                let tokens = usage["last"]["totalTokens"].as_u64();
                self.usage = json!({"tokens":tokens,"contextWindow":window});
                if let Some(window) = window {
                    self.model["contextWindow"] = json!(window);
                }
                self.events.push(json!({"type":"context_usage",
                    "data":{"contextUsage":self.usage}}));
            }
            // Deprecated app-server notification. New versions send a
            // contextCompaction item as well; do not show a second divider.
            "thread/compacted" => {
                let turn = params["turnId"].as_str().unwrap_or("");
                if self.compacting || !self.compacted_turns.contains(turn) {
                    self.begin_compaction(true);
                    self.end_compaction(json!({}), true);
                    self.compacted_turns.insert(turn.into());
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
                let failed = turn["status"] == "failed";
                let aborted = self.cancelling || turn["status"] == "interrupted";
                if self.retrying {
                    let error = self.redact(if aborted {
                        "Provider retry stopped"
                    } else {
                        turn["error"]["message"]
                            .as_str()
                            .unwrap_or("Codex turn failed.")
                    });
                    self.events.push(
                        json!({"type":"auto_retry_end","success":!failed && !aborted,
                        "finalError":error}),
                    );
                    self.retrying = false;
                }
                for item in turn["items"].as_array().into_iter().flatten() {
                    if item["type"] == "contextCompaction" {
                        self.replay_compaction(item, turn, true);
                    } else {
                        self.item(item, true, true);
                    }
                }
                let was_compacting = self.compacting;
                let turn_id = turn["id"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| self.turn.clone());
                if self.compacting {
                    self.end_compaction(self.compaction_outcome(turn), true);
                }
                if let Some(request) = self.compact_request.take() {
                    if failed {
                        self.reject(
                            &request,
                            self.compaction_outcome(turn)["errorMessage"]
                                .as_str()
                                .unwrap_or("Codex compaction failed.")
                                .to_owned(),
                        );
                    } else {
                        self.reply(&request, json!({"aborted":aborted}));
                    }
                }
                self.finish_history_turn(turn);
                let mut saved = turn.clone();
                saved["items"] = json!(std::mem::take(&mut self.turn_items));
                self.turns.push(saved);
                if failed {
                    self.events.push(json!({"type":"message_end","message":{"role":"assistant","stopReason":"error",
                        "errorMessage":auth_guidance(&self.redact(turn["error"]["message"].as_str().unwrap_or("Codex turn failed.")))}}));
                }
                self.running = false;
                self.turn = None;
                self.cancelling = false;
                if !aborted && !failed && !was_compacting && self.queued.is_empty() {
                    let run_id = self.active_run_id.take().or_else(|| turn_id.clone());
                    self.settled_run_id = run_id.clone();
                    if self.prompt_suggestions
                        && let (Some(run_id), Some(turn_id), Some(session_id)) =
                            (run_id, turn_id, self.session.clone())
                    {
                        self.request(
                            "thread/sample",
                            json!({
                                "threadId": session_id,
                                "expectedTurnId": turn_id,
                                "prompt": PROMPT_SUGGESTION_PROMPT,
                                "effort": "low",
                            }),
                            json!({"type": "prompt_suggestion", "runId": run_id}),
                        );
                    }
                } else {
                    self.active_run_id = None;
                    self.settled_run_id = None;
                }
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
            "error" if params["willRetry"] == true => {
                self.retry_attempt = params["attempt"].as_u64().unwrap_or(self.retry_attempt + 1);
                self.retrying = true;
                self.events.push(json!({"type":"auto_retry_start","attempt":self.retry_attempt,
                    "maxAttempts":params["maxAttempts"],"errorMessage":self.redact(params["error"]["message"].as_str().unwrap_or("Codex provider error."))}));
            }
            "error" if params["willRetry"] != true && self.compacting => {
                self.compaction_error = Some(
                    self.redact(
                        params["error"]["message"]
                            .as_str()
                            .unwrap_or("Codex compaction failed."),
                    ),
                );
            }
            "error" if params["willRetry"] != true => {
                if self.retrying {
                    self.events.push(json!({"type":"auto_retry_end","success":false,"finalError":
                        self.redact(params["error"]["message"].as_str().unwrap_or("Codex failed."))}));
                    self.retrying = false;
                }
                return Err(auth_guidance(
                    &self.redact(
                        params["error"]["message"]
                            .as_str()
                            .unwrap_or("Codex failed."),
                    ),
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
        if kind == "collabAgentToolCall" && item["tool"] == "spawnAgent" {
            for child in item["receiverThreadIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                self.child_parents
                    .entry(child.into())
                    .or_insert_with(|| id.into());
            }
        } else if kind == "subAgentActivity"
            && let Some(child) = item["agentThreadId"].as_str()
        {
            self.child_parents
                .entry(child.into())
                .or_insert_with(|| id.into());
        }
        match kind {
            "contextCompaction" => {
                // Some versions send the deprecated notification first. It already
                // completed this divider; a late item must not erase fresher usage.
                let legacy_completed = done
                    && !self.compacting
                    && self
                        .turn
                        .as_ref()
                        .is_some_and(|turn| self.compacted_turns.contains(turn));
                if !legacy_completed {
                    self.begin_compaction(emit);
                    if done {
                        let outcome = self
                            .compaction_error
                            .as_ref()
                            .map_or_else(|| json!({}), |error| json!({"errorMessage":error}));
                        self.end_compaction(outcome, emit);
                    }
                }
            }
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
            "plan" | "todoList" if done => self.plan(item, &format!("codex-plan-{id}"), emit),
            "userMessage" | "agentMessage" | "reasoning" | "plan" | "todoList" => {}
            _ => {
                let name = match kind {
                    "commandExecution" => "command".into(),
                    "fileChange" => "patch".into(),
                    "collabAgentToolCall"
                    | "subAgentActivity"
                    | "collabAgentSpawn"
                    | "collabAgentInteraction" => "subagent".into(),
                    "mcpToolCall" | "dynamicToolCall" | "functionCallOutput" => {
                        let tool = item["tool"]
                            .as_str()
                            .or_else(|| item["name"].as_str())
                            .unwrap_or("Codex tool");
                        item["server"]
                            .as_str()
                            .or_else(|| item["namespace"].as_str())
                            .map_or_else(|| tool.into(), |server| format!("{server}/{tool}"))
                    }
                    _ => kind.to_owned(),
                };
                let metadata = tool_metadata(item);
                if new {
                    self.history.push(json!({"role":"assistant","content":[{"type":"toolCall","id":id,"name":name,"arguments":item,"toolKind":metadata["toolKind"],"changes":metadata["changes"]}]}));
                    if emit {
                        self.events.push(json!({"type":"tool_execution_start","toolCallId":id,"toolName":name,"args":item,"toolKind":metadata["toolKind"],"changes":metadata["changes"]}));
                    }
                }
                if done {
                    let failed = matches!(item["status"].as_str(), Some("failed" | "declined"))
                        || item["exitCode"].as_i64().is_some_and(|code| code != 0)
                        || item["success"] == false
                        || !item["error"].is_null();
                    let mut result = metadata;
                    result["content"] = tool_content(item);
                    if item["aggregatedOutput"].as_str().is_none()
                        && item["output"].as_str().is_none()
                        && let Some(output) = self.text.get(&format!("output:{id}"))
                    {
                        result["content"] = json!([{"type":"text","text":output}]);
                    }
                    result["details"] = json!({"codex":item,"exitCode":item["exitCode"]});
                    if let Some(structured) = item["result"].get("structuredContent") {
                        result["structuredContent"] = structured.clone();
                    }
                    result["nestedCalls"] = item
                        .get("nestedCalls")
                        .or_else(|| item["result"]["structuredContent"].get("nestedCalls"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    if matches!(kind, "collabAgentToolCall" | "subAgentActivity") {
                        result["details"]["results"] = subagent_results(item);
                    }
                    let mut saved = result.clone();
                    saved["role"] = json!("toolResult");
                    saved["toolCallId"] = json!(id);
                    saved["isError"] = json!(failed);
                    self.history.push(saved);
                    if emit {
                        self.events
                            .push(json!({"type":"tool_execution_end","toolCallId":id,
                        "result":result,"isError":failed}));
                    }
                }
            }
        }
        if done {
            if emit {
                self.turn_items.push(item.clone());
            }
            self.completed.insert(id.into());
            self.items.remove(id);
            self.text.remove(&format!("output:{id}"));
        }
    }

    /// Nested notifications retain their native parent; they never settle the root turn.
    fn child_notification(
        &mut self,
        method: &str,
        params: &Value,
        parent: &str,
    ) -> Result<(), String> {
        let thread = params["threadId"].as_str().unwrap_or("");
        match method {
            "item/started" | "item/completed" => {
                if matches!(
                    params["item"]["type"].as_str(),
                    Some(
                        "userMessage"
                            | "agentMessage"
                            | "reasoning"
                            | "contextCompaction"
                            | "plan"
                            | "todoList"
                    )
                ) {
                    return Ok(());
                }
                let mut item = params["item"].clone();
                let Some(id) = item["id"].as_str() else {
                    return Ok(());
                };
                item["id"] = json!(format!("{thread}:{id}"));
                let (history, events, turns) =
                    (self.history.len(), self.events.len(), self.turn_items.len());
                self.item(&item, method == "item/completed", true);
                self.turn_items.truncate(turns);
                for message in &mut self.history[history..] {
                    for block in message["content"].as_array_mut().into_iter().flatten() {
                        if block["type"] == "toolCall" {
                            block["parentToolCallId"] = json!(parent);
                        }
                    }
                }
                for event in &mut self.events[events..] {
                    event["parentToolCallId"] = json!(parent);
                }
            }
            "item/commandExecution/outputDelta" => {
                let mut params = params.clone();
                params["itemId"] = json!(format!(
                    "{thread}:{}",
                    params["itemId"].as_str().unwrap_or("")
                ));
                self.notification(method, &params)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn plan(&mut self, source: &Value, id: &str, emit: bool) {
        let mut source = source.clone();
        source["_shinaPlanId"] = json!(id);
        crate::work::save_plan(&mut self.history, id, &source);
        if emit {
            self.events
                .push(json!({"type":"harness_update","field":"plan","value":source}));
        }
    }

    fn replay(&mut self, turn: &Value) {
        for item in turn["items"].as_array().into_iter().flatten() {
            if item["type"] == "contextCompaction" {
                self.replay_compaction(item, turn, false);
            } else {
                self.item(item, true, false);
            }
        }
        self.finish_history_turn(turn);
    }

    fn replay_compaction(&mut self, item: &Value, turn: &Value, emit: bool) {
        let Some(id) = item["id"].as_str() else {
            return;
        };
        if self.completed.contains(id) {
            return;
        }
        if emit
            && !self.compacting
            && turn["id"]
                .as_str()
                .is_some_and(|turn| self.compacted_turns.contains(turn))
        {
            self.completed.insert(id.into());
            return;
        }
        self.begin_compaction(emit);
        if turn["status"] == "inProgress" {
            self.items.insert(id.into(), item.clone());
        } else {
            self.completed.insert(id.into());
            self.end_compaction(self.compaction_outcome(turn), emit);
            if let Some(turn) = turn["id"].as_str() {
                self.compacted_turns.insert(turn.into());
            }
        }
    }

    fn begin_compaction(&mut self, emit: bool) {
        if self.compacting {
            return;
        }
        self.compacting = true;
        self.compaction_error = None;
        self.invalidate_usage();
        if self.compactions.len() >= 32 {
            self.compactions.remove(0);
        }
        self.compactions
            .push(json!({"running":true,"historyIndex":self.history.len()}));
        if emit {
            self.events.push(json!({"type":"compaction_start"}));
        }
    }

    fn end_compaction(&mut self, mut outcome: Value, emit: bool) {
        self.compacting = false;
        self.invalidate_usage();
        outcome["type"] = json!("compaction_end");
        if self
            .compactions
            .last()
            .is_some_and(|row| row["running"] == true)
        {
            outcome["historyIndex"] = self.compactions.last().unwrap()["historyIndex"].clone();
            *self.compactions.last_mut().unwrap() = outcome.clone();
        } else {
            return;
        }
        if let Some(turn) = &self.turn {
            self.compacted_turns.insert(turn.clone());
        }
        if emit {
            self.events.push(outcome);
        }
    }

    fn invalidate_usage(&mut self) {
        self.usage = json!({"tokens":null,"percent":null,
            "contextWindow":self.usage.get("contextWindow").cloned().unwrap_or(Value::Null)});
    }

    fn compaction_outcome(&self, turn: &Value) -> Value {
        if let Some(error) = &self.compaction_error {
            return json!({"errorMessage":error});
        }
        match turn["status"].as_str() {
            Some("failed") => json!({"errorMessage":self.redact(
                turn["error"]["message"].as_str().unwrap_or("Codex compaction failed.")
            )}),
            Some("interrupted") => json!({"aborted":true}),
            _ if self.cancelling => json!({"aborted":true}),
            _ => json!({}),
        }
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
        let has_message = self.active_message.is_some()
            || turn["items"].as_array().into_iter().flatten().any(|item| {
                !matches!(
                    item["type"].as_str(),
                    Some("userMessage" | "contextCompaction")
                )
            });
        if has_message
            && let Some(message) = self
                .history
                .iter_mut()
                .rev()
                .find(|m| m["role"] == "assistant")
        {
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
    let capabilities = crate::descriptors::map_codex_model_capabilities(model, None);
    Some(
        json!({"provider":"codex","id":id,"name":model["displayName"].as_str().unwrap_or(id),
        "thinkingLevels":levels,"input":model.get("inputModalities").cloned().unwrap_or_else(||json!(["text","image"])),
        "contextWindow":model["contextWindow"],"serviceTiers":service_tiers(model),
        "defaultServiceTier":model["defaultServiceTier"],
        "isDefault":model["isDefault"] == true,
        "capabilities":capabilities}),
    )
}

pub(crate) fn service_tiers(model: &Value) -> Vec<Value> {
    let mut tiers = model["serviceTiers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if tiers.is_empty() {
        tiers.extend(model["additionalSpeedTiers"].as_array().into_iter().flatten()
            .filter_map(|tier| tier.as_str())
            .map(|id| json!({"id":id,"name":if id == "fast" {"Fast"} else {id},"description":""})));
    }
    tiers.retain(|tier| tier["id"].as_str().is_some_and(|id| !id.is_empty()));
    if !tiers.is_empty() && !tiers.iter().any(|tier| tier["id"] == "default") {
        tiers.insert(
            0,
            json!({"id":"default","name":"Standard","description":"Standard speed"}),
        );
    }
    let default = model["defaultServiceTier"].as_str().unwrap_or("default");
    for tier in &mut tiers {
        tier["isDefault"] = json!(tier["id"] == default);
    }
    tiers
}

/// These optional methods and fields were checked in the 0.167.2 generated schema.
/// Unknown/older versions retain the existing compact-only contract.
fn supports_extensions(initialize: &Value) -> bool {
    initialize["userAgent"]
        .as_str()
        .into_iter()
        .flat_map(|ua| ua.split_whitespace())
        .filter_map(|part| part.split_once('/').map(|(_, version)| version))
        .filter_map(|version| {
            let parts: Vec<_> = version.split('.').take(3).map(str::parse::<u32>).collect();
            if let [Ok(major), Ok(minor), Ok(patch)] = parts.as_slice() {
                Some((*major, *minor, *patch) >= (0, 167, 2))
            } else {
                None
            }
        })
        .any(|supported| supported)
}

pub(crate) fn auth_guidance(message: &str) -> String {
    if crate::usage_limits::authentication_error(message) {
        format!("{message}\nRun `codex login` outside Shina, then retry.")
    } else {
        message.into()
    }
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
    command: Command,
    start: Start,
    input: Receiver<Value>,
    events: &SyncSender<Event>,
    wake: &Waker,
    stopped: &AtomicBool,
    owner: &SessionOwner,
) -> Result<(), String> {
    run_mode(command, start, input, events, wake, stopped, owner, None)
}

#[allow(clippy::too_many_arguments)]
fn run_mode(
    mut command: Command,
    start: Start,
    input: Receiver<Value>,
    events: &SyncSender<Event>,
    wake: &Waker,
    stopped: &AtomicBool,
    owner: &SessionOwner,
    probe: Option<bool>,
) -> Result<(), String> {
    if probe.is_some() && !start.workspace.exists() {
        std::fs::create_dir_all(&start.workspace)
            .map_err(|error| format!("Could not create the discovery directory: {error}"))?;
    }
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
    client.probe = probe;
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
            let now = Instant::now();
            client.pending.retain(|_, p| {
                !(matches!(
                    p.request["type"].as_str(),
                    Some("prompt_suggestion" | "rateLimits" | "account")
                ) && now >= p.deadline)
            });
            if client.pending.values().any(|p| now >= p.deadline) {
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
            if probe.is_some() && client.ready && client.skills_loaded {
                return Ok(());
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

/// Ported from KSG Code's CodexAdapterV2 item contracts, before output is flattened.
fn tool_metadata(item: &Value) -> Value {
    use crate::work::{FileChange, ToolKind, result_metadata};
    let kind = match item["type"].as_str().unwrap_or("") {
        "commandExecution" => ToolKind::Command,
        "fileChange" => ToolKind::FileChange,
        "fileSearch" => ToolKind::FileSearch,
        "webSearch" | "webFetch" => ToolKind::WebSearch,
        "readFile" | "imageView" => ToolKind::Read,
        "listDirectory" => ToolKind::List,
        "collabAgentToolCall"
        | "collabAgentSpawn"
        | "collabAgentInteraction"
        | "subAgentActivity" => ToolKind::Subagent,
        "todoList" | "plan" => ToolKind::Plan,
        _ => ToolKind::Other,
    };
    let changes = item["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|change| {
            let path = change["path"].as_str()?;
            let operation = change["kind"]["type"]
                .as_str()
                .or_else(|| change["kind"].as_str())
                .unwrap_or("update");
            let mut file = FileChange::new(path, operation, change["diff"].as_str().unwrap_or(""));
            if let Some(moved) = change["kind"]["move_path"]
                .as_str()
                .or_else(|| change["kind"]["movePath"].as_str())
            {
                file.old_path = Some(file.path);
                file.path = moved.into();
                file.operation = "move".into();
            }
            Some(file)
        })
        .collect();
    let mut metadata = result_metadata(kind, changes, item);
    metadata["input"] = item.clone();
    metadata
}

fn subagent_results(item: &Value) -> Value {
    let mut agents = item["agentsStates"].as_object().map(|states| states.iter().map(|(id, state)| {
        json!({"agent":id,"task":item["prompt"].as_str().unwrap_or("Codex subagent"),
            "model":item["model"],"finished":!matches!(state["status"].as_str(),Some("pendingInit"|"running")),
            "stopReason":if state["status"] == "errored" {"error"} else if state["status"] == "shutdown" {"aborted"} else {"stop"},
            "exitCode":if state["status"] == "errored" {1} else {0},
            "messages":[{"role":"assistant","content":[{"type":"text","text":state["message"].as_str().unwrap_or("")}]}]})
    }).collect::<Vec<_>>()).unwrap_or_default();
    if agents.is_empty()
        && let Some(id) = item["agentThreadId"].as_str()
    {
        agents.push(
            json!({"agent":id,"task":item["agentPath"],"model":item["model"],
            "finished":item["kind"] == "completed" || item["kind"] == "interrupted",
            "stopReason":if item["kind"] == "interrupted" {"aborted"} else {"stop"},"exitCode":0}),
        );
    }
    json!(agents)
}

fn tool_content(item: &Value) -> Value {
    if let Some(content) = item["result"]["content"].as_array() {
        return json!(content);
    }
    let content = item["contentItems"]
        .as_array()
        .or_else(|| item["output"].as_array());
    if let Some(content) = content {
        return json!(
            content
                .iter()
                .filter_map(|part| {
                    if let Some(text) = part["text"].as_str() {
                        Some(json!({"type":"text","text":text}))
                    } else {
                        let url = part["imageUrl"]
                            .as_str()
                            .or_else(|| part["image_url"].as_str())?;
                        let (mime, data) = url.strip_prefix("data:")?.split_once(";base64,")?;
                        Some(json!({"type":"image","mimeType":mime,"data":data}))
                    }
                })
                .collect::<Vec<_>>()
        );
    }
    if item["type"] == "imageView"
        && let Some(path) = item["path"].as_str()
        && let Ok(image) = crate::attachments::attach_file(std::path::Path::new(path))
        && image.is_image()
    {
        return json!([{"type":"image","shinaAttachment":image.to_value()}]);
    }
    let output = item["aggregatedOutput"]
        .as_str()
        .or_else(|| item["output"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| item.to_string());
    json!([{"type":"text","text":output}])
}
pub const TURN_CAPABILITIES: crate::model::TurnCapabilities =
    crate::model::TurnCapabilities::new(true, true);
