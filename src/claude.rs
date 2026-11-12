//! Claude Code stream-json stdio, ported from Claude Agent SDK 0.3.321
//! (ProcessTransport, Query and sdk.d.ts), verified against Claude Code 2.1.322.
//! KSG references: ClaudeAdapterV2.ts, ClaudeProvider.ts and Drivers/ClaudeSkills.ts.
//! The installed CLI calls the SDK's `default` permission mode `manual`.
//! No Node sidecar, credential access, configuration writes or inference retries.
use crate::{
    backend::{self, Event, Pi, SessionOwner, SessionSource},
    harness::Start,
};
use rune::Waker;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub const MISSING: &str = "Claude Code is unavailable. Shina checks PATH and ~/.local/bin or ~/.claude/local; set SHINA_CLAUDE_CLI to override.";
pub const AUTH_ERROR: &str =
    "Claude authentication failed. Run `claude auth login` outside Shina, then retry.";

pub fn process() -> Result<Command, String> {
    let candidates = if let Some(path) = std::env::var_os("SHINA_CLAUDE_CLI") {
        vec![PathBuf::from(path)]
    } else {
        let mut paths = vec![PathBuf::from("claude")];
        if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
            let home = PathBuf::from(home);
            for relative in if cfg!(windows) {
                vec![
                    ".local/bin/claude.exe",
                    ".claude/local/claude.exe",
                    "AppData/Roaming/npm/claude.cmd",
                ]
            } else {
                vec![
                    ".local/bin/claude",
                    ".claude/local/claude",
                    ".npm-global/bin/claude",
                ]
            } {
                paths.push(home.join(relative));
            }
        }
        paths
    };
    resolve_process(candidates)
}

fn resolve_process(candidates: Vec<PathBuf>) -> Result<Command, String> {
    for path in candidates {
        if let Ok((executable, script)) = crate::harness::executable(&path.to_string_lossy()) {
            let mut command = Command::new(executable);
            if let Some(script) = script {
                command.arg(script);
            }
            command
                .env_remove(crate::mcp::URL_ENV)
                .env_remove(crate::mcp::TOKEN_ENV)
                .env_remove("SHINA_RUNTIME_MODE")
                .env_remove("SHINA_PI_BRIDGE_PATH")
                // Causes known startup failures to arrive as structured result messages.
                .env("CLAUDE_CODE_STARTUP_FAILURE_RESULTS", "1");
            return Ok(command);
        }
    }
    Err(MISSING.into())
}

pub fn start(start: Start, wake: Waker) -> Result<Pi, String> {
    start_process(process()?, start, wake)
}

pub fn discover(workspace: PathBuf, wake: Waker) -> Result<Pi, String> {
    let command = process()?;
    let start = Start::discovery("claude", workspace);
    Ok(Pi::worker(
        wake,
        start.owner.clone(),
        None,
        move |input, events, wake, stopped, owner| {
            run_mode(command, start, input, events, wake, stopped, owner, true)
        },
    ))
}

fn start_process(command: Command, start: Start, wake: Waker) -> Result<Pi, String> {
    if matches!(start.source, SessionSource::Fork(_)) {
        return Err("Claude conversation forks and rewinds are not implemented in Shina.".into());
    }
    if start.link.is_none() {
        return Err("Claude requires Shina's thread-scoped MCP endpoint.".into());
    }
    if let SessionSource::Resume(id) = &start.source {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid Claude session id. Cannot resume.")?;
    }
    let key = match &start.source {
        SessionSource::Resume(id) => Some(PathBuf::from(format!("claude:{id}"))),
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

fn permission_mode(access: &str) -> &'static str {
    match access {
        "full-access" => "bypassPermissions",
        "auto-accept-edits" => "acceptEdits",
        "read-only" => "dontAsk",
        _ => "default",
    }
}

fn read_tool(name: &str) -> bool {
    matches!(
        name,
        "Read" | "Glob" | "Grep" | "LS" | "WebSearch" | "WebFetch" | "TaskOutput"
    )
}

fn edit_tool(name: &str) -> bool {
    matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
}

fn tool_name(name: &str) -> String {
    match name {
        "Read" => "read",
        "Glob" | "Grep" | "WebSearch" => "search",
        "LS" => "ls",
        "Write" => "write",
        "Edit" | "MultiEdit" | "NotebookEdit" => "edit",
        "Bash" => "bash",
        "PowerShell" => "powershell",
        "Task" | "Agent" => "subagent",
        other => other,
    }
    .into()
}

struct Pending {
    original: Value,
    request: Value,
    deadline: Instant,
}

struct Approval {
    request_id: String,
    input: Value,
    name: String,
    tool_id: Value,
    hook: bool,
}

struct Client {
    start: Start,
    config: PathBuf,
    session: String,
    ready: bool,
    running: bool,
    cancelling: bool,
    planning: bool,
    serial: usize,
    pending: HashMap<String, Pending>,
    approvals: HashMap<String, Approval>,
    models: Vec<Value>,
    commands: Vec<Value>,
    model: Value,
    effort: String,
    history: Vec<Value>,
    seen: HashSet<String>,
    tools: HashMap<String, Value>,
    blocks: HashMap<(String, u64), Value>,
    streamed: HashSet<String>,
    last_usage: Value,
    usage_model: String,
    usage: Value,
    queued: VecDeque<Value>,
    /// Sent UUIDs are only recovered when the interrupt receipt proves cancellation.
    sent: HashMap<String, Value>,
    capabilities: HashSet<String>,
    compact: Option<Value>,
    compacting: bool,
    compact_failure: Option<String>,
    compact_reported: bool,
    compactions: Vec<Value>,
    wire: Vec<Value>,
    events: Vec<Value>,
    usage_limits: Value,
}

impl Client {
    fn new(start: Start) -> Self {
        let session = match &start.source {
            SessionSource::Resume(id) => id.clone(),
            _ => uuid::Uuid::new_v4().to_string(),
        };
        Self {
            start,
            config: config_directory(),
            session,
            ready: false,
            running: false,
            cancelling: false,
            planning: false,
            serial: 0,
            pending: HashMap::new(),
            approvals: HashMap::new(),
            models: vec![],
            commands: vec![],
            model: Value::Null,
            effort: String::new(),
            history: vec![],
            seen: HashSet::new(),
            tools: HashMap::new(),
            blocks: HashMap::new(),
            streamed: HashSet::new(),
            last_usage: Value::Null,
            usage_model: String::new(),
            usage: Value::Null,
            queued: VecDeque::new(),
            sent: HashMap::new(),
            capabilities: HashSet::new(),
            compact: None,
            compacting: false,
            compact_failure: None,
            compact_reported: false,
            compactions: vec![],
            wire: vec![],
            events: vec![],
            usage_limits: Value::Null,
        }
    }

    fn access(&self) -> &str {
        &self.start.link.as_ref().expect("MCP required").access_mode
    }

    fn launch(&self, command: &mut Command) {
        if self.start.link.is_none() {
            command.args([
                "--print",
                "--output-format",
                "stream-json",
                "--input-format",
                "stream-json",
                "--verbose",
                "--permission-prompt-tool=stdio",
                "--no-session-persistence",
            ]);
            command.arg("--session-id").arg(&self.session);
            return;
        }
        let link = self.start.link.as_ref().unwrap();
        command
            .args([
                "--print",
                "--output-format",
                "stream-json",
                "--verbose",
                "--input-format",
                "stream-json",
                "--include-partial-messages",
                "--replay-user-messages",
                "--permission-prompt-tool=stdio",
                "--allow-dangerously-skip-permissions",
                "--permission-mode",
                if self.access() == "approval-required" {
                    "manual"
                } else {
                    permission_mode(self.access())
                },
                "--append-system-prompt",
            ])
            .arg(format!(
                "{}\n\n{}",
                crate::mcp::INSTRUCTIONS,
                self.start.instructions.as_deref().unwrap_or("")
            ))
            // Keep captain and MCP instructions fresh on resume, not a recorded old prompt.
            .args(["--system-prompt-snapshot", "off"])
            // Other local users can read argv, so the token goes through the environment.
            // Claude Code expands `${NAME}` in MCP headers (checked against 2.1.322).
            .env(crate::mcp::TOKEN_ENV, &link.token)
            .arg("--mcp-config")
            .arg(
                json!({"mcpServers":{"shina":{
                    "type":"http","url":link.url,
                    "headers":{"Authorization":format!("Bearer ${{{}}}", crate::mcp::TOKEN_ENV)}
                }}})
                .to_string(),
            );
        match &self.start.source {
            SessionSource::Resume(id) => {
                command.arg("--resume").arg(id);
            }
            _ => {
                command.arg("--session-id").arg(&self.session);
            }
        }
    }

    fn request(&mut self, request: Value, original: Value) {
        self.serial += 1;
        let id = format!("shina-claude-{}", self.serial);
        self.pending.insert(
            id.clone(),
            Pending {
                original,
                request: request.clone(),
                deadline: Instant::now() + Duration::from_secs(30),
            },
        );
        self.wire
            .push(json!({"type":"control_request","request_id":id,"request":request}));
    }

    fn initialize(&mut self) {
        self.request(json!({"subtype":"initialize",
            "forwardSubagentText":true,"forwardNestedSubagentToolCalls":true,
            "hooks":{"PreToolUse":[{"hookCallbackIds":["shina-policy"],"onFailure":"block","timeout":660}]}
        }), json!({"type":"initialize"}));
    }

    fn state(&self) -> Value {
        json!({"sessionFile":self.session,"isStreaming":self.running,
            "turnCapabilities":crate::model::TurnCapabilities::new(self.capabilities.contains("interrupt_cancel_queued_v1"), false),
            "isCompacting":self.compacting,"model":self.model,"thinkingLevel":self.effort})
    }

    fn reply(&mut self, record: &Value, data: Value) {
        self.events.push(json!({"type":"response","id":record["id"],
            "command":record["type"],"success":true,"data":data}));
    }

    fn reject(&mut self, record: &Value, error: impl Into<String>) {
        self.events.push(json!({"type":"response","id":record["id"],
            "command":record["type"],"success":false,"error":error.into()}));
    }

    fn update(&mut self, field: &str, value: Value) {
        self.events
            .push(json!({"type":"harness_update","field":field,"value":value}));
    }

    fn redact(&self, message: &str) -> String {
        crate::update::redact(&crate::projects::redact(
            &self.start.link.as_ref().map_or_else(
                || message.into(),
                |link| message.replace(&link.token, "[redacted]"),
            ),
        ))
    }

    fn levels(&self) -> Value {
        self.model
            .get("thinkingLevels")
            .cloned()
            .unwrap_or_else(|| json!([]))
    }

    fn initialized(&mut self, data: &Value) -> Result<(), String> {
        // A failed blocking hook must never degrade to execution under user allow rules.
        if data["hook_on_failure_supported"] != true || data["hooks_applied"] == false {
            return Err("Claude cannot enforce Shina's blocking tool policy. Update Claude Code outside Shina.".into());
        }
        self.planning = data["current_permission_mode"] == "plan";
        self.models =
            data["models"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|model| {
                    let id = model["value"].as_str()?;
                    let manifest = crate::descriptors::claude_manifest_model_entry(id);
                    let raw_levels: Vec<String> = model
                        .get("supportedEffortLevels")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                    let capabilities =
                        manifest
                            .as_ref()
                            .map(|m| m.capabilities.clone())
                            .unwrap_or_else(|| {
                                if raw_levels.is_empty() {
                                    Default::default()
                                } else {
                                    crate::descriptors::ModelCapabilities {
                                        option_descriptors: vec![
                            crate::descriptors::ProviderOptionDescriptor::Select {
                                id: "effort".into(),
                                label: "Reasoning".into(),
                                description: None,
                                options: std::iter::once(crate::descriptors::ProviderOptionChoice {
                                    id: "default".into(),
                                    label: "Default".into(),
                                    description: None,
                                    is_default: true,
                                })
                                .chain(raw_levels.iter().map(|lvl| {
                                    crate::descriptors::ProviderOptionChoice {
                                        id: lvl.clone(),
                                        label: lvl.clone(),
                                        description: None,
                                        is_default: false,
                                    }
                                }))
                                .collect(),
                                current_value: None,
                                prompt_injected_values: vec![],
                                role: Some(crate::descriptors::ProviderOptionRole::Reasoning),
                            },
                        ],
                                    }
                                }
                            });
                    let thinking_levels: Vec<String> = capabilities
                        .option_descriptors
                        .iter()
                        .find(|d| d.id() == "effort")
                        .map(|d| {
                            crate::descriptors::selectable_provider_option_choices(d)
                                .into_iter()
                                .map(|c| c.id)
                                .filter(|id| id != "default")
                                .collect()
                        })
                        .unwrap_or(raw_levels);
                    Some(json!({"provider":"claude","id":id,
                "name":model["displayName"].as_str().unwrap_or(id),
                "shortName":manifest.as_ref().and_then(|m| m.short_name.clone()),
                "aliases":manifest.as_ref().map(|m| m.aliases.clone()).unwrap_or_default(),
                "isLegacy":manifest.as_ref().is_some_and(|m| m.is_legacy),
                "thinkingLevels":thinking_levels,
                "capabilities":capabilities,
                "input":["text","image"]}))
                })
                .collect();
        // Documented CLI aliases supplement the live catalog, without invented effort levels.
        for alias in [
            "default",
            "opus",
            "sonnet",
            "haiku",
            "fable",
            "opusplan",
            "sonnet[1m]",
        ] {
            if !self.models.iter().any(|m| m["id"] == alias) {
                let manifest = crate::descriptors::claude_manifest_model_entry(alias);
                let capabilities = manifest
                    .as_ref()
                    .map(|m| m.capabilities.clone())
                    .unwrap_or_default();
                let thinking_levels: Vec<String> = capabilities
                    .option_descriptors
                    .iter()
                    .find(|d| d.id() == "effort")
                    .map(|d| {
                        crate::descriptors::selectable_provider_option_choices(d)
                            .into_iter()
                            .map(|c| c.id)
                            .filter(|id| id != "default")
                            .collect()
                    })
                    .unwrap_or_default();
                self.models
                    .push(json!({"provider":"claude","id":alias,"name":alias,
                    "shortName":manifest.as_ref().and_then(|m| m.short_name.clone()),
                    "isLegacy":manifest.as_ref().is_some_and(|m| m.is_legacy),
                    "thinkingLevels":thinking_levels,
                    "capabilities":capabilities,
                    "input":["text","image"]}));
            }
        }
        self.model = self
            .models
            .iter()
            .find(|m| m["id"] == "default")
            .cloned()
            .unwrap_or(Value::Null);
        self.set_commands(&data["commands"]);
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
            json!({"messages":self.history,"compactions":self.compactions}),
        );
        self.reply(
            &json!({"type":"get_commands","id":"commands"}),
            json!({"commands":self.commands}),
        );
        self.update("awareness", json!(true));
        Ok(())
    }

    fn set_commands(&mut self, commands: &Value) {
        self.commands = commands.as_array().into_iter().flatten().filter_map(|c| {
            Some(json!({"name":c["name"].as_str()?,"description":c["description"],"source":"Claude"}))
        }).collect();
        self.commands
            .extend(discover_skills(&self.config, &self.start.workspace));
        // The CLI already selected the active account's synced plugins. Its
        // unmarked rows include those skills, without us reading account files.
        for command in commands.as_array().into_iter().flatten() {
            if command["builtin"] == true {
                continue;
            }
            if let Some(name) = command["name"].as_str() {
                let name = format!("skill:{name}");
                if !self.commands.iter().any(|c| c["name"] == name) {
                    self.commands.push(
                        json!({"name":name,"description":command["description"],"source":"skill"}),
                    );
                }
            }
        }
    }

    fn incoming(&mut self, record: Value) -> Result<(), String> {
        if let Some(id) = record["session_id"].as_str()
            && id != self.session
        {
            return Err(
                "Claude returned a different session id. Refusing a replacement session.".into(),
            );
        }
        if record["type"] == "system" && record["permissionMode"] == "plan" {
            self.planning = true;
        }
        if record["parent_tool_use_id"].as_str().is_none() {
            for id in record["user_message_uuids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .chain(record["user_message_uuid"].as_str())
            {
                self.sent.remove(id);
            }
        }
        match record["type"].as_str().unwrap_or("") {
            "control_response" => {
                let response = &record["response"];
                let id = response["request_id"]
                    .as_str()
                    .ok_or("Claude response lacks request_id.")?;
                let Some(pending) = self.pending.remove(id) else {
                    return Ok(());
                };
                if response["subtype"] == "error" {
                    let error =
                        self.redact(response["error"].as_str().unwrap_or("Unknown Claude error"));
                    if pending.request["subtype"] == "initialize" || auth_error(&error) {
                        if auth_error(&error) {
                            return Err(format!("{error}\n{AUTH_ERROR}"));
                        }
                        return Err(format!(
                            "Claude startup failed: {error}. Sign in with the Claude CLI outside Shina, then retry."
                        ));
                    }
                    if pending.original["next"].is_object() {
                        return Err(format!(
                            "Claude could not apply the plan's permission mode: {error}"
                        ));
                    }
                    self.reject(&pending.original, error);
                    return Ok(());
                }
                let data = &response["response"];
                match pending.request["subtype"].as_str().unwrap_or("") {
                    "initialize" => self.initialized(data)?,
                    "set_model" => {
                        self.model = self
                            .models
                            .iter()
                            .find(|m| m["id"] == pending.request["model"])
                            .cloned()
                            .unwrap();
                        self.usage = Value::Null;
                        self.reply(&pending.original, self.model.clone());
                    }
                    "apply_flag_settings" => {
                        self.effort = pending.request["settings"]["effortLevel"]
                            .as_str()
                            .unwrap_or("")
                            .into();
                        self.reply(&pending.original, json!({"thinkingLevel":self.effort}));
                    }
                    "set_permission_mode" => {
                        self.start.link.as_mut().unwrap().access_mode =
                            pending.original["accessMode"]
                                .as_str()
                                .unwrap_or("read-only")
                                .into();
                        self.reply(&pending.original, json!({}));
                        if let Some(planning) = pending.original["planning"].as_bool() {
                            self.planning = planning;
                        }
                        if pending.original["next"].is_object() {
                            self.prompt(pending.original["next"].clone())?;
                        }
                    }
                    "interrupt" => {
                        let mut recovered = vec![];
                        let mut messages = vec![];
                        for id in data["cancelled"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                        {
                            if let Some(prompt) = self.sent.remove(id) {
                                recovered.push(prompt["message"].clone());
                                let mut content =
                                    vec![json!({"type":"text","text":prompt["message"]})];
                                content.extend(prompt["images"].as_array().into_iter().flatten().map(|image|
                                    json!({"type":"image","mimeType":image["mimeType"],"data":image["data"]})));
                                messages.push(json!({"role":"user","content":content,"attachments":prompt["attachments"]}));
                            }
                        }
                        self.reply(
                            &json!({"type":"clear_queue","id":"claude-recovered"}),
                            json!({"steering":recovered,"followUp":[],"messages":messages}),
                        );
                        for id in pending.original["sentUuids"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                        {
                            self.sent.remove(id);
                        }
                        self.reply(&pending.original, json!({}));
                    }
                    _ => self.reply(&pending.original, data.clone()),
                }
            }
            "control_request" => self.server_request(&record)?,
            "control_cancel_request" => {
                if let Some(id) = record["request_id"].as_str() {
                    self.approvals.remove(id);
                    self.events
                        .push(json!({"type":"extension_ui_cancel","id":id}));
                }
            }
            "stream_event" => self.stream(&record),
            "assistant" if record["error"] == "authentication_failed" => {
                return Err(AUTH_ERROR.into());
            }
            "assistant" | "user" => self.message(&record, true),
            "result" => {
                let mut error = if record["is_error"] == true {
                    Some(
                        self.redact(
                            &record["errors"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(Value::as_str)
                                        .collect::<Vec<_>>()
                                        .join("; ")
                                })
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| {
                                    record["result"]
                                        .as_str()
                                        .unwrap_or("Claude turn failed.")
                                        .into()
                                }),
                        ),
                    )
                } else {
                    None
                };
                if error.is_none() && !self.cancelling {
                    error = self.compact_failure.take();
                }
                if let Some(error) = &error
                    && (!self.ready
                        || auth_error(error)
                        || record.get("startup_failure_reason").is_some())
                {
                    if auth_error(error) {
                        return Err(format!("{error}\n{AUTH_ERROR}"));
                    }
                    return Err(format!(
                        "Claude failed: {error}. Sign in with the Claude CLI outside Shina, then retry. No prompt was retried."
                    ));
                }
                if self.compacting {
                    if error.is_none() && !self.cancelling {
                        error = Some(
                            "Claude ended without confirming native compaction completed.".into(),
                        );
                    }
                    self.end_compaction(error.as_deref());
                }
                if let Some(window) = record["modelUsage"][&self.usage_model]["contextWindow"]
                    .as_u64()
                    .or_else(|| {
                        if record["modelUsage"]
                            .as_object()
                            .is_some_and(|m| m.len() == 1)
                        {
                            record["modelUsage"].as_object()?.values().next()?["contextWindow"]
                                .as_u64()
                        } else {
                            None
                        }
                    })
                {
                    self.model["contextWindow"] = json!(window);
                }
                self.publish_usage();
                self.running = record["queued_turn_count"].as_u64().unwrap_or(0) > 0;
                if !self.running {
                    if let Some(error) = &error {
                        self.events.push(json!({"type":"message_end","message":{
                            "role":"assistant","stopReason":"error","errorMessage":error}}));
                    }
                    self.events.push(json!({"type":"agent_end","messages":[]}));
                    if self.cancelling || error.is_some() || self.queued.is_empty() {
                        self.events
                            .push(json!({"type":"agent_settled","aborted":self.cancelling}));
                    }
                    if !self.cancelling
                        && error.is_none()
                        && let Some(next) = self.queued.pop_front()
                    {
                        self.queue_update();
                        if let Some(implement) = next["restorePermissionMode"].as_bool() {
                            self.request(json!({"subtype":"set_permission_mode",
                                "mode":if implement {permission_mode(self.access())} else {"plan"}}),
                                json!({"type":"set_access_mode","accessMode":self.access(),"planning":!implement,"next":next}));
                        } else {
                            self.prompt(next)?;
                        }
                    }
                }
            }
            "system" => match record["subtype"].as_str().unwrap_or("") {
                "init" => {
                    if record["mcp_servers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|server| server["name"] == "shina" && server["status"] == "failed")
                    {
                        return Err(
                            "Claude could not connect to this thread's Shina MCP server.".into(),
                        );
                    }
                    if let Some(caps) = record["capabilities"].as_array() {
                        self.capabilities
                            .extend(caps.iter().filter_map(Value::as_str).map(str::to_owned));
                        self.update("capabilities", record["capabilities"].clone());
                        self.update(
                            "turnCapabilities",
                            json!(crate::model::TurnCapabilities::new(
                                self.capabilities.contains("interrupt_cancel_queued_v1"),
                                false,
                            )),
                        );
                    }
                    if let Some(id) = record["model"].as_str() {
                        if let Some(model) = self.models.iter().find(|m| m["id"] == id) {
                            self.model = model.clone();
                        }
                    }
                }
                "status" if record["status"] == "compacting" => self.begin_compaction(),
                "status" if record["compact_result"].is_string() => {
                    if record["compact_result"] == "failed" && !self.cancelling {
                        self.compact_failure = Some(
                            self.redact(
                                record["compact_error"]
                                    .as_str()
                                    .unwrap_or("Claude compaction failed."),
                            ),
                        );
                    }
                    if self.compacting {
                        let error = self.compact_failure.clone();
                        self.end_compaction(error.as_deref());
                    }
                }
                "compact_boundary" => {
                    if !self.compact_reported {
                        self.begin_compaction();
                        let error = self.compact_failure.clone();
                        self.end_compaction(error.as_deref());
                    }
                }
                "api_retry" if record["error_status"] == 401 => {
                    return Err(AUTH_ERROR.into());
                }
                "local_command_output" => {
                    let message = json!({"role":"assistant","content":[{"type":"text","text":record["content"]}],"stopReason":"stop"});
                    self.history.push(message.clone());
                    self.events
                        .push(json!({"type":"message_start","message":{"role":"assistant"}}));
                    self.events
                        .push(json!({"type":"message_end","message":message}));
                }
                "commands_changed" => {
                    self.set_commands(&record["commands"]);
                    self.reply(
                        &json!({"type":"get_commands"}),
                        json!({"commands":self.commands}),
                    );
                }
                _ => {}
            },
            "rate_limit_event" => {
                if self.compacting && record["rate_limit_info"]["status"] == "rejected" {
                    self.compact_failure =
                        Some("Claude usage limit reached during native compaction.".into());
                }
                self.usage_limits =
                    crate::usage_limits::claude(&record["rate_limit_info"], &self.usage_limits);
                self.update("usageLimits", self.usage_limits.clone());
            }
            "auth_status" if record["error"].is_string() => {
                return Err(AUTH_ERROR.into());
            }
            _ => {}
        }
        Ok(())
    }

    fn server_request(&mut self, record: &Value) -> Result<(), String> {
        let request = &record["request"];
        let id = record["request_id"]
            .as_str()
            .ok_or("Claude request lacks request_id.")?;
        if self.approvals.contains_key(id) {
            return Ok(());
        }
        let hook = request["subtype"] == "hook_callback";
        if hook
            && (request["callback_id"] != "shina-policy"
                || request["input"]["hook_event_name"] != "PreToolUse")
        {
            self.control_error(id, "Unknown Shina hook.");
            return Ok(());
        }
        if !hook && request["subtype"] != "can_use_tool" {
            self.control_error(id, "Shina does not support this Claude control request.");
            return Ok(());
        }
        let data = if hook { &request["input"] } else { request };
        let name = data["tool_name"].as_str().unwrap_or("").to_owned();
        let input = if hook {
            &data["tool_input"]
        } else {
            &data["input"]
        };
        let approval = Approval {
            request_id: id.into(),
            input: input.clone(),
            name: name.clone(),
            tool_id: data["tool_use_id"].clone(),
            hook,
        };
        if self.cancelling || self.approvals.len() >= 32 {
            self.answer(approval, false, None);
        } else if name == "ExitPlanMode" {
            let plan = input["plan"].as_str().unwrap_or("").to_owned();
            if hook && plan.trim().is_empty() && self.access() != "read-only" {
                // The permission request includes the native plan-file contents;
                // the earlier pre-tool input may not have that augmentation.
                self.control_reply(id,json!({"hookSpecificOutput":{"hookEventName":"PreToolUse",
                    "permissionDecision":"ask","permissionDecisionReason":"Shina must capture and confirm the proposed plan."}}));
                return Ok(());
            }
            if plan.trim().is_empty() {
                self.answer(approval, false, None);
                self.events.push(json!({"type":"extension_ui_request","method":"notify",
                    "message":"Claude did not supply plan markdown. Shina refused to approve an empty proposal."}));
                return Ok(());
            }
            self.update(
                "plan",
                json!({"type":"plan","text":plan,"markdown":plan,"plan":[],
                "status":"proposed","toolCallId":approval.tool_id}),
            );
            self.planning = true;
            if self.access() == "read-only" {
                self.answer(approval, false, None);
            } else {
                self.events.push(json!({"type":"extension_ui_request","id":id,"method":"confirm",
                    "title":"Approve Claude's plan?","message":format!("{plan}\n\nApprove to implement, or decline to keep planning.")}));
                self.approvals.insert(id.into(), approval);
            }
        } else if name == "AskUserQuestion" {
            let mut questions = input["questions"].as_array().cloned().unwrap_or_default();
            if questions.is_empty() || questions.len() > 32 {
                self.answer(approval, false, None);
            } else {
                for q in &mut questions {
                    // Claude keys its answers by question text, not header or array index.
                    q["id"] = q["question"].clone();
                    if !q["options"].is_array() {
                        q["options"] = json!([]);
                    }
                }
                self.events.push(
                    json!({"type":"extension_ui_request","id":id,"method":"input",
                    "title":"Claude needs your input","questions":questions}),
                );
                self.approvals.insert(id.into(), approval);
            }
        } else if self.access() == "read-only" && !read_tool(&name) {
            self.answer(approval, false, None);
        } else if !hook && self.planning && !read_tool(&name) && name != "TodoWrite" {
            // Native plan-file edits need no callback. Other Plan-mode writes
            // must not inherit Full access or Auto-accept edits before approval.
            self.answer(approval, false, None);
        } else if hook {
            // Continue normal CLI permission evaluation; only read-only must override allow rules.
            self.control_reply(id, json!({}));
        } else if read_tool(&name)
            || self.access() == "full-access"
            || self.access() == "auto-accept-edits" && edit_tool(&name)
        {
            self.answer(approval, true, None);
        } else {
            self.events.push(json!({"type":"extension_ui_request","id":id,"method":"confirm",
                "title":format!("Allow Claude {name}?"),"message":self.redact(&format!("{}\n{input}",data["decision_reason"].as_str().unwrap_or("")))}));
            self.approvals.insert(id.into(), approval);
        }
        Ok(())
    }

    fn control_reply(&mut self, id: &str, response: Value) {
        self.wire.push(json!({"type":"control_response","response":{
            "subtype":"success","request_id":id,"response":response}}));
    }

    fn control_error(&mut self, id: &str, error: &str) {
        self.wire.push(json!({"type":"control_response","response":{
            "subtype":"error","request_id":id,"error":error}}));
    }

    fn answer(&mut self, approval: Approval, allow: bool, updated: Option<Value>) {
        let message = if approval.name == "ExitPlanMode" {
            "Shina captured your proposed plan. Stop and wait for the user's feedback or implementation request."
        } else {
            "Shina refused this tool call. Do not bypass this decision."
        };
        let input = updated.unwrap_or(approval.input);
        let response = if approval.hook {
            json!({"hookSpecificOutput":{"hookEventName":"PreToolUse",
                "permissionDecision":if allow {"allow"} else {"deny"},
                "permissionDecisionReason":message,"updatedInput":input}})
        } else if allow {
            json!({"behavior":"allow","updatedInput":input,"toolUseID":approval.tool_id})
        } else {
            json!({"behavior":"deny","message":message,"toolUseID":approval.tool_id})
        };
        self.control_reply(&approval.request_id, response);
    }

    fn command(&mut self, record: Value) -> Result<(), String> {
        let kind = record["type"].as_str().unwrap_or("");
        if kind == "extension_ui_response" {
            if let Some(approval) = record["id"]
                .as_str()
                .and_then(|id| self.approvals.remove(id))
            {
                let allowed = record["cancelled"] != true;
                if approval.name == "AskUserQuestion" {
                    let supplied = record["value"]
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(Value::Null);
                    let mut input = approval.input.clone();
                    let mut answers = serde_json::Map::new();
                    for q in input["questions"].as_array().into_iter().flatten() {
                        if let Some(key) = q["question"].as_str()
                            && let Some(value) = supplied.get(key)
                        {
                            let value = value.as_str().map(str::to_owned).unwrap_or_else(|| {
                                value
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            });
                            answers.insert(key.into(), json!(value));
                        }
                    }
                    input["answers"] = json!(answers);
                    self.answer(approval, allowed && !answers.is_empty(), Some(input));
                } else if approval.name == "ExitPlanMode" {
                    let approve =
                        allowed && record["confirmed"] == true && self.access() != "read-only";
                    self.update(
                        "planDecision",
                        json!(if approve { "approved" } else { "keep-planning" }),
                    );
                    // Like KSG, never let ExitPlanMode run implementation in the planning turn.
                    self.answer(approval, false, None);
                    if allowed && self.queued.len() < 32 {
                        self.queued
                            .push_back(json!({"type":"prompt","restorePermissionMode":approve,"message":if approve {
                            "The user approved the captured plan. Implement it now."
                        } else { "Keep planning. The user has not approved implementation." }}));
                        self.queue_update();
                    }
                } else {
                    let allow = allowed
                        && record["confirmed"] == true
                        && (self.access() != "read-only" || read_tool(&approval.name));
                    self.answer(approval, allow, None);
                }
            }
            return Ok(());
        }
        if !self.ready {
            return Err("Claude session is not initialized.".into());
        }
        match kind {
            "get_state" => self.reply(&record, self.state()),
            "get_messages" | "get_entries" => {
                let mut record = record;
                record["type"] = json!("get_messages");
                self.reply(
                    &record,
                    json!({"messages":self.history,"compactions":self.compactions}),
                );
            }
            "get_available_models" => self.reply(&record, json!({"models":self.models})),
            "get_available_thinking_levels" => self.reply(&record, json!({"levels":self.levels()})),
            "get_commands" => self.reply(&record, json!({"commands":self.commands})),
            "get_session_stats" => self.reply(&record, json!({"contextUsage":self.usage})),
            "set_session_name" => self.reply(&record, json!({})),
            "set_model" => {
                if self.running || !self.models.iter().any(|m| m["id"] == record["modelId"]) {
                    self.reject(&record, "Choose an advertised Claude model while idle.");
                } else {
                    self.request(
                        json!({"subtype":"set_model","model":record["modelId"]}),
                        record,
                    );
                }
            }
            "set_thinking_level" => {
                if self.running
                    || !self
                        .levels()
                        .as_array()
                        .is_some_and(|a| a.contains(&record["level"]))
                {
                    self.reject(&record, "Choose an advertised Claude effort while idle.");
                } else {
                    self.request(json!({"subtype":"apply_flag_settings","settings":{"effortLevel":record["level"]}}), record);
                }
            }
            "set_option" => {
                if self.running {
                    self.reject(
                        &record,
                        "Wait for Claude to finish before changing options.",
                    );
                } else if let Some(option_id) = record["optionId"].as_str() {
                    let val = &record["value"];
                    match option_id {
                        "effort" => {
                            let effort = val.as_str().unwrap_or("");
                            let mapped = match effort {
                                "ultracode" => "xhigh",
                                "ultrathink" => "",
                                other => other,
                            };
                            self.request(
                                json!({"subtype":"apply_flag_settings","settings":{"effortLevel":mapped}}),
                                record,
                            );
                        }
                        "fastMode" => {
                            self.request(
                                json!({"subtype":"apply_flag_settings","settings":{"fastMode":val}}),
                                record,
                            );
                        }
                        "ultracode" => {
                            self.request(
                                json!({"subtype":"apply_flag_settings","settings":{"ultracode":val}}),
                                record,
                            );
                        }
                        "thinking" => {
                            self.request(
                                json!({"subtype":"apply_flag_settings","settings":{"alwaysThinkingEnabled":val}}),
                                record,
                            );
                        }
                        "contextWindow" => {
                            if let Some(base_id) = self.model["id"].as_str() {
                                let base = base_id.strip_suffix("[1m]").unwrap_or(base_id);
                                let api_id = if val.as_str() == Some("1m") {
                                    format!("{base}[1m]")
                                } else {
                                    base.to_owned()
                                };
                                self.request(json!({"subtype":"set_model","model":api_id}), record);
                            } else {
                                self.reply(&record, json!({}));
                            }
                        }
                        _ => self.reply(&record, json!({})),
                    }
                }
            }
            "set_access_mode" => {
                let mode = record["accessMode"].as_str().unwrap_or("read-only");
                if mode == "read-only" {
                    // Tighten the hook before the CLI processes any in-flight tool request.
                    self.start.link.as_mut().unwrap().access_mode = mode.into();
                }
                self.request(
                    json!({"subtype":"set_permission_mode","mode":if self.planning && mode!="read-only" {"plan"} else {permission_mode(mode)}}),
                    record,
                );
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
                let mut record = record;
                record["sentUuids"] = json!(self.sent.keys().collect::<Vec<_>>());
                self.cancelling = self.running;
                for (_, approval) in std::mem::take(&mut self.approvals) {
                    self.answer(approval, false, None);
                }
                self.request(json!({"subtype":"interrupt","cancel_queued":true}), record);
                if !self.running {
                    self.events
                        .push(json!({"type":"agent_settled","aborted":true}));
                }
            }
            "compact" => {
                if self.running {
                    self.reject(
                        &record,
                        "Wait for Claude to finish before compacting context.",
                    );
                } else {
                    let instructions = record["customInstructions"].as_str().unwrap_or("");
                    let input = json!({"message":format!("/compact {instructions}")});
                    self.compact = Some(record);
                    self.cancelling = false;
                    self.begin_compaction();
                    self.send_input(&input)?;
                }
            }
            "prompt"
                if self.running
                    && (record["streamingBehavior"] != "steer"
                        || self.compacting
                        || !self.capabilities.contains("interrupt_cancel_queued_v1")) =>
            {
                if self.queued.len() >= 32 {
                    self.reject(&record, "Claude follow-up queue is full.");
                } else {
                    self.queued.push_back(record.clone());
                    self.reply(&record, json!({"disposition":"queued"}));
                    self.queue_update();
                }
            }
            "prompt" => self.prompt(record)?,
            _ => self.reject(&record, format!("Claude does not support {kind} in Shina.")),
        }
        Ok(())
    }

    fn send_input(&mut self, record: &Value) -> Result<(), String> {
        let content = input_blocks(record)?;
        if self.sent.len() >= 32 {
            return Err("Claude has 32 unconsumed inputs. Wait for the current turn.".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.sent.insert(id.clone(), record.clone());
        self.wire
            .push(json!({"type":"user","uuid":id,"session_id":self.session,
            "parent_tool_use_id":null,"message":{"role":"user","content":content}}));
        self.running = true;
        Ok(())
    }

    fn prompt(&mut self, record: Value) -> Result<(), String> {
        let was_running = self.running;
        if let Err(error) = self.send_input(&record) {
            self.reject(&record, error);
            return Ok(());
        }
        self.cancelling = false;
        if !was_running {
            self.compact_failure = None;
            self.compact_reported = false;
            self.events.push(json!({"type":"agent_start"}));
        }
        // Acceptance is transport acceptance. Do not retry this message after an error.
        self.reply(&record, json!({}));
        Ok(())
    }

    fn queue_update(&mut self) {
        let texts: Vec<_> = self.queued.iter().map(|r| r["message"].clone()).collect();
        self.events
            .push(json!({"type":"queue_update","steering":[],"followUp":texts}));
    }

    fn stream(&mut self, record: &Value) {
        let parent = record["parent_tool_use_id"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let event = &record["event"];
        match event["type"].as_str().unwrap_or("") {
            "message_start" => {
                let id = event["message"]["id"].as_str().unwrap_or("").to_owned();
                if parent.is_empty() {
                    self.running = true;
                    self.streamed.insert(id);
                    self.events
                        .push(json!({"type":"message_start","message":{"role":"assistant"}}));
                    self.last_usage = event["message"]["usage"].clone();
                }
            }
            "content_block_start" => {
                self.blocks.insert(
                    (parent, event["index"].as_u64().unwrap_or(0)),
                    event["content_block"].clone(),
                );
            }
            "content_block_delta" => {
                let index = event["index"].as_u64().unwrap_or(0);
                let delta = &event["delta"];
                if delta["type"] == "input_json_delta" {
                    if let Some(block) = self.blocks.get_mut(&(parent.clone(), index)) {
                        let mut text = block["partial_json"].as_str().unwrap_or("").to_owned();
                        text.push_str(delta["partial_json"].as_str().unwrap_or(""));
                        block["partial_json"] = json!(text);
                    }
                } else if parent.is_empty() {
                    let thinking = delta["type"] == "thinking_delta";
                    let text = if thinking {
                        &delta["thinking"]
                    } else {
                        &delta["text"]
                    };
                    if text.is_string() {
                        self.events.push(json!({"type":"message_update","assistantMessageEvent":{
                            "type":if thinking {"thinking_delta"} else {"text_delta"},"delta":text,"contentIndex":index}}));
                    }
                }
            }
            "content_block_stop" => {
                if let Some(mut block) = self
                    .blocks
                    .remove(&(parent.clone(), event["index"].as_u64().unwrap_or(0)))
                    && block["type"] == "tool_use"
                {
                    if let Some(text) = block["partial_json"].as_str() {
                        block["input"] = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
                    }
                    self.tool_start(&block, &parent, true);
                }
            }
            "message_delta" if parent.is_empty() => {
                if let Some(fields) = event["usage"].as_object() {
                    for (key, value) in fields {
                        self.last_usage[key] = value.clone();
                    }
                }
            }
            _ => {}
        }
    }

    fn tool_start(&mut self, block: &Value, parent: &str, emit: bool) -> Value {
        let id = block["id"].as_str().unwrap_or("");
        let name = block["name"].as_str().unwrap_or("Claude tool");
        let mut args = block["input"].clone();
        if args.get("path").is_none()
            && let Some(path) = args.get("file_path").cloned()
        {
            args["path"] = path;
        }
        let tool = json!({"type":"toolCall","id":id,"name":tool_name(name),
            "arguments":args,"parentToolCallId":if parent.is_empty(){Value::Null}else{json!(parent)}});
        if self.tools.insert(id.into(), tool.clone()).is_none() && emit {
            self.events.push(json!({"type":"tool_execution_start","toolCallId":id,
                "toolName":tool["name"],"args":tool["arguments"],"parentToolCallId":tool["parentToolCallId"]}));
            if tool["name"] == "subagent" {
                self.events
                    .push(json!({"type":"tool_execution_update","toolCallId":id,
                    "partialResult":{"content":[],"details":{"results":[{
                        "agent":args["subagent_type"].as_str().unwrap_or("Claude"),
                        "task":args["prompt"].as_str().unwrap_or(""),"finished":false,"messages":[]
                    }]}}}));
            }
        }
        if name == "TodoWrite" {
            let plan: Vec<_> = args["todos"].as_array().into_iter().flatten().map(|todo| json!({
                "step":todo["content"],"status":match todo["status"].as_str() {
                    Some("completed")=>"completed",Some("in_progress")=>"inProgress",_=>"pending"
                }
            })).collect();
            self.update("plan", json!({"plan":plan}));
        }
        tool
    }

    fn message(&mut self, record: &Value, emit: bool) {
        let id = record["uuid"]
            .as_str()
            .or_else(|| record["message"]["id"].as_str())
            .unwrap_or("");
        if !id.is_empty() && !self.seen.insert(id.into()) {
            return;
        }
        let parent = record["parent_tool_use_id"].as_str().unwrap_or("");
        let assistant = record["type"] == "assistant";
        if let Some(uuid) = record["user_message_uuid"].as_str() {
            self.sent.remove(uuid);
        }
        if assistant
            && parent.is_empty()
            && emit
            && !self
                .streamed
                .remove(record["message"]["id"].as_str().unwrap_or(""))
        {
            self.events
                .push(json!({"type":"message_start","message":{"role":"assistant"}}));
        }
        let mut content = vec![];
        let raw = &record["message"]["content"];
        if let Some(text) = raw.as_str() {
            content.push(json!({"type":"text","text":text}));
        }
        for block in raw.as_array().into_iter().flatten() {
            match block["type"].as_str().unwrap_or("") {
                "tool_use" => content.push(self.tool_start(block, parent, emit)),
                "tool_result" => {
                    let call = block["tool_use_id"].as_str().unwrap_or("");
                    let output = if let Some(text) = block["content"].as_str() {
                        json!([{"type":"text","text":text}])
                    } else { block["content"].clone() };
                    let tool = self.tools.get(call).cloned().unwrap_or(Value::Null);
                    let mut result = json!({"content":output});
                    if tool["name"] == "subagent" {
                        result["details"] = json!({"results":[{
                            "agent":tool["arguments"]["subagent_type"].as_str().unwrap_or("Claude"),
                            "task":tool["arguments"]["prompt"].as_str().unwrap_or(""),
                            "finished":true,"exitCode":if block["is_error"]==true {1}else{0},
                            "messages":[{"role":"assistant","content":output}]
                        }]});
                    }
                    self.history.push(json!({"role":"toolResult","toolCallId":call,
                        "toolName":tool["name"],"content":output,"details":result["details"],"isError":block["is_error"]}));
                    if emit {
                        self.events.push(json!({"type":"tool_execution_end","toolCallId":call,
                            "result":result,"isError":block["is_error"]}));
                    }
                }
                "image" => content.push(json!({"type":"image","mimeType":block["source"]["media_type"],"data":block["source"]["data"]})),
                "text" | "thinking" => content.push(block.clone()),
                _ => {}
            }
        }
        if content.is_empty() {
            return;
        }
        let message = json!({"role":if assistant {"assistant"}else{"user"},
            "content":content,"stopReason":if record["is_error"]==true {"error"}else{"stop"}});
        // Nested assistant prose belongs to its Task result, not the main answer.
        if !parent.is_empty() {
            if assistant {
                self.history.push(json!({"role":"assistant","content":content.into_iter().filter(|b|b["type"]=="toolCall").collect::<Vec<_>>()}));
            }
            return;
        }
        if assistant {
            if record["message"]["usage"].is_object() {
                self.last_usage = record["message"]["usage"].clone();
            }
            if let Some(model) = record["message"]["model"].as_str() {
                self.usage_model = model.into();
            }
            if emit {
                self.events
                    .push(json!({"type":"message_end","message":message}));
            }
        } else if emit && record["isMeta"] != true {
            self.events
                .push(json!({"type":"message_start","message":message}));
        }
        if record["isMeta"] != true {
            self.history.push(message);
        }
    }

    fn publish_usage(&mut self) {
        // Last assistant request describes resident context. result.modelUsage is cumulative.
        let tokens = self.last_usage["input_tokens"].as_u64().map(|input| {
            input
                + self.last_usage["cache_read_input_tokens"]
                    .as_u64()
                    .unwrap_or(0)
                + self.last_usage["cache_creation_input_tokens"]
                    .as_u64()
                    .unwrap_or(0)
                + self.last_usage["output_tokens"].as_u64().unwrap_or(0)
        });
        let window = self
            .model
            .get("contextWindow")
            .cloned()
            .unwrap_or(Value::Null);
        self.usage = json!({"tokens":tokens,"contextWindow":window});
        self.events
            .push(json!({"type":"context_usage","data":{"contextUsage":self.usage}}));
    }

    fn begin_compaction(&mut self) {
        if !self.compacting {
            self.compacting = true;
            self.compact_failure = None;
            self.compact_reported = false;
            self.events.push(json!({"type":"compaction_start"}));
        }
    }

    fn end_compaction(&mut self, error: Option<&str>) {
        self.compacting = false;
        self.compact_reported = true;
        self.last_usage = Value::Null;
        self.usage_model.clear();
        self.usage = Value::Null;
        let outcome = json!({"errorMessage":error,"aborted":self.cancelling});
        self.compactions.push(outcome.clone());
        self.events
            .push(json!({"type":"compaction_end","errorMessage":error,"aborted":self.cancelling}));
        if let Some(request) = self.compact.take() {
            if let Some(error) = error {
                self.reject(&request, error);
            } else {
                self.reply(&request, outcome);
            }
        }
    }
}

fn auth_error(message: &str) -> bool {
    let text = message.to_lowercase();
    [
        "authentication",
        "unauthorized",
        "not logged in",
        "invalid api key",
        "login required",
        "oauth",
        "401",
    ]
    .iter()
    .any(|s| text.contains(s))
}

fn input_blocks(record: &Value) -> Result<Vec<Value>, String> {
    let mut text = record["message"].as_str().unwrap_or("").to_owned();
    if let Some(skill) = text.strip_prefix("/skill:") {
        text = format!("/{skill}");
    }
    let attachments: Vec<_> = record["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| crate::attachments::Attachment::from_value(v).ok_or("Invalid Claude attachment."))
        .collect::<Result<_, _>>()?;
    crate::attachments::validate_prompt_images(&attachments)?;
    for attachment in attachments {
        let path = attachment
            .path
            .as_ref()
            .ok_or("Claude attachment lacks an absolute path.")?;
        text.push_str(&format!(
            "\n\n[Attached file \"{}\" is saved at: {}]",
            attachment.name,
            path.display()
        ));
    }
    let mut blocks = vec![];
    if !text.is_empty() {
        blocks.push(json!({"type":"text","text":text}));
    }
    for image in record["images"].as_array().into_iter().flatten() {
        let mime = image["mimeType"]
            .as_str()
            .ok_or("Claude image lacks MIME type.")?;
        let data = image["data"].as_str().ok_or("Claude image lacks data.")?;
        blocks
            .push(json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}}));
    }
    if blocks.is_empty() {
        return Err("Claude requires text or attachments.".into());
    }
    Ok(blocks)
}

fn config_directory() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                    .unwrap_or_default(),
            )
            .join(".claude")
        })
}

/// SDK getSessionMessages reads transcript files, not replay-user-messages.
/// Only transcript JSONL is read. Never inspect .claude.json, account or credential files.
fn load_history(client: &mut Client) -> Result<(), String> {
    if !matches!(client.start.source, SessionSource::Resume(_)) {
        return Ok(());
    }
    let projects = client.config.join("projects");
    let name = format!("{}.jsonl", client.session);
    let mut found = None;
    for entry in std::fs::read_dir(&projects).map_err(|_| "Claude session history is unavailable. Cannot resume without restoring its transcript.")?.take(16384).flatten() {
        let path = entry.path().join(&name);
        if path.is_file() { found = Some(path); break; }
    }
    let path = found
        .ok_or("Claude session history was not found. Refusing to start a replacement session.")?;
    let records = read_history(&path)?;
    let mut by_id = HashMap::new();
    let mut leaf = None;
    for record in &records {
        if record["isSidechain"] == true {
            continue;
        }
        if let Some(id) = record["uuid"].as_str()
            && matches!(
                record["type"].as_str(),
                Some("user" | "assistant" | "system" | "progress")
            )
        {
            by_id.insert(id, record);
            if matches!(record["type"].as_str(), Some("user" | "assistant"))
                || record["subtype"] == "compact_boundary"
            {
                leaf = Some(id);
            }
        }
    }
    let mut chain = vec![];
    let mut seen = HashSet::new();
    while let Some(id) = leaf {
        if !seen.insert(id) {
            return Err("Claude session history contains a cycle.".into());
        }
        let record = by_id
            .get(id)
            .ok_or("Claude session history has a missing parent. Cannot reconstruct it safely.")?;
        chain.push(*record);
        leaf = record["parentUuid"].as_str();
    }
    chain.reverse();
    for record in chain {
        if record["subtype"] == "compact_boundary" {
            client.compactions.push(json!({"running":false}));
        } else {
            replay_subagents(
                client,
                record,
                &path.with_extension("").join("subagents"),
                0,
            )?;
            client.message(record, false);
        }
    }
    Ok(())
}

fn replay_subagents(
    client: &mut Client,
    record: &Value,
    directory: &Path,
    depth: usize,
) -> Result<(), String> {
    if depth >= 8 {
        return Ok(());
    }
    let Some(agent) = record["toolUseResult"]["agentId"].as_str() else {
        return Ok(());
    };
    if agent.is_empty()
        || agent.len() > 128
        || !agent
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err("Claude history has an invalid subagent id.".into());
    }
    let Some(call) = record["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|b| b["tool_use_id"].as_str())
    else {
        return Ok(());
    };
    let path = directory.join(format!("agent-{agent}.jsonl"));
    if !path.is_file() {
        return Ok(());
    }
    for mut nested in read_history(&path)? {
        replay_subagents(client, &nested, directory, depth + 1)?;
        nested["parent_tool_use_id"] = json!(call);
        client.message(&nested, false);
    }
    Ok(())
}

fn read_history(path: &Path) -> Result<Vec<Value>, String> {
    let file = std::fs::File::open(path).map_err(|_| "Cannot read Claude transcript.")?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
        return Err("Claude transcript exceeds Shina's 64 MiB history limit.".into());
    }
    BufReader::new(file)
        .lines()
        .map(|line| {
            let line = line.map_err(|e| e.to_string())?;
            serde_json::from_str(&line)
                .map_err(|_| "Claude transcript contains invalid JSON.".into())
        })
        .collect()
}

/// Ported filesystem roots from KSG Drivers/ClaudeSkills.ts. Settings are metadata only.
fn discover_skills(config: &Path, workspace: &Path) -> Vec<Value> {
    let mut settings = json!({});
    let managed = if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("PROGRAMDATA").unwrap_or_default())
            .join("ClaudeCode/managed-settings.json")
    } else {
        PathBuf::from("/etc/claude-code/managed-settings.json")
    };
    let mut settings_paths = vec![
        config.join("settings.json"),
        workspace.join(".claude/settings.json"),
        workspace.join(".claude/settings.local.json"),
    ];
    if let Some(root) = repository_root(workspace)
        && root != workspace
    {
        settings_paths.push(root.join(".claude/settings.local.json"));
    }
    settings_paths.push(managed);
    for path in settings_paths {
        if let Some(value) = metadata_json(&path) {
            for key in ["enabledPlugins", "skillOverrides"] {
                if let Some(fields) = value[key].as_object() {
                    for (name, value) in fields {
                        settings[key][name] = value.clone();
                    }
                }
            }
        }
    }
    let mut roots = vec![
        (config.join("skills"), String::new()),
        (workspace.join(".claude/skills"), String::new()),
    ];
    let plugins = std::env::var_os("CLAUDE_CODE_PLUGIN_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| config.join("plugins"));
    let plugins = if plugins.is_absolute() {
        plugins
    } else {
        workspace.join(plugins)
    };
    if let Some(installed) = metadata_json(&plugins.join("installed_plugins.json")) {
        for (key, entries) in installed["plugins"].as_object().into_iter().flatten() {
            if settings["enabledPlugins"][key] != true {
                continue;
            }
            let applicable = entries.as_array().into_iter().flatten().find(|entry| {
                matches!(entry["scope"].as_str(), Some("user" | "managed"))
                    || entry["projectPath"].as_str().is_some_and(|p| {
                        Path::new(p) == workspace
                            || repository_root(Path::new(p))
                                .zip(repository_root(workspace))
                                .is_some_and(|(a, b)| a == b)
                    })
            });
            if let Some(path) = applicable.and_then(|e| e["installPath"].as_str()) {
                let path = PathBuf::from(path);
                let manifest = metadata_json(&path.join(".claude-plugin/plugin.json"));
                let standalone = manifest.as_ref().is_none_or(|m| m.get("skills").is_none());
                let prefix = manifest
                    .as_ref()
                    .and_then(|m| m["name"].as_str())
                    .unwrap_or_else(|| key.split('@').next().unwrap_or(key))
                    .to_owned();
                roots.push((path.join("skills"), prefix.clone()));
                if let Some(manifest) = manifest {
                    let declared: Vec<_> = manifest["skills"]
                        .as_str()
                        .into_iter()
                        .chain(
                            manifest["skills"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str),
                        )
                        .collect();
                    for relative in declared {
                        let candidate = path.join(relative);
                        if candidate
                            .canonicalize()
                            .ok()
                            .zip(path.canonicalize().ok())
                            .is_some_and(|(candidate, root)| candidate.starts_with(root))
                        {
                            roots.push((candidate, prefix.clone()));
                        }
                    }
                }
                if standalone && !path.join("skills").is_dir() && path.join("SKILL.md").is_file() {
                    roots.push((path, prefix));
                }
            }
        }
    }
    let mut commands = vec![];
    let mut seen = HashSet::new();
    for (root, prefix) in roots {
        let single = root.join("SKILL.md").is_file() && !prefix.is_empty();
        let paths = if single {
            vec![root]
        } else {
            let mut paths: Vec<_> = std::fs::read_dir(root)
                .into_iter()
                .flatten()
                .take(4096)
                .flatten()
                .map(|e| e.path())
                .collect();
            paths.sort();
            paths
        };
        for path in paths {
            let Ok(file) = std::fs::File::open(path.join("SKILL.md")) else {
                continue;
            };
            if file.metadata().map_or(true, |m| m.len() > 256 * 1024) {
                continue;
            }
            let mut contents = String::new();
            if Read::take(file, 256 * 1024)
                .read_to_string(&mut contents)
                .is_err()
            {
                continue;
            }
            let Some(frontmatter) = skill_frontmatter(&contents) else {
                continue;
            };
            let mut name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let description = frontmatter["description"].as_str().unwrap_or("");
            if single && let Some(declared) = frontmatter["name"].as_str() {
                name = declared.into();
            }
            if !prefix.is_empty() {
                name = format!(
                    "{prefix}:{}",
                    name.strip_prefix(&format!("{prefix}:")).unwrap_or(&name)
                );
            }
            if frontmatter["user-invocable"] == false
                || frontmatter["user-invocable"].as_str().is_some_and(|s| {
                    matches!(s.to_lowercase().as_str(), "false" | "no" | "off" | "n")
                })
                || prefix.is_empty() && settings["skillOverrides"][&name] == "off"
                || !seen.insert(name.clone())
            {
                continue;
            }
            commands.push(
                json!({"name":format!("skill:{name}"),"description":description,"source":"skill"}),
            );
        }
    }
    commands
}

fn skill_frontmatter(contents: &str) -> Option<Value> {
    let mut lines = contents.lines();
    if lines.next() != Some("---") {
        return Some(json!({}));
    }
    let mut header = vec![];
    let mut closed = false;
    for line in lines {
        if line == "---" {
            closed = true;
            break;
        }
        header.push(line.to_owned());
    }
    if !closed {
        return None;
    }
    let header = header.join("\n");
    serde_yaml_ng::from_str::<Value>(&header)
        .ok()
        .filter(Value::is_object)
        .or_else(|| {
            // Match KSG's repair of Claude's accepted unquoted ": " scalar descriptions.
            let repaired = header
                .lines()
                .map(|line| {
                    if let Some(value) = line.strip_prefix("description:")
                        && value.contains(": ")
                        && !value
                            .trim_start()
                            .starts_with(['"', '\'', '|', '>', '[', '{'])
                    {
                        return format!("description: {}", json!(value.trim()));
                    }
                    line.into()
                })
                .collect::<Vec<_>>()
                .join("\n");
            serde_yaml_ng::from_str::<Value>(&repaired)
                .ok()
                .filter(Value::is_object)
        })
}

fn metadata_json(path: &Path) -> Option<Value> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > 2 * 1024 * 1024 {
        return None;
    }
    serde_json_lenient::from_reader(file).ok()
}

fn repository_root(path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    let root = path.ancestors().find(|p| p.join(".git").exists())?;
    let git = root.join(".git");
    if git.is_file() {
        let pointer = std::fs::read_to_string(&git).ok()?;
        let directory = root.join(pointer.trim().strip_prefix("gitdir:")?.trim());
        if let Ok(common) = std::fs::read_to_string(directory.join("commondir"))
            && let Ok(common) = directory.join(common.trim()).canonicalize()
            && common.file_name().is_some_and(|name| name == ".git")
        {
            return common.parent().map(Path::to_path_buf);
        }
    }
    Some(root.into())
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
    run_mode(command, start, input, events, wake, stopped, owner, false)
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
    probe: bool,
) -> Result<(), String> {
    if probe && !start.workspace.exists() {
        std::fs::create_dir_all(&start.workspace)
            .map_err(|error| format!("Could not create the discovery directory: {error}"))?;
    }
    let mut client = Client::new(start);
    if let Some((_, Some(path))) = command
        .get_envs()
        .find(|(key, _)| *key == "CLAUDE_CONFIG_DIR")
    {
        client.config = PathBuf::from(path);
    }
    if !client.config.is_absolute() {
        client.config = client.start.workspace.join(&client.config);
    }
    load_history(&mut client)?;
    client.launch(&mut command);
    command
        .current_dir(&client.start.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = crate::process::Process::spawn(&mut command)
        .map_err(|e| format!("Could not start Claude: {e}"))?;
    let mut stdin = process.stdin.take().unwrap();
    let stdout = process.stdout.take().unwrap();
    let stderr = process.stderr.take().unwrap();
    let (received, records) = mpsc::sync_channel(256);
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let record = backend::read_record(&mut reader).map_err(|e| e.replace("Pi", "Claude"));
            let done = !matches!(record, Ok(Some(_)));
            if received.send(record).is_err() || done {
                break;
            }
        }
    });
    // Never retain stderr: providers and plugins may print credentials.
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
                backend::write_record(&mut stdin, &record)
                    .map_err(|e| e.replace("Pi", "Claude"))?;
                *clock.lock().unwrap() = None;
            }
            Ok::<_, String>(())
        })();
        let _ = result_tx.send(result);
    });
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
                return Err("Claude stopped reading commands.".into());
            }
            if client
                .pending
                .values()
                .any(|p| Instant::now() >= p.deadline)
            {
                return Err("Claude did not acknowledge an operation within 30 seconds.".into());
            }
            for record in std::mem::take(&mut client.wire) {
                writes
                    .try_send(record)
                    .map_err(|_| "Claude stopped accepting commands.")?;
            }
            for mut record in std::mem::take(&mut client.events) {
                crate::attachments::import_history(&mut record);
                if !backend::publish(events, wake, Event::Record(record)) {
                    return Ok(());
                }
            }
            if probe && client.ready {
                return Ok(());
            }
            for _ in 0..64 {
                match records.try_recv() {
                    Ok(Ok(Some(record))) => client.incoming(record)?,
                    Ok(Ok(None)) => return Err("Claude closed its output. Sign in with the Claude CLI if authentication failed, then resume the saved session. No prompt was retried.".into()),
                    Ok(Err(error)) => return Err(error),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return Err("Claude output reader stopped.".into()),
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
                    "Claude exited with {status}. Sign in with the Claude CLI if required. No prompt was retried."
                ));
            }
        }
    })();
    if client.compacting {
        client.end_compaction(Some("Claude disconnected during compaction."));
    }
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
            "Could not reap Claude. Restart Shina before reconnecting: {error}"
        ));
    }
    crate::process::join_for(writer, Duration::from_millis(200));
    crate::process::join_for(reader, Duration::from_millis(200));
    crate::process::join_for(errors, Duration::from_millis(200));
    result
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
