//! ACP v1 JSON-RPC over stdio, ported from KSG's effect-acp and provider-acp.
//! The UI sees the same event envelope as Pi. No provider authentication is performed here.
use crate::{
    backend::{self, Event, HostLink, Pi, SessionOwner, SessionSource},
    harness::{Agent, Start},
};
use rune::Waker;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::Stdio,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

/// Metadata-only ACP probe. It negotiates `initialize` in a short-lived
/// process and never creates a session, registers MCP or sends a prompt.
pub fn probe(agent: &Agent) -> Result<Value, String> {
    static PROBES: OnceLock<Mutex<HashMap<String, Result<Value, String>>>> = OnceLock::new();
    let key = json!({"id":agent.id,"command":agent.command,"args":agent.args,"env":agent.env})
        .to_string();
    let cache = PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().expect("ACP probes").get(&key) {
        return cached.clone();
    }
    let result = probe_uncached(agent);
    cache
        .lock()
        .expect("ACP probes")
        .insert(key, result.clone());
    result
}

/// Re-probe before allocation. Discovery may be cached, but dispatch must
/// answer from the executable and protocol state that exists now.
pub(crate) fn probe_live(agent: &Agent) -> Result<Value, String> {
    probe_uncached(agent)
}

fn probe_uncached(agent: &Agent) -> Result<Value, String> {
    let mut command = agent.process()?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // The same owned launch as sessions: no console window on Windows and the
    // job object reaps agent descendants on every return path.
    let mut child = crate::process::Process::spawn(&mut command)
        .map_err(|error| format!("Could not start ACP harness {}: {error}", agent.id))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("ACP probe could not open stdin.")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("ACP probe could not open stdout.")?;
    // KSG agentProfile: Devin's initialize advertises client terminals and the
    // cognition.ai capability meta in probes too.
    let mut client_capabilities =
        json!({"fs":{"readTextFile":false,"writeTextFile":false},"terminal":agent.id == "devin"});
    if agent.id == "devin" {
        client_capabilities["_meta"] = crate::devin::client_capabilities_meta();
    }
    let request = json!({
        "jsonrpc":"2.0","id":"shina-probe","method":"initialize",
        "params":{"protocolVersion":1,
            "clientInfo":{"name":"Shina","version":env!("CARGO_PKG_VERSION")},
            "clientCapabilities":client_capabilities}
    });
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .map_err(|error| format!("ACP probe could not write initialize: {error}"))?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut result = Err("ACP agent exited before answering initialize.".into());
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value["id"] != "shina-probe" {
                continue;
            }
            result = if value["error"].is_null() {
                if value["result"]["protocolVersion"] == 1 {
                    Ok(value["result"]["agentCapabilities"].clone())
                } else {
                    Err("This ACP agent did not negotiate protocol version 1.".into())
                }
            } else {
                Err(value["error"]["message"]
                    .as_str()
                    .unwrap_or("ACP initialize failed.")
                    .to_owned())
            };
            break;
        }
        let _ = sender.send(result);
    });
    let result = receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| {
            Err("ACP initialize timed out before advertising its capabilities.".into())
        });
    let _ = child.kill();
    let _ = child.wait();
    result
}

pub fn start(agent: Agent, start: Start, wake: Waker) -> Result<Pi, String> {
    let command = agent.process()?;
    start_command(agent, start, wake, command)
}

/// The Antigravity layer supplies its own isolated command; every other agent
/// builds one from its registry entry.
pub(crate) fn start_command(
    agent: Agent,
    start: Start,
    wake: Waker,
    command: std::process::Command,
) -> Result<Pi, String> {
    if matches!(start.source, SessionSource::Fork(_)) {
        return Err("ACP conversation forks are not supported. Start a new thread instead.".into());
    }
    if !start.metadata_only && start.link.is_none() {
        return Err("ACP requires Shina's MCP bridge. Retry after the bridge is available.".into());
    }
    let claude = crate::composer_skills::is_claude(&agent);
    let skill_args = agent.args.clone();
    let inherit_config = agent.env.iter().any(|name| name == "CLAUDE_CONFIG_DIR");
    let inherit_plugins = agent
        .env
        .iter()
        .any(|name| name == "CLAUDE_CODE_PLUGIN_CACHE_DIR");
    let session_key = match &start.source {
        SessionSource::Resume(id) => Some(PathBuf::from(format!("acp:{}:{id}", agent.id))),
        _ => None,
    };
    let owner = start.owner.clone();
    #[cfg(test)]
    let profile = crate::store::directory().expect("isolated test profile");
    Ok(Pi::worker(
        wake,
        owner,
        session_key,
        move |input, events, wake, stopped, owner| {
            let action = || {
                let skills = if claude {
                    crate::composer_skills::claude_config(inherit_config)
                        .map(|config| {
                            let config =
                                crate::composer_skills::directory_path(config, &start.workspace);
                            let plugins = inherit_plugins
                                .then(|| std::env::var_os("CLAUDE_CODE_PLUGIN_CACHE_DIR"))
                                .flatten()
                                .map(PathBuf::from)
                                .map(|path| {
                                    crate::composer_skills::directory_path(path, &start.workspace)
                                })
                                .unwrap_or_else(|| config.join("plugins"));
                            crate::composer_skills::claude_skills(
                                &config,
                                &start.workspace,
                                &skill_args,
                                &plugins,
                            )
                        })
                        .unwrap_or_default()
                } else {
                    vec![]
                };
                run(command, start, input, events, wake, stopped, owner, skills)
            };
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

struct Pending {
    request: Value,
    deadline: Option<Instant>,
}

struct Permission {
    id: Value,
    allow: Option<String>,
    session: Option<String>,
    reject: Option<String>,
    offered: Option<Vec<crate::approval::OptionItem>>,
    question: Option<Value>,
}

/// A `terminal/wait_for_exit` request answered once the child actually exits.
struct TerminalWaiter {
    id: Value,
    terminal: String,
}

/// KSG Code's opt-in ACP extension. Standard ACP v1 does not admit active steering.
fn supports_active_steering(meta: &Value) -> bool {
    let advertisement = &meta["t3code/steering"];
    advertisement["version"] == 1
        && advertisement["idempotent"] == true
        && advertisement["holdsPrompt"] == true
}

/// Protocol state is separate from process supervision so negotiation and reduction are testable.
struct Client {
    start: Start,
    capabilities: Value,
    active_steering: bool,
    restart_steering: bool,
    restart_prompt: Option<Value>,
    turn_id: Option<String>,
    session: Option<String>,
    pending: HashMap<String, Pending>,
    permissions: HashMap<String, Permission>,
    serial: usize,
    ready: bool,
    running: bool,
    cancelling: bool,
    awareness_sent: bool,
    history: Vec<Value>,
    commands: Vec<Value>,
    claude_skills: Vec<crate::composer_skills::Skill>,
    models: Vec<Value>,
    model: Value,
    model_option: Option<String>,
    option_descriptors: Vec<crate::descriptors::ProviderOptionDescriptor>,
    stdio_fallback: bool,
    tools: HashMap<String, Value>,
    queued: VecDeque<Value>,
    wire: Vec<Value>,
    events: Vec<Value>,
    /// Built-in Antigravity runtime hooks (KSG makeAntigravityAcpRuntime).
    antigravity: bool,
    antigravity_version: Option<String>,
    initialize_result: Value,
    config_options: Vec<Value>,
    thinking_levels: HashMap<String, Option<Value>>,
    /// A combined `family-level` pick awaiting its level after the family switch.
    pending_thinking: Option<(Value, String, String)>,
    /// The real `agy` CLI wraps setMode with antigravityCliSessionMode; a
    /// harnesses.json override is an arbitrary executable and does not.
    antigravity_cli: bool,
    /// A gateway URL was configured for this session (KSG authMethod "gateway").
    antigravity_gateway: bool,
    /// The configured gateway model list; non-empty enables KSG's gateway branch.
    gateway_models: String,
    /// The adapter-owned `reasoningEffort` pick, re-applied to each route select.
    gateway_effort: Option<String>,
    /// `setup.models` from session start: legacy combined ids for gateway row lookup.
    legacy_models: Value,
    /// The session's ACP `modes` block, for KSG's setMode modeState.
    modes: Value,
    file_reads: bool,
    /// KSG `registryAgentId === "devin"`: normalization, subagent routing and
    /// `shellCommands` client terminals.
    devin: bool,
    /// Child session ids Devin has introduced, via reroute metadata or subagent tools.
    devin_children: HashSet<String>,
    terminals: crate::devin::Terminals,
    terminal_waiters: Vec<TerminalWaiter>,
}

impl Client {
    fn new(start: Start) -> Self {
        let awareness_sent = start.context_sent;
        let restart_steering = !matches!(start.id.as_str(), "github-copilot-cli" | "copilot");
        let devin = start.id == "devin" && !start.metadata_only;
        Self {
            start,
            capabilities: Value::Null,
            active_steering: false,
            restart_steering,
            restart_prompt: None,
            turn_id: None,
            session: None,
            pending: HashMap::new(),
            permissions: HashMap::new(),
            serial: 0,
            ready: false,
            running: false,
            cancelling: false,
            awareness_sent,
            history: vec![],
            commands: vec![],
            claude_skills: vec![],
            models: vec![],
            model: Value::Null,
            model_option: None,
            option_descriptors: vec![],
            stdio_fallback: false,
            tools: HashMap::new(),
            queued: VecDeque::new(),
            wire: vec![],
            events: vec![],
            antigravity: false,
            antigravity_version: None,
            initialize_result: Value::Null,
            config_options: vec![],
            thinking_levels: HashMap::new(),
            pending_thinking: None,
            antigravity_cli: false,
            antigravity_gateway: false,
            gateway_models: String::new(),
            gateway_effort: None,
            legacy_models: Value::Null,
            modes: Value::Null,
            file_reads: false,
            devin,
            devin_children: HashSet::new(),
            terminals: crate::devin::Terminals::default(),
            terminal_waiters: vec![],
        }
    }

    fn available_commands(&self) -> Vec<Value> {
        let mut commands = self.commands.clone();
        for command in crate::composer_skills::command_values(&self.claude_skills) {
            if !commands.iter().any(|c| c["name"] == command["name"]) {
                commands.push(command);
            }
        }
        commands
    }

    fn request(&mut self, method: &str, params: Value, request: Value) {
        self.serial += 1;
        let id = format!("shina-acp-{}", self.serial);
        self.pending.insert(
            id.clone(),
            Pending {
                request,
                deadline: (method != "session/prompt")
                    .then(|| Instant::now() + Duration::from_secs(30)),
            },
        );
        self.wire
            .push(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
    }

    fn initialize(&mut self) {
        // KSG antigravityClientInfo + client-file-reads rule: serve reads except on the
        // qualified 1.4.5 release; writes stay off so the agent keeps its diff edits.
        let (client_info, read_files) = if self.antigravity {
            let version = self.antigravity_version.as_deref();
            self.file_reads =
                !self.start.metadata_only && !crate::antigravity::is_qualified_release(version);
            (crate::antigravity::client_info(version), self.file_reads)
        } else {
            (
                json!({"name":"Shina","version":env!("CARGO_PKG_VERSION")}),
                false,
            )
        };
        // KSG agentProfile: Devin gets client terminals plus the cognition.ai
        // capability hints that switch on subagent context and message grouping.
        let mut client_capabilities =
            json!({"fs":{"readTextFile":read_files,"writeTextFile":false},"terminal":self.devin});
        if self.devin {
            client_capabilities["_meta"] = crate::devin::client_capabilities_meta();
        }
        self.request(
            "initialize",
            json!({
                "protocolVersion":1,
                "clientInfo":client_info,
                "clientCapabilities":client_capabilities
            }),
            json!({"type":"initialize"}),
        );
    }

    fn reply(&mut self, request: &Value, data: Value) {
        self.events.push(json!({"type":"response","id":request["id"],"command":request["type"],"success":true,"data":data}));
    }

    fn reject(&mut self, request: &Value, error: impl Into<String>) {
        self.events.push(json!({"type":"response","id":request["id"],"command":request["type"],"success":false,"error":error.into()}));
    }

    fn state(&self) -> Value {
        json!({"sessionFile":self.session,"isStreaming":self.running,"model":self.model,"thinkingLevel":"",
            "turnCapabilities":self.turn_capabilities()})
    }

    fn turn_capabilities(&self) -> crate::model::TurnCapabilities {
        let mut capabilities = crate::model::TurnCapabilities::new(self.active_steering, false);
        capabilities.supports_steering_by_interrupt_restart = self.restart_steering;
        capabilities
    }

    fn history_value(&self) -> Value {
        let replayed = self.history.iter().any(|m| {
            m["role"] == "user"
                || m["role"] == "toolResult" && m["toolKind"] != "plan"
                || m["content"].as_array().is_some_and(|blocks| {
                    blocks
                        .iter()
                        .any(|b| b["type"] != "toolCall" || b["toolKind"] != "plan")
                })
        });
        json!({"messages":self.history,"historyReplayed":replayed})
    }

    fn mcp_servers(&self) -> Result<Value, String> {
        if self.start.metadata_only {
            return Ok(json!([]));
        }
        let host = self
            .start
            .link
            .as_ref()
            .ok_or("Shina MCP is unavailable.")?;
        let capabilities = &self.capabilities["mcpCapabilities"];
        if capabilities["http"] == true && !self.stdio_fallback {
            return Ok(json!([{"type":"http","name":"shina","url":host.url,
                "headers":[{"name":"Authorization","value":format!("Bearer {}",host.token)}]}]));
        }
        // Stdio MCP is mandatory in ACP v1 unless explicitly disabled by the agent.
        if capabilities["stdio"] == false {
            return Err("This ACP agent supports neither HTTP nor stdio MCP. Shina cannot register its tools.".into());
        }
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(
            json!([{"name":"shina","command":executable,"args":["--mcp-bridge"],
            "env":[{"name":crate::mcp::URL_ENV,"value":host.url},{"name":crate::mcp::TOKEN_ENV,"value":host.token}]}]),
        )
    }

    fn incoming(&mut self, record: Value) -> Result<(), String> {
        if record["jsonrpc"] != "2.0" {
            return Err("ACP agent sent a record without JSON-RPC 2.0 framing.".into());
        }
        if let Some(method) = record["method"].as_str() {
            if method == "session/update" && record.get("id").is_none() {
                let id = record["params"]["sessionId"]
                    .as_str()
                    .ok_or("ACP update has no sessionId.")?;
                if self.session.as_deref().is_some_and(|session| session != id)
                    && !(self.devin && self.devin_children.contains(id))
                {
                    return Err("ACP agent sent an update for another session.".into());
                }
                if let SessionSource::Resume(expected) = &self.start.source
                    && !self.ready
                    && expected != id
                {
                    return Err("ACP agent replayed another session.".into());
                }
                if self.devin {
                    // KSG's adapter order: resolve embedded terminal content on the raw
                    // notification, then normalizeDevinSessionUpdate reroutes children.
                    let mut resolved = record["params"].clone();
                    if let Some(content) = resolved["update"]["content"].as_array_mut() {
                        for part in content.iter_mut() {
                            if part["type"] == "terminal" {
                                let terminal = part["terminalId"].as_str().unwrap_or("");
                                let text = self
                                    .terminals
                                    .output_snapshot(id, terminal)
                                    .unwrap_or_else(|| format!("[terminal {terminal}]"));
                                *part = json!({"type":"content",
                                    "content":{"type":"text","text":text}});
                            }
                        }
                    }
                    let mut normalized = crate::devin::normalize_session_update(&resolved);
                    if let Some(child) = normalized["sessionId"].as_str().map(str::to_owned) {
                        if child != id {
                            self.devin_children.insert(child.clone());
                        }
                        // KSG projectSubagentTool keys child work `{child}:tool:{id}`.
                        if Some(child.as_str()) != self.session.as_deref()
                            && self.devin_children.contains(&child)
                            && let Some(tool_id) = normalized["update"]["toolCallId"].as_str()
                        {
                            normalized["update"]["toolCallId"] =
                                json!(format!("{child}:{tool_id}"));
                        }
                    }
                    self.update(&normalized["update"]);
                } else {
                    self.update(&record["params"]["update"]);
                }
            } else if method == "session/request_permission" && record.get("id").is_some() {
                self.permission(&record)?;
            } else if record.get("id").is_some() {
                // KSG readAntigravityClientTextFile, advertised per the qualified-release rule.
                if method == "fs/read_text_file" && self.antigravity && self.file_reads {
                    let workspace = self.start.workspace.clone();
                    let response = match crate::antigravity::read_text_file(
                        &record["params"],
                        &[workspace.as_path()],
                    ) {
                        Ok(result) => json!({"jsonrpc":"2.0","id":record["id"],"result":result}),
                        Err((code, message)) => json!({"jsonrpc":"2.0","id":record["id"],
                            "error":{"code":code,"message":message}}),
                    };
                    self.wire.push(response);
                    return Ok(());
                }
                // KSG makeAcpClientTerminals: Devin advertised `terminal` at initialize.
                if self.devin && method.starts_with("terminal/") {
                    self.terminal_request(method, &record);
                    return Ok(());
                }
                self.wire.push(json!({"jsonrpc":"2.0","id":record["id"],"error":{"code":-32601,"message":format!("Shina does not support ACP client capability {method}. Filesystem and terminal capabilities were not advertised.")}}));
                self.reject(
                    &json!({"type":"capability"}),
                    format!("ACP agent requested unsupported client capability {method}."),
                );
            }
            return Ok(());
        }
        let id = record["id"]
            .as_str()
            .ok_or("ACP response has no string request id.")?;
        let Some(pending) = self.pending.remove(id) else {
            return Err("ACP agent answered an unknown request.".into());
        };
        let request = pending.request;
        let kind = request["type"].as_str().unwrap_or("");
        if kind == "steer" {
            let original = &request["original"];
            match record["result"]["status"].as_str() {
                Some("accepted") if record["error"].is_null() => {
                    self.reply(original, json!({}));
                }
                Some("turn_settled") if record["error"].is_null() => {
                    self.events.push(json!({"type":"response","id":original["id"],"command":"prompt",
                        "success":false,"queueInstead":true,"error":"The ACP turn settled before steering admission."}));
                }
                Some("rejected")
                    if record["error"].is_null() && record["result"]["reason"].is_string() =>
                {
                    self.reject(
                        original,
                        format!(
                            "ACP steering rejected. {}",
                            record["result"]["reason"].as_str().unwrap()
                        ),
                    );
                }
                _ => self.uncertain_steer(original),
            }
            return Ok(());
        }
        if !record["error"].is_null() {
            let error = &record["error"];
            if kind == "start"
                && !self.start.metadata_only
                && error["code"] == -32602
                && self.capabilities["mcpCapabilities"]["http"] == true
                && self.capabilities["mcpCapabilities"]["stdio"] != false
                && !self.stdio_fallback
            {
                // A rejected transport has not created a session. Retry once with v1 stdio,
                // never on an authentication failure or after an accepted prompt.
                self.stdio_fallback = true;
                let mut params = request["params"].clone();
                params["mcpServers"] = self.mcp_servers()?;
                let mut retry = request.clone();
                retry["params"] = params.clone();
                self.request(request["method"].as_str().unwrap(), params, retry);
                return Ok(());
            }
            if kind == "antigravity_gateway" {
                // KSG: some gateway families advertise their default tier without the
                // "-medium" suffix; select the family the agent does advertise.
                let route = request["route"].as_str().unwrap_or("");
                let row = request["row"].as_str().unwrap_or("");
                let values: Vec<&str> = crate::antigravity::model_options(&self.config_options)
                    .into_iter()
                    .filter_map(|o| o["value"].as_str())
                    .collect();
                if error["code"] == -32602
                    && error["message"].as_str()
                        == Some(format!("Unknown model ID: {route}").as_str())
                    && route == format!("{row}-medium")
                    && !values.contains(&route)
                    && values.contains(&row)
                    && let Some(config) = crate::antigravity::model_config(&self.config_options)
                {
                    let config_id = config["id"].as_str().unwrap_or("").to_owned();
                    let mut retry = request.clone();
                    retry["route"] = json!(row);
                    self.request(
                        "session/set_config_option",
                        json!({"sessionId":self.session,"configId":config_id,"value":row}),
                        retry,
                    );
                    return Ok(());
                }
            }
            let message = error["message"].as_str().unwrap_or("Unknown ACP error");
            let message = if let Some(host) = &self.start.link {
                message.replace(&host.token, "[redacted]")
            } else {
                message.to_owned()
            };
            // Never authenticate or retry after an auth denial.
            let message = if self.antigravity
                && kind == "authenticate"
                && crate::antigravity::is_sign_in_required_error(
                    error["code"].as_i64().unwrap_or(0),
                    &message,
                ) {
                crate::antigravity::SIGN_IN_REQUIRED.to_owned()
            } else {
                format!(
                    "ACP {kind} failed: {message}. If authentication is required, configure or sign in to the agent outside Shina, then retry."
                )
            };
            if kind == "authenticate" {
                return Err(message);
            }
            if matches!(kind, "initialize" | "start") {
                return Err(message);
            }
            if kind == "prompt" {
                // The request may already have run tools. Do not restore/replay accepted input.
                return Err(message);
            } else if kind == "antigravity_gateway" {
                self.reject(&request["original"], message);
            } else {
                self.reject(&request, message);
            }
            return Ok(());
        }
        let result = &record["result"];
        match kind {
            "initialize" => {
                if result["protocolVersion"] != 1 {
                    return Err("This ACP agent did not negotiate protocol version 1. ACP v2 is not supported.".into());
                }
                self.capabilities = result["agentCapabilities"].clone();
                self.initialize_result = result.clone();
                self.active_steering = supports_active_steering(&result["_meta"]);
                self.events
                    .push(json!({"type":"harness_update","field":"turnCapabilities",
                    "value":self.turn_capabilities()}));
                // KSG authenticateEagerly with the resolved authMethodId.
                if self.antigravity {
                    let method = if self.antigravity_gateway {
                        "gateway"
                    } else {
                        "oauth-personal"
                    };
                    self.request(
                        "authenticate",
                        json!({"methodId":method}),
                        json!({"type":"authenticate"}),
                    );
                    return Ok(());
                }
                self.start_session()?;
            }
            "authenticate" => self.start_session()?,
            "start" => {
                if let Some(session) = result["sessionId"].as_str() {
                    self.session = Some(session.into());
                }
                if self.session.as_deref().is_none_or(str::is_empty) {
                    return Err("ACP agent did not return a session id.".into());
                }
                // The persisted per-session marker avoids repeating context after resume.
                self.finish_replay();
                self.configure_models(result);
                if let Some(modes) = result.get("modes") {
                    self.modes = modes.clone();
                }
                self.ready = true;
                // KSG sessionModeForPolicy: the thread's runtime policy becomes the
                // session mode at setup; the agy CLI maps yolo/auto_edit to default.
                if self.antigravity && !self.start.metadata_only {
                    let access = self
                        .start
                        .link
                        .as_ref()
                        .map(|h| h.access_mode.as_str())
                        .unwrap_or("");
                    let policy = crate::antigravity::permission_mode(access).to_owned();
                    self.antigravity_set_mode(&policy, json!({"type":"antigravity_mode"}));
                }
                self.reply(
                    &json!({"id":"models","type":"get_available_models"}),
                    json!({"models":self.models}),
                );
                self.reply(
                    &json!({"id":"levels","type":"get_available_thinking_levels"}),
                    json!({"levels":[]}),
                );
                self.reply(&json!({"id":"state","type":"get_state"}), self.state());
                self.reply(
                    &json!({"id":"history","type":"get_messages"}),
                    self.history_value(),
                );
                self.reply(
                    &json!({"id":"commands","type":"get_commands"}),
                    json!({"commands":self.available_commands()}),
                );
                if let Some(modes) = result.get("modes") {
                    self.events
                        .push(json!({"type":"harness_update","field":"modes","value":modes}));
                }
            }
            "prompt" => {
                if let Some(error) = result.get("error").filter(|e| !e.is_null()) {
                    return Err(error["message"]
                        .as_str()
                        .unwrap_or("ACP prompt failed")
                        .to_owned());
                }
                self.settle(result["stopReason"] == "cancelled");
            }
            "set_model" => {
                if result["configOptions"].is_array() {
                    self.configure_models(result);
                } else {
                    self.model = self
                        .models
                        .iter()
                        .find(|model| model["id"] == request["modelId"])
                        .cloned()
                        .unwrap_or_else(|| {
                            self.model_value(request["modelId"].as_str().unwrap_or("default"), None)
                        });
                }
                // KSG applyAntigravityAcpModelSelection: thinking levels vary by family, so
                // read them after the family switch and apply the level second.
                if self.antigravity
                    && let Some((original, combined, level)) = self.pending_thinking.take()
                {
                    let levels = crate::antigravity::thinking_config(&self.config_options);
                    let offered = levels
                        .map(|c| {
                            crate::antigravity::select_values(c)
                                .into_iter()
                                .any(|e| e["value"] == level)
                        })
                        .unwrap_or(false);
                    if let (Some(levels), true) = (levels, offered) {
                        let config_id = levels["id"].as_str().unwrap_or("").to_owned();
                        self.request(
                            "session/set_config_option",
                            json!({"sessionId":self.session,"configId":config_id,"value":level}),
                            json!({"type":"antigravity_level","original":original,"combined":combined}),
                        );
                        return Ok(());
                    }
                    self.reject(
                        &original,
                        format!("Antigravity model '{combined}' is unavailable. Select an available model."),
                    );
                    return Ok(());
                }
                self.reply(&request, self.model.clone());
            }
            // KSG applyAntigravityAcpModelSelection gateway select: the resolved route
            // lands in the model config option; the caller's pick replies on success.
            "antigravity_gateway" => {
                if result["configOptions"].is_array() {
                    self.configure_models(result);
                } else {
                    // A bare set_model reply carries no configOptions; mark the row
                    // containing the resolved route current.
                    let route = request["route"].as_str().unwrap_or("");
                    let selections =
                        crate::antigravity::gateway_model_selections(&self.gateway_models);
                    let rows = crate::antigravity::gateway_model_rows(
                        &selections,
                        &self.gateway_entries(),
                    );
                    if let Some(row) = rows
                        .iter()
                        .find(|row| row.slug == route || row.routes.iter().any(|(v, _)| v == route))
                        && let Some(model) = self.models.iter().find(|m| m["id"] == row.slug)
                    {
                        self.model = model.clone();
                    }
                }
                let original = request["original"].clone();
                if original["type"] == "set_option" {
                    self.reply(&original, result.clone());
                } else {
                    self.reply(&original, self.model.clone());
                }
            }
            "antigravity_mode" => {
                if result["configOptions"].is_array() {
                    self.configure_models(result);
                }
                self.note_modes(result);
                if self.modes.is_object() {
                    self.events
                        .push(json!({"type":"harness_update","field":"modes","value":self.modes}));
                }
            }
            "antigravity_level" => {
                let original = request["original"].clone();
                if result["configOptions"].is_array() {
                    self.configure_models(result);
                }
                self.pending_thinking = None;
                let combined = request["combined"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| crate::antigravity::current_model(&self.config_options));
                self.model = self
                    .models
                    .iter()
                    .find(|m| m["id"].as_str() == combined.as_deref())
                    .cloned()
                    .unwrap_or_else(|| {
                        self.model_value(combined.as_deref().unwrap_or("default"), None)
                    });
                self.reply(&original, self.model.clone());
            }
            "set_option" | "set_thinking_level" => {
                if result["configOptions"].is_array() {
                    self.configure_models(result);
                }
                self.reply(&request, result.clone());
            }
            _ => self.reply(&request, result.clone()),
        }
        Ok(())
    }

    /// Session creation, extracted so Antigravity can authenticate first. KSG
    /// `antigravitySessionMeta` rides along as `_meta`; Antigravity prefers session/resume.
    fn start_session(&mut self) -> Result<(), String> {
        let mut params = json!({"cwd":self.start.workspace,"mcpServers":self.mcp_servers()?});
        let method = match &self.start.source {
            SessionSource::New => "session/new",
            SessionSource::Resume(session) => {
                params["sessionId"] = json!(session);
                self.session = Some(session.clone());
                if self.antigravity
                    && self.capabilities["sessionCapabilities"]
                        .get("resume")
                        .is_some_and(|v| !v.is_null())
                {
                    "session/resume"
                } else if self.capabilities["loadSession"] == true {
                    "session/load"
                } else if self.capabilities["sessionCapabilities"]
                    .get("resume")
                    .is_some_and(|v| !v.is_null())
                {
                    "session/resume"
                } else {
                    return Err("This ACP agent does not advertise session/load or session/resume. Start a new thread instead.".into());
                }
            }
            SessionSource::Fork(_) => return Err("ACP forks are unsupported.".into()),
        };
        if self.antigravity
            && !self.start.metadata_only
            && let Some(meta) = crate::antigravity::session_meta(
                &self.initialize_result,
                Some(crate::mcp::INSTRUCTIONS),
            )
        {
            params["_meta"] = meta;
        }
        self.request(
            method,
            params.clone(),
            json!({"type":"start","method":method,"params":params}),
        );
        Ok(())
    }

    /// KSG `AcpSessionRuntime.setMode`: a mode pick goes through the session's
    /// `category: "mode"` config option when one exists, else `session/set_mode`
    /// when the agent negotiated `modes`. The real `agy` CLI wraps the pick in
    /// `antigravityCliSessionMode`; short-circuits when the mode already matches.
    fn antigravity_set_mode(&mut self, mode_id: &str, request: Value) {
        let mode_id = if self.antigravity_cli {
            crate::antigravity::cli_session_mode(mode_id)
        } else {
            mode_id
        };
        if self.modes["currentModeId"].as_str() == Some(mode_id) {
            self.reply(&request, json!({"currentModeId":mode_id}));
            return;
        }
        if let Some(config_id) = crate::antigravity::mode_config(&self.config_options)
            .and_then(|o| o["id"].as_str().map(str::to_owned))
        {
            self.request(
                "session/set_config_option",
                json!({"sessionId":self.session,"configId":config_id,"value":mode_id}),
                request,
            );
        } else if self.modes.is_object() {
            self.request(
                "session/set_mode",
                json!({"sessionId":self.session,"modeId":mode_id}),
                request,
            );
        } else {
            self.reject(&request, "This ACP agent did not negotiate session modes.");
        }
    }

    /// KSG `withinCeiling` + `antigravityRuntimeModeForSessionMode`: under a managed
    /// policy ceiling, a mode pick applies only when its granted runtime mode is
    /// known and stays at or under the ceiling.
    fn antigravity_mode_denial(&self, value: &str) -> Option<String> {
        let policy = crate::model::pi_session::read_managed_policy()?;
        let ceiling = policy.max_access?;
        match crate::antigravity::runtime_mode_for_session_mode(value)
            .and_then(crate::model::AccessMode::parse)
        {
            Some(mode) => crate::model::pi_session::managed_access_mode_denial(Some(&policy), mode),
            None => Some(format!(
                "{} limits threads to {}.",
                policy.organization,
                ceiling.label()
            )),
        }
    }

    /// KSG `updateCurrentModeId` + `parseSessionModeState`: track the current mode
    /// from a mode reply or the mode-category config option's currentValue.
    fn note_modes(&mut self, result: &Value) {
        if let Some(modes) = result.get("modes") {
            self.modes = modes.clone();
        }
        let current = result["currentModeId"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                crate::antigravity::mode_config(&self.config_options)
                    .and_then(|o| o["currentValue"].as_str().map(str::to_owned))
            });
        if let Some(current) = current {
            if self.modes.is_object() {
                self.modes["currentModeId"] = json!(current);
            } else {
                self.modes = json!({"currentModeId":current,"availableModes":[]});
            }
        }
    }

    fn model_value(&self, id: &str, name: Option<&str>) -> Value {
        let input = if self.capabilities["promptCapabilities"]["image"] == true {
            vec!["text", "image"]
        } else {
            vec!["text"]
        };
        let thinking_levels: Vec<String> = self
            .option_descriptors
            .iter()
            .find(|d| crate::descriptors::is_reasoning_control(d))
            .map(|d| {
                crate::descriptors::selectable_provider_option_choices(d)
                    .into_iter()
                    .map(|c| c.id)
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "id": id,
            "name": name.unwrap_or(id),
            "provider": self.start.id,
            "input": input,
            "thinkingLevels": thinking_levels,
            "capabilities": { "optionDescriptors": self.option_descriptors }
        })
    }

    fn configure_models(&mut self, result: &Value) {
        self.option_descriptors = crate::descriptors::acp_provider_option_descriptors(
            result["configOptions"].as_array().map(Vec::as_slice),
            result.get("modes"),
        );
        self.config_options = result["configOptions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // KSG observeThinkingLevels + buildAntigravityModelsFromSession.
        if self.antigravity {
            crate::antigravity::observe_thinking_levels(
                &self.config_options,
                &mut self.thinking_levels,
            );
            if result["models"].is_object() {
                self.legacy_models = result["models"].clone();
            }
            let (mut rows, current) = crate::antigravity::session_models(
                result,
                &self.thinking_levels,
                &self.gateway_models,
            );
            crate::antigravity::apply_manifest(&mut rows);
            self.model_option = crate::antigravity::model_config(&self.config_options)
                .and_then(|c| c["id"].as_str().map(str::to_owned));
            self.models = rows
                .iter()
                .map(|row| self.antigravity_model_value(row))
                .collect();
            self.model = self
                .models
                .iter()
                .find(|m| m["id"].as_str() == current.as_deref())
                .cloned()
                .unwrap_or_else(|| self.model_value(current.as_deref().unwrap_or("default"), None));
            return;
        }
        if let Some(options) = result["configOptions"].as_array()
            && let Some(option) = options
                .iter()
                .find(|o| o["category"] == "model" && o["type"] == "select")
        {
            self.model_option = option["id"].as_str().map(str::to_owned);
            let mut choices = vec![];
            for group in option["options"].as_array().into_iter().flatten() {
                if let Some(options) = group["options"].as_array() {
                    choices.extend(options.iter());
                } else {
                    choices.push(group);
                }
            }
            self.models = choices
                .into_iter()
                .filter_map(|choice| {
                    Some(self.model_value(choice["value"].as_str()?, choice["name"].as_str()))
                })
                .collect();
            self.model =
                self.model_value(option["currentValue"].as_str().unwrap_or("default"), None);
        } else {
            self.models = result["models"]["availableModels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|model| {
                    Some(self.model_value(model["modelId"].as_str()?, model["name"].as_str()))
                })
                .collect();
            self.model = self.model_value(
                result["models"]["currentModelId"]
                    .as_str()
                    .unwrap_or("default"),
                None,
            );
        }
    }

    /// KSG `thinkingCapabilities`: per-model reasoning descriptor, current value
    /// stripped unless this model was observed with it.
    fn antigravity_model_value(&self, row: &Value) -> Value {
        let mut value = self.model_value(
            row["slug"].as_str().unwrap_or("default"),
            row["name"].as_str(),
        );
        value["isDefault"] = row["isDefault"].clone();
        value["aliases"] = row["aliases"].clone();
        value["isLegacy"] = row["isLegacy"].clone();
        if let Some(efforts) = row["efforts"].as_array() {
            // KSG buildSelectOptionDescriptor(ANTIGRAVITY_GATEWAY_EFFORT_OPTION_ID):
            // the adapter owns the pick and routes it through the model config option.
            let descriptor = crate::descriptors::ProviderOptionDescriptor::Select {
                id: crate::antigravity::GATEWAY_EFFORT_OPTION_ID.into(),
                label: "Reasoning".into(),
                description: None,
                options: efforts
                    .iter()
                    .map(|e| crate::descriptors::ProviderOptionChoice {
                        id: e["id"].as_str().unwrap_or_default().to_owned(),
                        label: e["label"].as_str().unwrap_or_default().to_owned(),
                        description: None,
                        is_default: e["isDefault"] == true,
                    })
                    .collect(),
                current_value: efforts
                    .iter()
                    .find(|e| e["isDefault"] == true)
                    .and_then(|e| e["id"].as_str())
                    .map(str::to_owned),
                prompt_injected_values: vec![],
                role: None,
            };
            value["thinkingLevels"] = json!(
                efforts
                    .iter()
                    .filter_map(|e| e["id"].as_str())
                    .collect::<Vec<_>>()
            );
            value["capabilities"] = json!({"optionDescriptors":[descriptor]});
        } else if row["thinking"].is_object() {
            let descriptors = crate::descriptors::acp_provider_option_descriptors(
                Some(std::slice::from_ref(&row["thinking"])),
                None,
            );
            if let Some(descriptor) = descriptors.into_iter().next() {
                let levels = crate::descriptors::selectable_provider_option_choices(&descriptor)
                    .into_iter()
                    .map(|c| c.id)
                    .collect::<Vec<_>>();
                value["thinkingLevels"] = json!(levels);
                value["capabilities"] = json!({"optionDescriptors":[descriptor]});
            }
        }
        value
    }

    /// KSG `[...antigravityModelOptions, ...legacyModels?.availableModels]`, the entry
    /// list `applyAntigravityAcpModelSelection` groups gateway rows over.
    fn gateway_entries(&self) -> Vec<(String, String)> {
        crate::antigravity::model_options(&self.config_options)
            .into_iter()
            .filter_map(|e| {
                Some((
                    e["value"].as_str()?.to_owned(),
                    e["name"].as_str().unwrap_or("").to_owned(),
                ))
            })
            .chain(
                self.legacy_models["availableModels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| {
                        Some((
                            m["modelId"].as_str()?.to_owned(),
                            m["name"].as_str().unwrap_or("").to_owned(),
                        ))
                    }),
            )
            .collect()
    }

    /// KSG applyAntigravityAcpModelSelection, gateway branch: resolve a pick (or the
    /// stored effort pick) to a route and select it through the model config option.
    /// Returns true when the record was handled.
    fn antigravity_gateway_select(&mut self, requested: Option<&str>, record: &Value) -> bool {
        let selections = crate::antigravity::gateway_model_selections(&self.gateway_models);
        if selections.is_empty() {
            return false;
        }
        let rows = crate::antigravity::gateway_model_rows(&selections, &self.gateway_entries());
        let current = crate::antigravity::current_model(&self.config_options).or_else(|| {
            self.legacy_models["currentModelId"]
                .as_str()
                .map(str::to_owned)
        });
        let row = match requested {
            Some(id) => rows
                .iter()
                .find(|row| row.slug == id || row.routes.iter().any(|(v, _)| v == id)),
            None => current
                .as_deref()
                .and_then(|current| {
                    rows.iter().find(|row| {
                        row.slug == current || row.routes.iter().any(|(v, _)| v == current)
                    })
                })
                .or_else(|| rows.first()),
        };
        let Some(row) = row else {
            return if requested.is_none()
                || self.gateway_effort.is_some()
                || requested.is_some_and(|id| {
                    crate::antigravity::has_configured_gateway_model(&selections, id)
                }) {
                self.reject(
                    record,
                    format!(
                        "Antigravity model '{}' is unavailable. Select an available model.",
                        requested.unwrap_or(crate::antigravity::DEFAULT_MODEL)
                    ),
                );
                true
            } else {
                false
            };
        };
        let effort = self.gateway_effort.as_deref();
        let exact = requested.filter(|id| *id != row.slug).and_then(|id| {
            row.routes
                .iter()
                .find(|(v, _)| v == id)
                .map(|(v, _)| v.clone())
        });
        let route = match effort {
            Some(effort) => row
                .efforts
                .iter()
                .find(|e| e.value == effort)
                .map(|e| e.route.clone()),
            None => Some(exact.unwrap_or_else(|| row.default_route.clone())),
        };
        let Some(route) = route else {
            self.reject(
                record,
                format!(
                    "Antigravity model '{} ({})' is unavailable. Select an available model.",
                    row.slug,
                    effort.unwrap_or("")
                ),
            );
            return true;
        };
        let pending =
            json!({"type":"antigravity_gateway","original":record,"route":route,"row":row.slug});
        if let Some(config_id) = crate::antigravity::model_config(&self.config_options)
            .and_then(|c| c["id"].as_str().map(str::to_owned))
        {
            // KSG allowUnlistedValue: a gateway route need not be an advertised value.
            self.request(
                "session/set_config_option",
                json!({"sessionId":self.session,"configId":config_id,"value":route}),
                pending,
            );
        } else if Some(route.as_str()) != current.as_deref() {
            self.request(
                "session/set_model",
                json!({"sessionId":self.session,"modelId":route}),
                pending,
            );
        } else {
            // KSG returns without sending when the resolved route is already current.
            self.reply(record, self.model.clone());
        }
        true
    }

    fn command(&mut self, record: Value) -> Result<(), String> {
        let kind = record["type"].as_str().unwrap_or("");
        if self.start.metadata_only && !matches!(kind, "get_available_models" | "get_commands") {
            self.reject(
                &record,
                "ACP catalog discovery only supports metadata requests.",
            );
            return Ok(());
        }
        if kind == "extension_ui_response" {
            let key = record["id"].as_str().unwrap_or("");
            if let Some(permission) = self.permissions.remove(key) {
                if let Some(question) = &permission.question {
                    let supplied: Value = record["value"]
                        .as_str()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(Value::Null);
                    let answer = &supplied[question["id"].as_str().unwrap_or("")];
                    let answer = answer.as_str().or_else(|| {
                        answer
                            .as_array()
                            .filter(|a| a.len() == 1)?
                            .first()?
                            .as_str()
                    });
                    let options = question["options"].as_array().expect("validated");
                    let option = if record["cancelled"] == true {
                        None
                    } else {
                        answer.and_then(|answer| {
                            options
                                .iter()
                                .find(|o| o["value"] == answer)
                                .or_else(|| {
                                    let matching: Vec<_> =
                                        options.iter().filter(|o| o["label"] == answer).collect();
                                    (matching.len() == 1).then(|| matching[0])
                                })
                                .and_then(|o| o["value"].as_str())
                                .map(str::to_owned)
                        })
                    };
                    self.wire.push(permission_response(permission.id, option));
                    return Ok(());
                }
                let decision = crate::approval::decision(&record)?;
                let decision = if decision == "acceptForSession"
                    && permission
                        .offered
                        .as_ref()
                        .is_some_and(|options| !options.iter().any(|o| o.decision == decision))
                {
                    "accept"
                } else {
                    decision
                };
                let option = match decision {
                    "accept" => permission.allow,
                    "acceptForSession" => permission.session.or(permission.allow),
                    "decline" => permission.reject,
                    _ => None,
                };
                self.wire.push(permission_response(permission.id, option));
            }
            return Ok(());
        }
        if !self.ready {
            return Err("ACP session is not initialized.".into());
        }
        match kind {
            "get_state" => self.reply(&record, self.state()),
            "get_entries" | "get_messages" => {
                let mut request = record.clone();
                request["type"] = json!("get_messages");
                self.reply(&request, self.history_value());
            }
            "get_available_models" => self.reply(&record, json!({"models":self.models})),
            "get_available_thinking_levels" => self.reply(&record, json!({"levels":[]})),
            "get_commands" => self.reply(&record, json!({"commands":self.available_commands()})),
            "get_session_stats" => self.reply(&record, json!({"contextUsage":null})),
            "set_session_name" => {
                // Shina owns the title. ACP v1 has no standard rename method.
                self.reply(&record, json!({}));
            }
            "clear_queue" => {
                let queued: Vec<_> = self.restart_prompt.take().into_iter().chain(self.queued.drain(..))
                    .filter_map(|record| record["message"].as_str().map(str::to_owned)).collect();
                self.reply(&record, json!({"steering":[],"followUp":queued}));
                self.queue_update();
            }
            "abort" => {
                for (_, permission) in self.permissions.drain() {
                    self.wire.push(permission_response(permission.id, None));
                }
                self.cancelling = self.running;
                self.wire.push(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":self.session}}));
                self.reply(&record, json!({}));
                if !self.running { self.events.push(json!({"type":"agent_settled","aborted":true})); }
            }
            "prompt" if self.running => {
                if record["streamingBehavior"] == "steer" && self.active_steering && !self.cancelling {
                    self.steer(record)?;
                } else if record["streamingBehavior"] == "steer" && self.restart_steering && !self.cancelling {
                    // KSG's interrupt_restart waits for cancellation before the next attempt.
                    self.restart_prompt = Some(record.clone());
                    self.cancelling = true;
                    self.wire.push(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":self.session}}));
                    self.reply(&record, json!({"disposition":"queued"}));
                    self.queue_update();
                } else if self.queued.len() >= 32 {
                    self.reject(&record, "ACP follow-up queue is full.");
                } else {
                    self.queued.push_back(record.clone());
                    self.reply(&record, json!({"disposition":"queued"}));
                    self.queue_update();
                }
            }
            "prompt" => self.prompt(record)?,
            "set_model" => {
                if self.running {
                    self.reject(&record, "Wait for the ACP run to finish before changing its model.");
                } else if self.antigravity {
                    // KSG applyAntigravityAcpModelSelection.
                    let raw = record["modelId"].as_str().unwrap_or("");
                    let requested = (!raw.is_empty() && raw != crate::antigravity::DEFAULT_MODEL)
                        .then_some(raw);
                    // A new row's selection starts from its descriptor defaults, like
                    // KSG's freshly-built modelSelection options.
                    if requested != self.model["id"].as_str() {
                        self.gateway_effort = None;
                    }
                    if self.antigravity_gateway_select(requested, &record) {
                        return Ok(());
                    }
                    let resolved = crate::antigravity::resolve_model(
                        &self.config_options,
                        requested,
                        None,
                    );
                    let current = crate::antigravity::current_model(&self.config_options);
                    let Some(resolved) = resolved else {
                        self.reject(&record, format!("Antigravity model '{raw}' is unavailable. Select an available model."));
                        return Ok(());
                    };
                    if requested.is_none() && current.as_deref() == Some(resolved.as_str()) {
                        self.reply(&record, self.model.clone());
                    } else if let Some(option) = &self.model_option {
                        let split = crate::antigravity::split_model(&self.config_options, &resolved);
                        match split {
                            Some((family, None)) | Some((family, Some(_)))
                                if family == resolved =>
                            {
                                self.request("session/set_config_option", json!({"sessionId":self.session,"configId":option,"value":family}), record);
                            }
                            Some((family, Some(level))) => {
                                self.pending_thinking =
                                    Some((record.clone(), resolved.clone(), level));
                                self.request("session/set_config_option", json!({"sessionId":self.session,"configId":option,"value":family}), record);
                            }
                            _ if crate::antigravity::legacy_family(&self.config_options, &resolved)
                                .is_some()
                                && crate::antigravity::thinking_config(&self.config_options)
                                    .is_none() =>
                            {
                                // Legacy 1.3 combined id the agent still accepts.
                                self.request("session/set_config_option", json!({"sessionId":self.session,"configId":option,"value":resolved}), record);
                            }
                            _ => self.reject(&record, format!("Antigravity model '{resolved}' is unavailable. Select an available model.")),
                        }
                    } else {
                        self.request("session/set_model", json!({"sessionId":self.session,"modelId":resolved}), record);
                    }
                } else if !self.models.iter().any(|model| model["id"] == record["modelId"]) {
                    self.reject(&record, "This ACP agent did not advertise that model. Refresh the session or choose one of its advertised models.");
                } else if let Some(option) = &self.model_option {
                    self.request("session/set_config_option", json!({"sessionId":self.session,"configId":option,"value":record["modelId"]}), record);
                } else {
                    self.request("session/set_model", json!({"sessionId":self.session,"modelId":record["modelId"]}), record);
                }
            }
            "set_thinking_level" => {
                if self.running {
                    self.reject(&record, "Wait for the ACP run to finish before changing reasoning.");
                } else if let Some(reasoning) = self
                    .option_descriptors
                    .iter()
                    .find(|d| crate::descriptors::is_reasoning_control(d))
                {
                    let config_id = reasoning.id().to_owned();
                    let raw = record["level"].as_str().unwrap_or("");
                    let value = if raw == crate::descriptors::ACP_EMPTY_OPTION_VALUE {
                        ""
                    } else {
                        raw
                    };
                    self.request(
                        "session/set_config_option",
                        json!({"sessionId":self.session,"configId":config_id,"value":value}),
                        record,
                    );
                } else {
                    self.reject(&record, format!("ACP does not support the {kind} operation in Shina. Native compaction, reasoning selection and conversation forks are unavailable for ACP."));
                }
            }
            "set_option" => {
                if self.running {
                    self.reject(&record, "Wait for the ACP run to finish before changing options.");
                } else if let Some(option_id) = record["optionId"].as_str() {
                    if self.antigravity
                        && option_id == crate::antigravity::GATEWAY_EFFORT_OPTION_ID
                        && !crate::antigravity::gateway_model_selections(&self.gateway_models)
                            .is_empty()
                    {
                        // KSG ownedModelOptionIds: the effort pick lives in the model
                        // selection and re-routes the current family through the
                        // model config option instead of a reasoningEffort config.
                        self.gateway_effort = record["value"]
                            .as_str()
                            .filter(|v| {
                                !v.is_empty() && *v != crate::descriptors::ACP_EMPTY_OPTION_VALUE
                            })
                            .map(str::to_owned);
                        let current_slug = self.model["id"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                        if !self.antigravity_gateway_select(current_slug.as_deref(), &record) {
                            self.reject(
                                &record,
                                "Antigravity model is unavailable. Select an available model.",
                            );
                        }
                    } else if self.antigravity
                        && crate::antigravity::is_mode_choice(&self.config_options, option_id)
                        && let Some(denial) = self
                            .antigravity_mode_denial(record["value"].as_str().unwrap_or(""))
                    {
                        self.reject(&record, denial);
                    } else if option_id == crate::descriptors::ACP_SESSION_MODE_OPTION_ID {
                        let mode_id = record["value"]
                            .as_str()
                            .map(|v| if v == crate::descriptors::ACP_EMPTY_OPTION_VALUE { "" } else { v })
                            .unwrap_or("")
                            .to_owned();
                        if self.antigravity {
                            self.antigravity_set_mode(&mode_id, record);
                        } else {
                            self.request(
                                "session/set_mode",
                                json!({"sessionId":self.session,"modeId":mode_id}),
                                record,
                            );
                        }
                    } else {
                        let value = match record["value"].as_str() {
                            Some(v) if v == crate::descriptors::ACP_EMPTY_OPTION_VALUE => json!(""),
                            _ => record["value"].clone(),
                        };
                        self.request(
                            "session/set_config_option",
                            json!({"sessionId":self.session,"configId":option_id,"value":value}),
                            record,
                        );
                    }
                }
            }
            _ => self.reject(&record, format!("ACP does not support the {kind} operation in Shina. Native compaction, reasoning selection and conversation forks are unavailable for ACP.")),
        }
        Ok(())
    }

    fn prompt(&mut self, record: Value) -> Result<(), String> {
        let images = record["images"].as_array().cloned().unwrap_or_default();
        if !images.is_empty() && self.capabilities["promptCapabilities"]["image"] != true {
            self.reject(&record, "This ACP agent did not negotiate image input.");
            return Ok(());
        }
        let text = record["message"].as_str().unwrap_or("");
        let mut content = vec![];
        if !self.awareness_sent {
            content.push(json!({"type":"text","text":format!(
                "<shina_context>\n{}\n{}\n</shina_context>",
                self.start.shina, self.start.instructions.as_deref().unwrap_or("")
            )}));
        }
        let dispatch = crate::composer_skills::claude_dispatch(text, &self.claude_skills);
        let mut command_text = None;
        if !text.is_empty() {
            if dispatch.len() > 1 || dispatch[0] != text {
                if dispatch.len() > 1 {
                    content.push(json!({"type":"text","text":dispatch[0]}));
                }
                command_text = dispatch.last().cloned();
            } else {
                content.push(json!({"type":"text","text":dispatch[0]}));
            }
        }
        let files: Vec<_> = record["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|value| {
                crate::attachments::Attachment::from_value(value)
                    .ok_or_else(|| "Invalid ACP attachment reference.".to_owned())
            })
            .collect::<Result<_, _>>()?;
        let parts =
            crate::attachments::acp_parts(&files, &self.capabilities["promptCapabilities"])?;
        content.extend(parts.clone());
        content.extend(images.clone());
        if let Some(command) = command_text {
            content.push(json!({"type":"text","text":command}));
        }
        self.awareness_sent = true;
        self.events
            .push(json!({"type":"harness_update","field":"awareness","value":true}));
        self.running = true;
        self.cancelling = false;
        let mut history_content = vec![json!({"type":"text","text":text})];
        history_content.extend(parts);
        history_content.extend(images);
        self.history
            .push(json!({"role":"user","content":history_content}));
        self.events.push(
            json!({"type":"message_start","message":{"role":"user","content":history_content}}),
        );
        self.events.push(json!({"type":"agent_start"}));
        let turn = uuid::Uuid::new_v4().to_string();
        self.turn_id = Some(turn.clone());
        let mut params = json!({"sessionId":self.session,"prompt":content});
        if self.active_steering {
            params["_meta"] = json!({"t3code/turnId":turn});
        }
        self.request("session/prompt", params, record.clone());
        // Like Pi, acknowledge dispatch immediately, not when the full run finishes.
        self.reply(&record, json!({}));
        Ok(())
    }

    fn steer(&mut self, record: Value) -> Result<(), String> {
        let Some(turn) = self.turn_id.clone() else {
            self.reject(&record, "The ACP turn is no longer active.");
            return Ok(());
        };
        let images = record["images"].as_array().cloned().unwrap_or_default();
        if !images.is_empty() && self.capabilities["promptCapabilities"]["image"] != true {
            self.reject(&record, "This ACP agent did not negotiate image input.");
            return Ok(());
        }
        let mut prompt = vec![json!({"type":"text","text":record["message"]})];
        let files: Vec<_> = record["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|value| {
                crate::attachments::Attachment::from_value(value)
                    .ok_or("Invalid ACP attachment reference.")
            })
            .collect::<Result<_, _>>()?;
        prompt.extend(crate::attachments::acp_parts(
            &files,
            &self.capabilities["promptCapabilities"],
        )?);
        prompt.extend(images);
        self.request(
            "_t3/steer",
            json!({"sessionId":self.session,"turnId":turn,
            "messageId":record["id"],"prompt":prompt}),
            json!({"type":"steer","original":record}),
        );
        Ok(())
    }

    fn uncertain_steer(&mut self, record: &Value) {
        // No refusal guarantee: never queue or restore a possibly admitted message.
        self.reply(record, json!({"deliveryUnknown":true}));
        self.events.push(
            json!({"type":"steer_delivery_unknown","text":record["message"],
            "message":{"content":record["images"]},"attachments":record["attachments"]}),
        );
    }

    fn settle(&mut self, aborted: bool) {
        self.running = false;
        self.turn_id = None;
        let aborted = aborted || self.cancelling;
        if aborted
            && let Some(message) = self
                .history
                .iter_mut()
                .rev()
                .find(|m| m["role"] == "assistant")
        {
            message["stopReason"] = json!("aborted");
        }
        self.cancelling = false;
        if let Some(record) = self.restart_prompt.take() {
            self.events.push(json!({"type":"steering_restart"}));
            self.queue_update();
            if let Err(error) = self.prompt(record.clone()) {
                self.reject(&record, error);
            }
            return;
        }
        self.events
            .push(json!({"type":"agent_settled","aborted":aborted}));
        // Dispatch on the worker after all settlement events, before accepting newer input.
        if !aborted && let Some(record) = self.queued.pop_front() {
            self.queue_update();
            if let Err(error) = self.prompt(record) {
                self.reject(&json!({"type":"prompt"}), error);
            }
        }
    }

    fn queue_update(&mut self) {
        let texts: Vec<_> = self
            .restart_prompt
            .iter()
            .chain(self.queued.iter())
            .map(|record| record["message"].clone())
            .collect();
        self.events
            .push(json!({"type":"queue_update","steering":[],"followUp":texts}));
    }

    /// KSG makeAcpClientTerminals dispatch. `wait_for_exit` defers until the watcher
    /// thread records an exit status; the loop flushes waiters on every pass.
    fn terminal_request(&mut self, method: &str, record: &Value) {
        let id = record["id"].clone();
        let params = record["params"].clone();
        let (result, deferred) = match method {
            "terminal/create" => (
                self.terminals
                    .create(&params, &self.start.workspace)
                    .map(|terminal| json!({"terminalId":terminal})),
                false,
            ),
            "terminal/output" => (self.terminals.output(&params), false),
            "terminal/kill" => (self.terminals.kill(&params).map(|()| json!({})), false),
            "terminal/release" => (self.terminals.release(&params).map(|()| json!({})), false),
            "terminal/wait_for_exit" => match self.terminals.exit_status(&params) {
                Ok(Some(status)) => (Ok(status), false),
                Ok(None) => (Ok(Value::Null), params["terminalId"].is_string()),
                Err(error) => (Err(error), false),
            },
            _ => (
                Err(format!(
                    "Shina does not support ACP client capability {method}."
                )),
                false,
            ),
        };
        if deferred {
            self.terminal_waiters.push(TerminalWaiter {
                id,
                terminal: params["terminalId"].as_str().unwrap().to_owned(),
            });
            return;
        }
        match result {
            Ok(result) => self
                .wire
                .push(json!({"jsonrpc":"2.0","id":id,"result":result})),
            Err(message) => self
                .wire
                .push(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":message}})),
        }
    }

    /// Answer every `terminal/wait_for_exit` whose child has recorded an exit status.
    fn flush_terminal_waiters(&mut self) {
        let mut index = 0;
        while index < self.terminal_waiters.len() {
            let Some(status) = self
                .terminals
                .exit_status_of(&self.terminal_waiters[index].terminal)
            else {
                index += 1;
                continue;
            };
            let waiter = self.terminal_waiters.remove(index);
            self.wire
                .push(json!({"jsonrpc":"2.0","id":waiter.id,"result":status}));
        }
    }

    fn permission(&mut self, record: &Value) -> Result<(), String> {
        if self.start.metadata_only {
            self.wire
                .push(permission_response(record["id"].clone(), None));
            return Ok(());
        }
        let params = &record["params"];
        if self
            .session
            .as_deref()
            .is_some_and(|session| params["sessionId"] != session)
            && !(self.devin
                && params["sessionId"]
                    .as_str()
                    .is_some_and(|session| self.devin_children.contains(session)))
        {
            return Err("ACP permission request belongs to another session.".into());
        }
        let options = params["options"]
            .as_array()
            .ok_or("ACP permission request has no options.")?;
        // KSG devinAnswerablePermissionOptions: `plan_bypass` is filtered before any
        // automatic or supervised selection can send its id back to Devin.
        let filtered;
        let options: &[Value] = if self.devin {
            filtered = crate::devin::answerable_permission_options(params);
            &filtered
        } else {
            options
        };
        let interaction = self.start.id.contains("antigravity")
            && params["toolCall"]["toolCallId"]
                .as_str()
                .is_some_and(|id| id.starts_with("interaction_"));
        let question = interaction.then(|| antigravity_question(params)).flatten();
        if interaction && question.is_none() {
            self.wire
                .push(permission_response(record["id"].clone(), None));
            return Ok(());
        }
        let option = |kinds: &[&str]| -> Option<String> {
            kinds.iter().find_map(|kind| {
                options
                    .iter()
                    .find(|o| o["kind"] == *kind)
                    .and_then(|o| o["optionId"].as_str())
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            })
        };
        let permission = Permission {
            id: record["id"].clone(),
            allow: option(&["allow_once"]),
            session: option(&["allow_always"]),
            reject: option(&["reject_once"]),
            offered: acp_approval_options(&self.start.id, params),
            question,
        };
        let tool = &params["toolCall"];
        let access = self
            .start
            .link
            .as_ref()
            .map(|h| h.access_mode.as_str())
            .unwrap_or("");
        let auto = (access == "full-access"
            || matches!(tool["kind"].as_str(), Some("read" | "search"))
            || access == "auto-accept-edits" && tool["kind"] == "edit")
            && (!self.start.id.contains("kiro") || permission.allow.is_some());
        if auto && !interaction || self.cancelling {
            self.wire.push(permission_response(
                permission.id,
                if self.cancelling {
                    None
                } else {
                    permission.allow.or(permission.session)
                },
            ));
        } else if self.permissions.len() >= 32 {
            self.wire.push(permission_response(permission.id, None));
        } else {
            self.serial += 1;
            let key = format!("acp-permission-{}", self.serial);
            let mut event = json!({"type":"extension_ui_request","id":key,"method":"confirm",
                "title":params["title"].as_str().or_else(||tool["title"].as_str()).unwrap_or("Allow ACP tool?"),
                "message":format!("{}\n{}",params["description"].as_str().unwrap_or(""),tool["rawInput"]),
            });
            if let Some(question) = &permission.question {
                event = json!({"type":"extension_ui_request","id":key,"method":"input",
                    "title":"Question","questions":[question],"responseCapability":{"type":"live"}});
            }
            if let Some(options) = &permission.offered {
                event["approvalOptions"] = json!(options);
            }
            self.events.push(event);
            self.permissions.insert(key, permission);
        }
        Ok(())
    }

    fn history_chunk(&mut self, role: &str, block: Value) {
        if self
            .history
            .last()
            .is_none_or(|message| message["role"] != role)
        {
            self.history.push(json!({"role":role,"content":[]}));
        }
        let content = self.history.last_mut().unwrap()["content"]
            .as_array_mut()
            .unwrap();
        let field = if block["type"] == "thinking" {
            "thinking"
        } else {
            "text"
        };
        if let Some(last) = content
            .last_mut()
            .filter(|last| last["type"] == block["type"])
            && let (Some(left), Some(right)) = (last[field].as_str(), block[field].as_str())
        {
            last[field] = json!(format!("{left}{right}"));
        } else {
            content.push(block);
        }
    }

    fn finish_replay(&mut self) {
        for message in &mut self.history {
            if message["role"] != "user" {
                continue;
            }
            if let Some(content) = message["content"].as_array_mut() {
                content.retain_mut(|block| {
                    let Some(text) = block["text"].as_str() else {
                        return true;
                    };
                    if let Some(context) = text.strip_prefix("<shina_context>\n")
                        && let Some((_, remainder)) = context.split_once("</shina_context>")
                    {
                        self.awareness_sent = true;
                        let remainder = remainder.to_owned();
                        if remainder.is_empty() {
                            return false;
                        }
                        block["text"] = json!(remainder);
                    }
                    true
                });
            }
        }
        self.history.retain(|message| {
            message["content"]
                .as_array()
                .is_none_or(|content| !content.is_empty())
        });
        if self.awareness_sent {
            self.events
                .push(json!({"type":"harness_update","field":"awareness","value":true}));
        }
    }

    fn update(&mut self, update: &Value) {
        // KSG normalizeAntigravitySessionUpdate bounds tool payload size.
        let normalized;
        let update = if self.antigravity {
            normalized = crate::antigravity::normalize_session_update(
                &json!({"sessionId":self.session,"update":update}),
            );
            &normalized["update"]
        } else {
            update
        };
        let emit = self.ready;
        match update["sessionUpdate"].as_str().unwrap_or("") {
            "agent_message_chunk" | "agent_thought_chunk" | "user_message_chunk" => {
                let thought = update["sessionUpdate"] == "agent_thought_chunk";
                let user = update["sessionUpdate"] == "user_message_chunk";
                let content = &update["content"];
                if content["type"] == "text" {
                    let text = content["text"].as_str().unwrap_or("");
                    if !emit
                        && user
                        && text.starts_with("<shina_context>\n")
                        && text.ends_with("</shina_context>")
                    {
                        self.awareness_sent = true;
                        self.events.push(
                            json!({"type":"harness_update","field":"awareness","value":true}),
                        );
                        return;
                    }
                    self.history_chunk(
                        if user { "user" } else { "assistant" },
                        if thought {
                            json!({"type":"thinking","thinking":text})
                        } else {
                            json!({"type":"text","text":text})
                        },
                    );
                    if emit && !user {
                        self.events.push(json!({"type":"message_update","assistantMessageEvent":{
                            "type":if thought {"thinking_delta"} else {"text_delta"},"delta":text,"contentIndex":0
                        }}));
                    }
                } else if !emit && user && content["type"] == "image" {
                    self.history_chunk("user", content.clone());
                } else if emit {
                    self.reject(
                        &json!({"type":"capability"}),
                        format!(
                            "Shina cannot display ACP {} message content.",
                            content["type"]
                        ),
                    );
                }
            }
            "tool_call" | "tool_call_update" | "tool_call_content_chunk" => {
                let Some(id) = update["toolCallId"].as_str() else {
                    return;
                };
                let new = !self.tools.contains_key(id);
                let chunk = update["sessionUpdate"] == "tool_call_content_chunk";
                let tool = self.tools.entry(id.into()).or_insert_with(|| json!({}));
                for (key, value) in update.as_object().into_iter().flatten() {
                    if value.is_null() || (chunk && key == "content") {
                        continue;
                    }
                    // Devin's _meta carries subagent lifecycle keys; a completion-only
                    // update must not erase the started keys the card still reads.
                    if self.devin
                        && key == "_meta"
                        && tool["_meta"].is_object()
                        && value.is_object()
                    {
                        for (meta_key, meta_value) in value.as_object().unwrap() {
                            tool["_meta"][meta_key] = meta_value.clone();
                        }
                    } else {
                        tool[key] = value.clone();
                    }
                }
                if chunk {
                    // KSG's reducer streams chunk text into the tool's content parts.
                    if !tool["content"].is_array() {
                        tool["content"] = json!([]);
                    }
                    let content = tool["content"].as_array_mut().unwrap();
                    let part = &update["content"];
                    if part["type"] == "content"
                        && part["content"]["type"] == "text"
                        && let Some(last) = content.last_mut().filter(|last| {
                            last["type"] == "content" && last["content"]["type"] == "text"
                        })
                    {
                        let merged = format!(
                            "{}{}",
                            last["content"]["text"].as_str().unwrap_or(""),
                            part["content"]["text"].as_str().unwrap_or("")
                        );
                        last["content"]["text"] = json!(merged);
                    } else {
                        content.push(part.clone());
                    }
                }
                let tool = tool.clone();
                // KSG normalizeDevinToolCall then extractDevinSubagentUpdate, both on
                // the merged state.
                let tool = if self.devin {
                    crate::devin::normalize_tool_call(&tool)
                } else {
                    tool
                };
                let subagent = if self.devin {
                    let subagent = crate::devin::extract_subagent_update(&tool);
                    if let Some(subagent) = &subagent {
                        self.devin_children
                            .insert(subagent.child_session_id.clone());
                    }
                    subagent
                } else {
                    None
                };
                let name = subagent
                    .as_ref()
                    .and_then(|subagent| subagent.title.clone())
                    .or_else(|| tool["title"].as_str().map(str::to_owned))
                    .or_else(|| tool["kind"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| "ACP tool".to_owned());
                let mut metadata = tool_metadata(&tool);
                if subagent.is_some() {
                    metadata["toolKind"] = json!("subagent");
                }
                if new {
                    self.history_chunk(
                        "assistant",
                        json!({"type":"toolCall","id":id,"name":name,"arguments":tool["rawInput"],"toolKind":metadata["toolKind"],"changes":metadata["changes"]}),
                    );
                    if emit {
                        self.events.push(json!({"type":"tool_execution_start","toolCallId":id,"toolName":name,"args":tool["rawInput"],"toolKind":metadata["toolKind"],"changes":metadata["changes"]}));
                    }
                }
                // The extracted subagent lifecycle settles a tool even when the agent
                // leaves the call's own status running.
                let failed = tool["status"] == "failed"
                    || subagent.as_ref().is_some_and(|s| s.status == "failed");
                let done = matches!(tool["status"].as_str(), Some("completed" | "failed"))
                    || subagent.as_ref().is_some_and(|s| s.status != "running");
                let mut result = tool_result(&tool);
                if let Some(summary) = subagent.as_ref().and_then(|s| s.result.as_ref())
                    && let Some(content) = result["content"].as_array_mut()
                {
                    content.push(json!({"type":"text","text":summary}));
                }
                if done {
                    let mut saved = result.clone();
                    saved["role"] = json!("toolResult");
                    saved["toolCallId"] = json!(id);
                    saved["isError"] = json!(failed);
                    if let Some(previous) = self
                        .history
                        .iter_mut()
                        .find(|m| m["role"] == "toolResult" && m["toolCallId"] == id)
                    {
                        *previous = saved;
                    } else {
                        self.history.push(saved);
                    }
                }
                if emit {
                    let mut event = json!({"type":if done {"tool_execution_end"} else {"tool_execution_update"},
                        "toolCallId":id,"isError":failed});
                    event[if done { "result" } else { "partialResult" }] = result;
                    self.events.push(event);
                }
            }
            "available_commands_update" => {
                self.commands = update["availableCommands"].as_array().into_iter().flatten()
                    .map(|command| json!({"name":command["name"],"description":command["description"],"source":"ACP"})).collect();
                if emit {
                    self.reply(
                        &json!({"type":"get_commands","id":"commands"}),
                        json!({"commands":self.available_commands()}),
                    );
                }
            }
            "plan" | "plan_update" => {
                let turn = self.history.iter().filter(|m| m["role"] == "user").count();
                let id = update["planId"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("acp-plan-{turn}"));
                let mut value = update.clone();
                value["_shinaPlanId"] = json!(id);
                crate::work::save_plan(&mut self.history, &id, &value);
                self.events
                    .push(json!({"type":"harness_update","field":"plan","value":value}));
            }
            "config_option_update" => {
                self.option_descriptors = crate::descriptors::acp_provider_option_descriptors(
                    update["configOptions"].as_array().map(Vec::as_slice),
                    None,
                );
                self.config_options = update["configOptions"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if self.antigravity {
                    crate::antigravity::observe_thinking_levels(
                        &self.config_options,
                        &mut self.thinking_levels,
                    );
                }
                self.events.push(json!({
                    "type": "harness_update",
                    "field": "optionDescriptors",
                    "value": self.option_descriptors
                }));
            }
            "current_mode_update" => {
                self.events.push(
                    json!({"type":"harness_update","field":update["sessionUpdate"],"value":update}),
                );
            }
            other => {
                self.events.push(json!({"type":"extension_ui_request","method":"notify","message":format!("Unsupported ACP session update: {other}")}));
            }
        }
    }
}

/// KSG AntigravityProtocol, provider-kiro/acpSupport, provider-grok/acpSupport.
fn acp_approval_options(
    provider: &str,
    params: &Value,
) -> Option<Vec<crate::approval::OptionItem>> {
    let options = params["options"].as_array()?;
    let find = |kind: &str| {
        options.iter().find(|o| {
            o["kind"] == kind
                && o["optionId"]
                    .as_str()
                    .is_some_and(|id| !id.trim().is_empty())
        })
    };
    let option = |decision: &str, label: &str| crate::approval::OptionItem {
        decision: decision.into(),
        label: label.into(),
        warning: None,
    };
    if provider.contains("antigravity") {
        if params["toolCall"]["toolCallId"]
            .as_str()
            .is_some_and(|id| id.starts_with("interaction_"))
        {
            return Some(vec![]);
        }
        let mut result = vec![];
        if find("allow_once").is_some() {
            result.push(option("accept", "Allow once"));
        }
        if let Some(always) = find("allow_always") {
            let warning = &always["_meta"]["agy.security.warning"];
            let text = warning["message"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    warning["title"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                });
            let mut entry = option("acceptForSession", "Allow for this thread");
            entry.warning = text.map(|s| {
                if s.encode_utf16().count() > 512 {
                    format!(
                        "{}...",
                        String::from_utf16_lossy(&s.encode_utf16().take(509).collect::<Vec<_>>())
                    )
                } else {
                    s.to_owned()
                }
            });
            result.push(entry);
        }
        if find("reject_once").is_some() {
            result.push(option("decline", "Deny"));
        }
        result.push(option("cancel", "Cancel"));
        Some(result)
    } else if provider.contains("kiro") {
        let mut result = vec![];
        if find("allow_once").is_some() {
            result.push(option("accept", "Allow once"));
        } else if find("allow_always").is_some() {
            result.push(option("acceptForSession", "Always allow"));
        }
        if find("reject_once").is_some() {
            result.push(option("decline", "Deny"));
        }
        result.push(option("cancel", "Cancel"));
        Some(result)
    } else if provider.contains("grok") {
        let mut result = vec![option("cancel", "Cancel")];
        if find("reject_once").is_some() {
            result.push(option("decline", "Decline"));
        }
        if options.iter().any(|o| {
            o["kind"] == "allow_always"
                && o["optionId"]
                    .as_str()
                    .is_some_and(|id| id.trim() == "allow-edits-session")
        }) {
            result.push(option("acceptForSession", "Allow all edits this session"));
        }
        if find("allow_once").is_some() {
            result.push(option("accept", "Approve"));
        }
        Some(result)
    } else {
        None
    }
}

/// KSG AntigravityProtocol.extractAntigravityUserInputQuestion.
fn antigravity_question(params: &Value) -> Option<Value> {
    fn bounded(text: &str, limit: usize) -> String {
        if text.encode_utf16().count() <= limit {
            return text.to_owned();
        }
        format!(
            "{}...",
            String::from_utf16_lossy(&text.encode_utf16().take(limit - 3).collect::<Vec<_>>())
        )
    }
    let options = params["options"].as_array().filter(|a| !a.is_empty())?;
    let mut seen = std::collections::HashSet::new();
    let options = options
        .iter()
        .map(|o| {
            let id = o["optionId"].as_str().filter(|id| !id.trim().is_empty())?;
            if !seen.insert(id) {
                return None;
            }
            let label = bounded(
                o["name"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(id),
                512,
            );
            Some(json!({"value":id,"label":label,"description":label}))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(
        json!({"id":params["toolCall"]["toolCallId"],"header":"Question",
        "question":bounded(params["toolCall"]["title"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Choose an option."),8_000),
        "multiSelect":false,"allowCustomAnswer":false,"options":options}),
    )
}

fn permission_response(id: Value, option: Option<String>) -> Value {
    let outcome = option.map_or_else(
        || json!({"outcome":"cancelled"}),
        |id| json!({"outcome":"selected","optionId":id}),
    );
    json!({"jsonrpc":"2.0","id":id,"result":{"outcome":outcome}})
}

fn tool_result(tool: &Value) -> Value {
    let mut content: Vec<Value> = tool["content"].as_array().into_iter().flatten().map(|part| {
        if part["type"] == "content" { part["content"].clone() }
        else if part["type"] == "diff" {
            json!({"type":"text","text":format!("{}\n{}", part["path"].as_str().unwrap_or("Diff"), part["newText"].as_str().unwrap_or(""))})
        } else { json!({"type":"text","text":part.to_string()}) }
    }).collect();
    if content.is_empty() && !tool["rawOutput"].is_null() {
        content.push(json!({"type":"text","text":tool["rawOutput"].as_str().map(str::to_owned).unwrap_or_else(||tool["rawOutput"].to_string())}));
    }
    let mut result = tool_metadata(tool);
    result["content"] = json!(content);
    result["details"] = json!({"acp":tool,"exitCode":tool["rawOutput"]["exitCode"]});
    result
}

/// Ported from KSG Code's provider-acp adapter's kind switch and diff parts.
fn tool_metadata(tool: &Value) -> Value {
    use crate::work::{FileChange, ToolKind, result_metadata, text_diff};
    let kind = match tool["kind"].as_str().unwrap_or("") {
        "execute" => ToolKind::Command,
        "edit" | "delete" | "move" => ToolKind::FileChange,
        "search"
            if tool["rawInput"]["url"].is_string()
                || tool["rawOutput"]["results"]
                    .as_array()
                    .is_some_and(|r| r.iter().any(|r| r["url"].is_string())) =>
        {
            ToolKind::WebSearch
        }
        "search" => ToolKind::FileSearch,
        "fetch" => ToolKind::WebSearch,
        "read" => ToolKind::Read,
        "list" => ToolKind::List,
        "subagent" => ToolKind::Subagent,
        "plan" => ToolKind::Plan,
        _ => ToolKind::Other,
    };
    let mut changes: Vec<_> = tool["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["type"] == "diff")
        .filter_map(|part| {
            let path = part["path"].as_str()?;
            let diff = part["patch"]["text"]
                .as_str()
                .or_else(|| part["diff"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    text_diff(
                        part["oldText"].as_str().unwrap_or(""),
                        part["newText"].as_str().unwrap_or(""),
                    )
                });
            let mut change = FileChange::new(path, tool["kind"].as_str().unwrap_or("edit"), &diff);
            change.old_path = part["oldPath"].as_str().map(str::to_owned);
            Some(change)
        })
        .collect();
    if kind == ToolKind::FileChange && changes.is_empty() {
        for location in tool["locations"].as_array().into_iter().flatten() {
            if let Some(path) = location["path"].as_str() {
                changes.push(FileChange::new(
                    path,
                    tool["kind"].as_str().unwrap_or("edit"),
                    "",
                ));
            }
        }
    }
    let mut metadata = result_metadata(kind, changes, &tool["rawOutput"]);
    metadata["input"] = tool["rawInput"].clone();
    metadata
}

fn run(
    mut command: std::process::Command,
    start: Start,
    input: Receiver<Value>,
    output: &SyncSender<Event>,
    wake: &Waker,
    stopped: &AtomicBool,
    owner: &SessionOwner,
    claude_skills: Vec<crate::composer_skills::Skill>,
) -> Result<(), String> {
    command
        .current_dir(&start.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = crate::process::Process::spawn(&mut command)
        .map_err(|e| format!("Could not start ACP agent: {e}"))?;
    let mut stdin = process.stdin.take().unwrap();
    let stdout = process.stdout.take().unwrap();
    let stderr = process.stderr.take().unwrap();
    let is_antigravity = start.id == "antigravity";
    let (received, records) = mpsc::sync_channel(256);
    let reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            // KSG makeAntigravityStdoutTransform: the OAuth URL line is not JSON-RPC.
            let record = if is_antigravity {
                match crate::antigravity::read_agent_record(&mut stdout) {
                    Ok(crate::antigravity::AgentLine::AuthUrl(url)) => {
                        Ok(Some(json!({"shina/antigravity-auth-url":url})))
                    }
                    Ok(crate::antigravity::AgentLine::Record(record)) => Ok(Some(record)),
                    Ok(crate::antigravity::AgentLine::Eof) => Ok(None),
                    Err(error) => Err(error),
                }
            } else {
                backend::read_record(&mut stdout).map_err(|error| error.replace("Pi", "ACP"))
            };
            let done = !matches!(record, Ok(Some(_)));
            if received.send(record).is_err() || done {
                break;
            }
        }
    });
    let diagnostics = output.clone();
    let diagnostic_wake = wake.clone();
    let bridge_token = start.link.as_ref().map(|host| host.token.clone());
    let errors = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut sent = 0;
        loop {
            let mut bytes = Vec::new();
            match Read::by_ref(&mut reader)
                .take(8192)
                .read_until(b'\n', &mut bytes)
            {
                Ok(0) | Err(_) => break,
                _ => {
                    let text = String::from_utf8_lossy(&bytes);
                    let text = match bridge_token.as_deref() {
                        Some(token) => std::borrow::Cow::Owned(text.replace(token, "[redacted]")),
                        None => text,
                    };
                    // KSG makeAntigravityStderrHandler: the BROWSER helper marks the OAuth URL.
                    if is_antigravity && let Some(url) = crate::antigravity::stderr_auth_url(&text)
                    {
                        let _ = backend::publish(
                            &diagnostics,
                            &diagnostic_wake,
                            Event::Record(json!({"type":"harness_update",
                                "field":"signInUrl","value":url})),
                        );
                        continue;
                    }
                    if sent < 12 {
                        sent += 1;
                        // Agent stderr can include provider secrets. Keep it out of persisted diagnostics.
                        let message = crate::update::redact(&crate::projects::redact(&text));
                        if !backend::publish(
                            &diagnostics,
                            &diagnostic_wake,
                            Event::Diagnostic(message),
                        ) {
                            break;
                        }
                    }
                }
            }
        }
    });
    let (writes, outgoing) = mpsc::sync_channel(32);
    let (write_result, written) = mpsc::channel();
    let writing = std::sync::Arc::new(std::sync::Mutex::new(None::<Instant>));
    let writer_clock = writing.clone();
    let writer = thread::spawn(move || {
        let result = (|| {
            for record in outgoing {
                *writer_clock.lock().unwrap() = Some(Instant::now());
                backend::write_record(&mut stdin, &record).map_err(|e| e.replace("Pi", "ACP"))?;
                *writer_clock.lock().unwrap() = None;
            }
            Ok::<_, String>(())
        })();
        let _ = write_result.send(result);
    });
    let mut client = Client::new(start);
    let executable = command.get_program().to_string_lossy();
    if executable.rsplit(['/', '\\']).next().is_some_and(|name| {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "copilot" | "copilot.exe" | "copilot.cmd"
        )
    }) {
        client.restart_steering = false;
    }
    client.claude_skills = claude_skills;
    if is_antigravity {
        client.antigravity = true;
        // A configured gateway switches authenticate to the "gateway" method and
        // narrows the model catalog to the configured gateway families.
        let gateway = crate::antigravity::gateway()?;
        client.antigravity_gateway = gateway.is_some();
        client.gateway_models = gateway.map(|gateway| gateway.models).unwrap_or_default();
        // Version gating only applies to the real `agy` CLI, never a registry override.
        let program = std::path::Path::new(&*executable)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(
            program.as_str(),
            "agy" | "agy.exe" | "antigravity" | "antigravity.exe"
        ) {
            client.antigravity_cli = true;
            client.antigravity_version = crate::antigravity::cli_version();
        }
    }
    client.initialize();
    let result = (|| {
        let mut started = Instant::now();
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
                return Err("ACP agent stopped reading commands.".into());
            }
            // A permission dialog is live startup, just like Pi's workspace trust dialog.
            if !client.permissions.is_empty() {
                started = Instant::now();
                for request in client.pending.values_mut() {
                    if request.deadline.is_some() {
                        request.deadline = Some(Instant::now() + Duration::from_secs(30));
                    }
                }
            }
            if !client.ready && started.elapsed() >= Duration::from_secs(15) {
                return Err("ACP agent did not initialize its session within 15 seconds.".into());
            }
            if client.pending.values().any(|p| {
                p.deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            }) {
                return Err(
                    "ACP agent did not answer a session operation within 30 seconds.".into(),
                );
            }
            for record in std::mem::take(&mut client.wire) {
                writes
                    .try_send(record)
                    .map_err(|_| "ACP agent stopped accepting commands.")?;
            }
            for mut record in std::mem::take(&mut client.events) {
                crate::attachments::import_history(&mut record);
                if !backend::publish(output, wake, Event::Record(record)) {
                    return Ok(());
                }
            }
            for _ in 0..64 {
                match records.try_recv() {
                    Ok(Ok(Some(record))) => {
                        if let Some(url) = record["shina/antigravity-auth-url"].as_str() {
                            client.events.push(json!({"type":"harness_update",
                                "field":"signInUrl","value":url}));
                            continue;
                        }
                        client.incoming(record)?;
                    }
                    Ok(Ok(None)) => {
                        return Err(
                            "ACP agent closed its output. Retry to resume the saved session."
                                .into(),
                        );
                    }
                    Ok(Err(error)) => return Err(error),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Err("ACP output reader stopped.".into());
                    }
                }
            }
            // terminal/wait_for_exit answers once the child records an exit status.
            client.flush_terminal_waiters();
            // Publish responses before the UI can dispatch its next operation.
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
                    "ACP agent exited with {status}. Retry to resume its session."
                ));
            }
        }
    })();
    // Preserve already received chunks when a later record in the same batch fails.
    for mut record in std::mem::take(&mut client.events) {
        crate::attachments::import_history(&mut record);
        if !backend::publish(output, wake, Event::Record(record)) {
            break;
        }
    }
    let uncertain: Vec<_> = client
        .pending
        .values()
        .filter(|pending| pending.request["type"] == "steer")
        .map(|pending| pending.request["original"].clone())
        .collect();
    for record in uncertain {
        client.uncertain_steer(&record);
    }
    for mut record in std::mem::take(&mut client.events) {
        crate::attachments::import_history(&mut record);
        backend::publish(output, wake, Event::Record(record));
    }
    drop(writes);
    drop(records);
    process.terminate();
    if let Err(error) = process.reap(Duration::from_secs(1)) {
        owner.quarantined.store(true, Ordering::Release);
        return Err(format!(
            "Could not reap ACP agent. Reconnect is disabled until restart: {error}"
        ));
    }
    crate::process::join_for(writer, Duration::from_millis(200));
    crate::process::join_for(reader, Duration::from_millis(200));
    crate::process::join_for(errors, Duration::from_millis(200));
    result
}

#[cfg(test)]
#[path = "acp_tests.rs"]
mod tests;

/// Self-invoked stdio MCP fallback, like KSG's acp-mcp-bridge. Only its inherited
/// thread-scoped bearer token reaches the loopback server. Nothing is saved to disk.
pub fn mcp_bridge() -> Result<(), String> {
    let url = std::env::var(crate::mcp::URL_ENV).map_err(|_| "Missing Shina MCP URL.")?;
    let token = std::env::var(crate::mcp::TOKEN_ENV).map_err(|_| "Missing Shina MCP token.")?;
    let host = HostLink {
        url,
        token,
        extension: PathBuf::new(),
        access_mode: String::new(),
    };
    let mut stdin = BufReader::new(std::io::stdin().lock());
    let mut stdout = std::io::stdout().lock();
    while let Some(record) = backend::read_record(&mut stdin)? {
        match relay(&host, &record) {
            Ok(Some(response)) => backend::write_record(&mut stdout, &response)?,
            Ok(None) => {}
            Err(error) => {
                if let Some(id) = record.get("id") {
                    backend::write_record(
                        &mut stdout,
                        &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":error}}),
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn relay(host: &HostLink, record: &Value) -> Result<Option<Value>, String> {
    let endpoint = host
        .url
        .strip_prefix("http://127.0.0.1:")
        .and_then(|value| value.strip_suffix("/mcp"))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or("Shina MCP must use its loopback HTTP endpoint.")?;
    if host.token.contains(['\r', '\n']) {
        return Err("Invalid Shina MCP bearer token.".into());
    }
    let mut stream =
        TcpStream::connect_timeout(&([127, 0, 0, 1], endpoint).into(), Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(635)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let body = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    write!(stream, "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{endpoint}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", host.token, body.len()).map_err(|e| e.to_string())?;
    stream.write_all(&body).map_err(|e| e.to_string())?;
    let (status, response) = http_response(&mut BufReader::new(stream))?;
    if !matches!(status, 200 | 202) {
        return Err(
            "Shina MCP refused the request. Retry the thread to refresh its bridge.".into(),
        );
    }
    if record.get("id").is_none() {
        return Ok(None);
    }
    serde_json::from_slice(&response)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Shina sends fixed-length responses. Read that body, not until TCP EOF. In
/// particular Windows can reset a closed connection after delivering a valid response.
pub(crate) fn http_response(reader: &mut impl BufRead) -> Result<(u16, Vec<u8>), String> {
    let mut length = None;
    let mut status = None;
    let mut total = 0;
    loop {
        let mut line = vec![];
        let count = Read::by_ref(reader)
            .take(8193)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        total += count;
        if count == 0 || count > 8192 || total > 32 * 1024 {
            return Err("Invalid or oversized Shina MCP HTTP response headers.".into());
        }
        let line = std::str::from_utf8(&line).map_err(|e| e.to_string())?;
        if status.is_none() {
            status = line
                .split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u16>().ok());
            if !line.starts_with("HTTP/1.1 ") || status.is_none() {
                return Err("Invalid Shina MCP HTTP status.".into());
            }
        } else if line == "\r\n" {
            break;
        } else if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            if length.is_some() {
                return Err("Duplicate Shina MCP Content-Length.".into());
            }
            length = Some(value.trim().parse::<usize>().map_err(|e| e.to_string())?);
        }
    }
    let length = length
        .filter(|length| *length <= 8 * 1024 * 1024)
        .ok_or("Missing or oversized Shina MCP response body length.")?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok((status.unwrap(), body))
}
