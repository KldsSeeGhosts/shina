//! ACP v1 JSON-RPC over stdio, ported from KSG's effect-acp and provider-acp.
//! The UI sees the same event envelope as Pi. No provider authentication is performed here.
use crate::{
    backend::{self, Event, HostLink, Pi, SessionOwner, SessionSource},
    harness::{Agent, Start},
};
use rune::Waker;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub fn start(agent: Agent, start: Start, wake: Waker) -> Result<Pi, String> {
    if matches!(start.source, SessionSource::Fork(_)) {
        return Err("ACP conversation forks are not supported. Start a new thread instead.".into());
    }
    if start.link.is_none() {
        return Err("ACP requires Shina's MCP bridge. Retry after the bridge is available.".into());
    }
    let command = agent.process()?;
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
}

impl Client {
    fn new(start: Start) -> Self {
        let awareness_sent = start.context_sent;
        let restart_steering = !matches!(start.id.as_str(), "github-copilot-cli" | "copilot");
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
        self.request("initialize", json!({
            "protocolVersion":1,
            "clientInfo":{"name":"Shina","version":env!("CARGO_PKG_VERSION")},
            "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}
        }), json!({"type":"initialize"}));
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
                if self.session.as_deref().is_some_and(|session| session != id) {
                    return Err("ACP agent sent an update for another session.".into());
                }
                if let SessionSource::Resume(expected) = &self.start.source
                    && !self.ready
                    && expected != id
                {
                    return Err("ACP agent replayed another session.".into());
                }
                self.update(&record["params"]["update"]);
            } else if method == "session/request_permission" && record.get("id").is_some() {
                self.permission(&record)?;
            } else if record.get("id").is_some() {
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
            let message = error["message"].as_str().unwrap_or("Unknown ACP error");
            let message = if let Some(host) = &self.start.link {
                message.replace(&host.token, "[redacted]")
            } else {
                message.to_owned()
            };
            // Never authenticate or retry after an auth denial.
            let message = format!(
                "ACP {kind} failed: {message}. If authentication is required, configure or sign in to the agent outside Shina, then retry."
            );
            if matches!(kind, "initialize" | "start") {
                return Err(message);
            }
            if kind == "prompt" {
                // The request may already have run tools. Do not restore/replay accepted input.
                return Err(message);
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
                self.active_steering = supports_active_steering(&result["_meta"]);
                self.events
                    .push(json!({"type":"harness_update","field":"turnCapabilities",
                    "value":self.turn_capabilities()}));
                let mut params =
                    json!({"cwd":self.start.workspace,"mcpServers":self.mcp_servers()?});
                let method = match &self.start.source {
                    SessionSource::New => "session/new",
                    SessionSource::Resume(session) => {
                        params["sessionId"] = json!(session);
                        self.session = Some(session.clone());
                        if self.capabilities["loadSession"] == true {
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
                self.request(
                    method,
                    params.clone(),
                    json!({"type":"start","method":method,"params":params}),
                );
            }
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
                self.ready = true;
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
                self.reply(&request, self.model.clone());
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

    fn command(&mut self, record: Value) -> Result<(), String> {
        let kind = record["type"].as_str().unwrap_or("");
        if kind == "extension_ui_response" {
            let key = record["id"].as_str().unwrap_or("");
            if let Some(permission) = self.permissions.remove(key) {
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
                    if option_id == crate::descriptors::ACP_SESSION_MODE_OPTION_ID {
                        let mode_id = record["value"]
                            .as_str()
                            .map(|v| if v == crate::descriptors::ACP_EMPTY_OPTION_VALUE { "" } else { v })
                            .unwrap_or("");
                        self.request(
                            "session/set_mode",
                            json!({"sessionId":self.session,"modeId":mode_id}),
                            record,
                        );
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
                crate::mcp::INSTRUCTIONS, self.start.instructions.as_deref().unwrap_or("")
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

    fn permission(&mut self, record: &Value) -> Result<(), String> {
        let params = &record["params"];
        if self
            .session
            .as_deref()
            .is_some_and(|session| params["sessionId"] != session)
        {
            return Err("ACP permission request belongs to another session.".into());
        }
        let options = params["options"]
            .as_array()
            .ok_or("ACP permission request has no options.")?;
        let option = |kinds: &[&str]| -> Option<String> {
            kinds.iter().find_map(|kind| {
                options
                    .iter()
                    .find(|o| o["kind"] == *kind)
                    .and_then(|o| o["optionId"].as_str())
                    .map(str::to_owned)
            })
        };
        let permission = Permission {
            id: record["id"].clone(),
            allow: option(&["allow_once"]),
            session: option(&["allow_always"]),
            reject: option(&["reject_once"]),
            offered: acp_approval_options(&self.start.id, params),
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
        if auto || self.cancelling {
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
            "tool_call" | "tool_call_update" => {
                let Some(id) = update["toolCallId"].as_str() else {
                    return;
                };
                let new = !self.tools.contains_key(id);
                let tool = self.tools.entry(id.into()).or_insert_with(|| json!({}));
                for (key, value) in update.as_object().into_iter().flatten() {
                    if !value.is_null() {
                        tool[key] = value.clone();
                    }
                }
                let tool = tool.clone();
                let name = tool["title"]
                    .as_str()
                    .or_else(|| tool["kind"].as_str())
                    .unwrap_or("ACP tool");
                let metadata = tool_metadata(&tool);
                if new {
                    self.history_chunk(
                        "assistant",
                        json!({"type":"toolCall","id":id,"name":name,"arguments":tool["rawInput"],"toolKind":metadata["toolKind"],"changes":metadata["changes"]}),
                    );
                    if emit {
                        self.events.push(json!({"type":"tool_execution_start","toolCallId":id,"toolName":name,"args":tool["rawInput"],"toolKind":metadata["toolKind"],"changes":metadata["changes"]}));
                    }
                }
                let done = matches!(tool["status"].as_str(), Some("completed" | "failed"));
                let result = tool_result(&tool);
                if done {
                    let mut saved = result.clone();
                    saved["role"] = json!("toolResult");
                    saved["toolCallId"] = json!(id);
                    saved["isError"] = json!(tool["status"] == "failed");
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
                        "toolCallId":id,"isError":tool["status"]=="failed"});
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
        if options
            .iter()
            .any(|o| o["kind"] == "allow_always" && o["optionId"] == "allow-edits-session")
        {
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
    let (received, records) = mpsc::sync_channel(256);
    let reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            let record =
                backend::read_record(&mut stdout).map_err(|error| error.replace("Pi", "ACP"));
            let done = !matches!(record, Ok(Some(_)));
            if received.send(record).is_err() || done {
                break;
            }
        }
    });
    let diagnostics = output.clone();
    let diagnostic_wake = wake.clone();
    let bridge_token = start
        .link
        .as_ref()
        .map(|host| host.token.clone())
        .unwrap_or_default();
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
                _ if sent < 12 => {
                    sent += 1;
                    // Agent stderr can include provider secrets. Keep it out of persisted diagnostics.
                    let text = String::from_utf8_lossy(&bytes).replace(&bridge_token, "[redacted]");
                    let message = crate::update::redact(&crate::projects::redact(&text));
                    if !backend::publish(&diagnostics, &diagnostic_wake, Event::Diagnostic(message))
                    {
                        break;
                    }
                }
                _ => {}
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
                    Ok(Ok(Some(record))) => client.incoming(record)?,
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
