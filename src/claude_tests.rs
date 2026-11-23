use super::*;
use crate::model::{AccessMode, Status, Thread};

const SESSION: &str = "40000000-0000-4000-8000-000000000031";

#[test]
fn claude_task_tracks_its_nested_tool_activity_while_running() {
    let mut client = client("full-access");
    let mut thread = Thread::new(1);
    client.tool_start(
        &json!({"id":"task","name":"Task","input":{
            "subagent_type":"Explore","prompt":"Find entry points"
        }}),
        "",
        true,
    );
    client.tool_start(
        &json!({"id":"nested","name":"Read","input":{"file_path":"src/main.rs"}}),
        "task",
        true,
    );
    for record in std::mem::take(&mut client.events) {
        thread.apply(record);
    }
    let runs = crate::agents::runs(&thread);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].tool_id, "task");
    assert_eq!(runs[0].brief, "Find entry points");
    assert_eq!(runs[0].activity, Some("src/main.rs"));
    assert_eq!(runs[0].state, crate::agents::AgentState::Working);
}

#[test]
fn approval_cancel_interrupts_decline_does_not_and_session_uses_session_rules() {
    for decision in ["accept", "acceptForSession", "decline", "cancel"] {
        let mut client = client("approval-required");
        client.answer_decision(
            Approval {
                request_id: "decision".into(),
                input: json!({"command":"echo hello"}),
                name: "Bash".into(),
                tool_id: json!("tool"),
                hook: false,
                suggestions: vec![],
            },
            decision,
            None,
        );
        let response = &client.wire.last().unwrap()["response"]["response"];
        assert_eq!(response["interrupt"] == true, decision == "cancel");
        assert_eq!(
            response["behavior"],
            if decision.starts_with("accept") {
                "allow"
            } else {
                "deny"
            }
        );
        if decision == "acceptForSession" {
            assert_eq!(response["updatedPermissions"][0]["destination"], "session");
            assert_eq!(
                response["updatedPermissions"][0]["rules"][0]["toolName"],
                "Bash"
            );
        }
    }
}

#[test]
fn claude_catalog_probe_initializes_without_a_prompt_or_persisted_session() {
    for version in ["--handshake-version", "--legacy-version"] {
        check_claude_catalog_probe(version);
    }
}

fn check_claude_catalog_probe(version: &str) {
    let dir = crate::store::directory().unwrap();
    let script = dir.join("claude-probe.py");
    std::fs::write(&script, include_str!("../assets/tests/fake_claude.py")).unwrap();
    let log = dir.join(format!("claude-probe{version}.jsonl"));
    let mut command = Command::new(if cfg!(windows) { "python" } else { "python3" });
    command
        .arg("-u")
        .arg(script)
        .arg("--log")
        .arg(&log)
        .arg(version)
        .env("CLAUDE_CONFIG_DIR", dir.join("probe-config"));
    let start = Start::discovery("claude", dir.clone());
    let pi = Pi::worker(
        Arc::new(|| {}),
        start.owner.clone(),
        None,
        move |input, events, wake, stopped, owner| {
            run_mode(command, start, input, events, wake, stopped, owner, true)
        },
    );
    let mut models = Value::Null;
    loop {
        match pi
            .events
            .recv_timeout(Duration::from_secs(10))
            .expect("probe timeout")
        {
            Event::Record(record) if record["command"] == "get_available_models" => {
                models = record["data"]["models"].clone()
            }
            Event::Record(_) => {}
            Event::Exited => break,
            Event::Error(error) => panic!("{error}"),
            Event::Diagnostic(_) | Event::ProviderJob(_) => {}
        }
    }
    // Manifest rows list under their slugs, with the manifest's display names;
    // `default` is Claude Code's pointer, not a catalog row.
    assert!(
        models
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == "claude-opus-4-5" && model["name"] == "Claude Opus 4.5")
    );
    let wire = records(&log);
    assert!(wire.iter().all(|record| record["type"] != "user"));
    if cfg!(windows) {
        assert!(
            wire.iter()
                .filter(|record| record["argv"].is_array())
                .all(|record| record["console_window"] == false)
        );
    }
    let launches = wire.iter().filter_map(|record| record["argv"].as_array());
    assert_eq!(
        launches.clone().count(),
        if version == "--legacy-version" { 2 } else { 1 }
    );
    assert_eq!(
        launches.flatten().any(|arg| arg == "--version"),
        version == "--legacy-version"
    );
    let mut launch = Command::new("claude");
    Client::new(Start::discovery("claude", dir)).launch(&mut launch);
    let args: Vec<_> = launch
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert!(args.contains(&"--no-session-persistence".into()));
    assert!(!args.contains(&"--mcp-config".into()));
}

fn spec(source: SessionSource, access: &str) -> Start {
    Start {
        id: "claude".into(),
        workspace: crate::store::directory().unwrap(),
        source,
        instructions: Some("Captain profile\nEssential memory: use Rust".into()),
        shina: crate::mcp::INSTRUCTIONS,
        link: Some(crate::backend::HostLink {
            url: "http://127.0.0.1:12345/mcp".into(),
            token: "fixture-thread-token".into(),
            extension: PathBuf::new(),
            access_mode: access.into(),
        }),
        owner: Arc::default(),
        context_sent: false,
        prompt_suggestions: false,
        metadata_only: false,
    }
}

fn fixture(access: &str, flags: &[&str]) -> (Thread, PathBuf) {
    let dir = crate::store::directory().unwrap();
    let script = dir.join("fake_claude.py");
    std::fs::write(&script, include_str!("../assets/tests/fake_claude.py")).unwrap();
    let log = dir.join("claude.jsonl");
    let mut command = Command::new(if cfg!(windows) { "python" } else { "python3" });
    command
        .arg("-u")
        .arg(script)
        .arg("--log")
        .arg(&log)
        .args(flags)
        .env("CLAUDE_CONFIG_DIR", dir.join("claude-config"));
    let mut thread = Thread::new(31);
    thread.harness = "claude".into();
    thread.native.access = AccessMode::parse(access).unwrap();
    thread.pi =
        Some(start_process(command, spec(SessionSource::New, access), Arc::new(|| {})).unwrap());
    (thread, log)
}

fn pump(thread: &mut Thread, done: impl Fn(&Thread) -> bool) {
    pump_for(thread, done, Duration::from_secs(10));
}

fn pump_for(thread: &mut Thread, done: impl Fn(&Thread) -> bool, timeout: Duration) {
    let end = Instant::now() + timeout;
    while !done(thread) {
        assert!(
            Instant::now() < end,
            "Claude timeout {:?}, {:?}",
            thread.status,
            thread.error
        );
        match thread
            .pi
            .as_ref()
            .unwrap()
            .events
            .recv_timeout(Duration::from_millis(100))
        {
            Ok(Event::Record(record)) => thread.apply(record),
            Ok(Event::Error(error)) => panic!("Claude fixture: {error}"),
            Ok(Event::Exited) => panic!("Claude fixture exited"),
            _ => {}
        }
    }
}

fn records(log: &Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

fn initialized_client(access: &str) -> Client {
    let mut client = Client::new(spec(SessionSource::New, access));
    client
        .initialized(
            &json!({"hook_on_failure_supported":true,"hooks_applied":true,
        "commands":[],"models":[{"value":"default","supportedEffortLevels":["low","high"]}]}),
        )
        .unwrap();
    settings_response(&mut client, json!({"effective":{},"sources":[]}));
    client
}

/// Answers the session's `get_settings` request, as the CLI's own settings files
/// would report them; finishes deferred initialization. `system`/`init` goes first,
/// the order the real CLI emits it.
fn settings_response(client: &mut Client, response: Value) {
    client
        .incoming(json!({"type":"system","subtype":"init","claude_code_version":"9.9.9"}))
        .unwrap();
    let id = client
        .pending
        .iter()
        .find(|(_, pending)| pending.request["subtype"] == "get_settings")
        .map(|(id, _)| id.clone())
        .expect("get_settings request");
    client
        .incoming(
            json!({"type":"control_response","response":{"subtype":"success",
        "request_id":id,"response":response}}),
        )
        .unwrap();
}

/// Answers the session's `mcp_status` request with `servers`, as the CLI would report its
/// resolved configuration.
fn status_response(client: &mut Client, servers: Value) {
    let id = client
        .pending
        .iter()
        .find(|(_, pending)| pending.request["subtype"] == "mcp_status")
        .map(|(id, _)| id.clone())
        .expect("mcp_status request");
    client
        .incoming(
            json!({"type":"control_response","response":{"subtype":"success",
        "request_id":id,"response":{"mcpServers":servers}}}),
        )
        .unwrap();
}

/// The CLI resolved exactly the inline `shina` entry the session launched with.
fn bind(client: &mut Client) {
    status_response(
        client,
        json!([{"name":"shina","status":"connected","config":{"type":"http",
            "url":"http://127.0.0.1:12345/mcp",
            "headers":{"Authorization":"Bearer fixture-thread-token"}}}]),
    );
}

fn client(access: &str) -> Client {
    let mut client = initialized_client(access);
    bind(&mut client);
    client.events.clear();
    client.wire.clear();
    client
        .capabilities
        .insert("interrupt_cancel_queued_v1".into());
    client
}

#[test]
fn handshake_stream_tools_nested_usage_and_process_only_mcp() {
    let (mut thread, log) = fixture("approval-required", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    assert!(thread.session.is_some());
    // Manifest rows list the catalog's effort levels under their manifest slugs.
    let opus = thread
        .models
        .iter()
        .find(|m| m.id == "claude-opus-5-5")
        .expect("Claude Opus 5.5 row");
    assert_eq!(
        opus.thinking_levels,
        ["low", "medium", "high", "xhigh", "max"]
    );
    assert!(thread.native.commands.iter().any(|c| c.name == "review"));
    thread.draft.replace_text("hello");
    thread.send();
    pump(&mut thread, |t| {
        t.status == Status::Completed && t.harness_state["usageLimits"].is_object()
    });
    assert_eq!(
        thread
            .messages
            .iter()
            .filter(|m| m.text == "Hello Claude")
            .count(),
        1
    );
    assert_eq!(thread.thoughts[0].text, "Checking");
    let read = thread.tools.iter().find(|t| t.id == "read").unwrap();
    assert_eq!(read.name, "read");
    assert_eq!(read.output, "fixture");
    assert!(read.done);
    assert_eq!(
        thread
            .tools
            .iter()
            .find(|t| t.id == "nested")
            .unwrap()
            .parent_call
            .as_deref(),
        Some("task")
    );
    assert_eq!(
        thread
            .tools
            .iter()
            .find(|t| t.id == "task")
            .unwrap()
            .subagents[0]["name"],
        "Explore"
    );
    assert!(thread.harness_state["plan"]["plan"].is_array());
    assert_eq!(thread.context.usage.tokens, Some(340));
    let records = records(&log);
    let args = records[0]["argv"].as_array().unwrap();
    let arg = |name: &str| {
        args[args.iter().position(|v| v == name).unwrap() + 1]
            .as_str()
            .unwrap()
    };
    let mcp: Value = serde_json::from_str(arg("--mcp-config")).unwrap();
    assert_eq!(
        mcp["mcpServers"]["shina"]["headers"]["Authorization"],
        "Bearer ${SHINA_MCP_TOKEN}"
    );
    assert!(
        !records[0]["argv"]
            .to_string()
            .contains("fixture-thread-token"),
        "the MCP token must not be visible in argv"
    );
    assert_eq!(records[0]["mcp_token"], "fixture-thread-token");
    assert!(arg("--append-system-prompt").contains(crate::mcp::INSTRUCTIONS));
    assert!(arg("--append-system-prompt").contains("Essential memory"));
    assert_eq!(arg("--permission-mode"), "manual");
    assert_eq!(records[1]["request"]["subtype"], "initialize");
    assert!(
        !thread
            .harness_state
            .to_string()
            .contains("fixture-thread-token")
    );
}

#[test]
fn permissions_for_every_mode_and_blocking_read_only_even_with_allow_rules() {
    for access in [
        "approval-required",
        "auto-accept-edits",
        "full-access",
        "read-only",
    ] {
        for name in [
            "Read",
            "Edit",
            "Bash",
            "mcp__server__mutate",
            "mcp__shina__list_threads",
            "UnknownTool",
        ] {
            let mut client = client(access);
            client
                .server_request(
                    &json!({"request_id":"p","request":{"subtype":"can_use_tool",
                "tool_name":name,"tool_use_id":"call","input":{"path":"x"}}}),
                )
                .unwrap();
            // A CLI-forwarded ask requires consent even under bypass or an ask rule for Read.
            // Authentic Shina tools are authorized server-side and need no dialog.
            let ask = access != "read-only" || name == "Read";
            assert_eq!(!client.approvals.is_empty(), ask, "{access} {name}");
            if ask {
                client
                    .command(json!({"type":"extension_ui_response","id":"p","confirmed":true}))
                    .unwrap();
            }
            let readonly_denied = !matches!(name, "Read") && !crate::mcp::is_tool(name);
            let expected = if access == "read-only" && readonly_denied {
                "deny"
            } else {
                "allow"
            };
            assert_eq!(
                client.wire.last().unwrap()["response"]["response"]["behavior"],
                expected,
                "{access} {name}"
            );
        }
    }
    let mut client = client("read-only");
    for (name, expected) in [
        ("Write", "deny"),
        ("Bash", "deny"),
        ("PowerShell", "deny"),
        ("Task", "deny"),
        ("Agent", "deny"),
        ("Skill", "deny"),
        ("mcp__shina__delegate_task", "allow"),
        ("mcp__shina__captain_crew_create", "allow"),
        ("mcp__shina__exec_shell", "deny"),
        ("mcp__shina_evil__delegate_task", "deny"),
        ("ExitPlanMode", "deny"),
    ] {
        client
            .server_request(
                &json!({"request_id":name,"request":{"subtype":"hook_callback",
            "callback_id":"shina-policy","input":{"hook_event_name":"PreToolUse",
            "tool_name":name,"tool_input":{},"tool_use_id":"call"}}}),
            )
            .unwrap();
        assert_eq!(
            client.wire.last().unwrap()["response"]["response"]["hookSpecificOutput"]["permissionDecision"],
            expected,
            "{name}"
        );
        assert!(client.approvals.is_empty());
    }
}

#[test]
fn read_only_admits_only_exact_shina_tools_on_an_authenticated_endpoint() {
    for (name, expected) in [
        ("mcp__shina__orchestrator_capabilities", "allow"),
        ("mcp__shina__list_threads", "allow"),
        ("mcp__shina__captain_crew_send", "allow"),
        ("mcp__shina__delegate_task", "allow"),
        ("mcp__shina__exec_shell", "deny"),
        ("mcp__shina_evil__list_threads", "deny"),
        ("mcp__Shina__list_threads", "deny"),
        ("mcp__shina2__delegate_task", "deny"),
        ("mcp__server__mutate", "deny"),
        ("Bash", "deny"),
        ("Write", "deny"),
        ("Task", "deny"),
    ] {
        let mut client = client("read-only");
        client
            .server_request(
                &json!({"request_id":"p","request":{"subtype":"can_use_tool",
            "tool_name":name,"tool_use_id":"call","input":{}}}),
            )
            .unwrap();
        assert_eq!(
            client.wire.last().unwrap()["response"]["response"]["behavior"],
            expected,
            "{name}"
        );
        assert!(client.approvals.is_empty(), "{name} needs no dialog");
    }
    // Without the thread-scoped authenticated endpoint, even exact names are foreign.
    let mut start = spec(SessionSource::New, "read-only");
    start.link = None;
    let mut offline = Client::new(start);
    offline
        .initialized(
            &json!({"hook_on_failure_supported":true,"hooks_applied":true,
        "commands":[],"models":[]}),
        )
        .unwrap();
    settings_response(&mut offline, json!({"effective":{},"sources":[]}));
    offline
        .server_request(
            &json!({"request_id":"p","request":{"subtype":"can_use_tool",
        "tool_name":"mcp__shina__list_threads","tool_use_id":"call","input":{}}}),
        )
        .unwrap();
    assert_eq!(
        offline.wire.last().unwrap()["response"]["response"]["behavior"],
        "deny"
    );
    // Other access modes keep forwarding Shina tool requests to the user.
    let mut client = client("approval-required");
    client
        .server_request(
            &json!({"request_id":"p","request":{"subtype":"can_use_tool",
        "tool_name":"mcp__shina__delegate_task","tool_use_id":"call","input":{}}}),
        )
        .unwrap();
    assert!(!client.approvals.is_empty());
    client
        .command(json!({"type":"extension_ui_response","id":"p","confirmed":false}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["response"]["response"]["behavior"],
        "deny"
    );
}

/// The fixture answers `mcp_status` with the endpoint it was launched with, asks for an
/// authentic tool, receives `allow`, then issues a real `tools/call` the MCP server
/// routes to `State::mcp_call` as this thread.
#[test]
fn read_only_dispatches_authentic_shina_calls_to_the_calling_thread() {
    let server = crate::mcp::server(Arc::new(|| {})).unwrap();
    let mut start = spec(SessionSource::New, "read-only");
    start.link = Some(crate::backend::HostLink {
        url: server.url.clone(),
        token: server.token_for(31),
        extension: PathBuf::new(),
        access_mode: "read-only".into(),
    });
    let dir = crate::store::directory().unwrap();
    let script = dir.join("fake_claude.py");
    std::fs::write(&script, include_str!("../assets/tests/fake_claude.py")).unwrap();
    let log = dir.join("claude-dispatch.jsonl");
    let mut command = Command::new(if cfg!(windows) { "python" } else { "python3" });
    command
        .arg("-u")
        .arg(script)
        .arg("--log")
        .arg(&log)
        .env("CLAUDE_CONFIG_DIR", dir.join("claude-config"));
    let mut thread = Thread::new(31);
    thread.harness = "claude".into();
    thread.native.access = AccessMode::parse("read-only").unwrap();
    thread.pi = Some(start_process(command, start, Arc::new(|| {})).unwrap());
    let mut state = crate::model::State::new(dir.clone(), false);
    state.load_global_for_test();
    state.threads.push({
        let mut caller = Thread::new(31);
        caller.harness = "claude".into();
        caller
    });
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.draft.replace_text("coordinate");
    thread.send();
    let end = Instant::now() + Duration::from_secs(15);
    while thread.status != Status::Completed {
        assert!(Instant::now() < end, "dispatch timeout {:?}", thread.error);
        while let Some(request) = server.next_request() {
            let result = state.mcp_call(request.thread, &request.tool, &request.args);
            let _ = request.reply.send(result);
        }
        match thread
            .pi
            .as_ref()
            .unwrap()
            .events
            .recv_timeout(Duration::from_millis(100))
        {
            Ok(Event::Record(record)) => thread.apply(record),
            Ok(Event::Error(error)) => panic!("Claude fixture: {error}"),
            _ => {}
        }
    }
    let records = records(&log);
    let dispatched = |name: &str| {
        records
            .iter()
            .find(|record| record["callback"] == name)
            .unwrap_or_else(|| panic!("no dispatch record for {name}"))
    };
    let capabilities = dispatched("mcp__shina__orchestrator_capabilities");
    assert_eq!(capabilities["allowed"], true);
    assert!(
        capabilities["result"]
            .as_str()
            .unwrap()
            .contains("\"caller\""),
        "authenticated dispatch returned the caller's capabilities: {capabilities}"
    );
    // Invalid arguments still reach the real handler: the server, not the callback,
    // rejects them. That distinguishes authenticated dispatch from a permission mock.
    let crew = dispatched("mcp__shina__captain_crew_send");
    assert_eq!(crew["allowed"], true);
    assert!(
        crew["result"].as_str().unwrap().contains("Captain home"),
        "dispatch reached mcp_call's caller check: {crew}"
    );
    // Denied names are never dispatched to the endpoint.
    let spoof = dispatched("mcp__shina_evil__list_threads");
    assert_eq!(spoof["allowed"], false);
    assert_eq!(spoof["result"], "");
    let shell = dispatched("mcp__shina__exec_shell");
    assert_eq!(shell["allowed"], false);
    assert_eq!(shell["result"], "");
}

#[test]
fn permission_dialog_plan_capture_and_questionnaire_round_trips() {
    let (mut thread, log) = fixture("approval-required", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    for prompt in ["permission", "plan", "question"] {
        thread.draft.replace_text(prompt);
        thread.send();
        pump(&mut thread, |t| t.request.is_some());
        if prompt == "plan" {
            assert_eq!(
                thread.harness_state["plan"]["markdown"],
                "# Plan\nChange the fixture."
            );
        }
        if prompt == "question" {
            assert!(thread.native.questionnaire.is_some());
            thread.respond(json!({"value":"{\"Which file?\":\"a.rs\"}"}));
        } else {
            thread.respond(json!({"confirmed":prompt=="permission"}));
        }
        pump(&mut thread, |t| t.status == Status::Completed);
    }
    let records = records(&log);
    assert!(
        records
            .iter()
            .any(|r| r["response"]["response"]["behavior"] == "allow")
    );
    assert!(records.iter().any(|r|r["response"]["response"]["hookSpecificOutput"]["updatedInput"]["answers"]["Which file?"]=="a.rs"));
    assert!(
        records.iter().any(
            |r| r["response"]["response"]["hookSpecificOutput"]["permissionDecision"] == "deny"
        )
    );
}

#[test]
fn steering_interrupt_queue_recovery_and_native_compaction() {
    let (mut thread, log) = fixture("full-access", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.draft.replace_text("hold");
    thread.send();
    pump(&mut thread, |t| t.harness_state["capabilities"].is_array());
    thread.draft.replace_text("steered");
    thread.send_during_run();
    pump(&mut thread, |t| {
        t.messages.iter().any(|m| m.text == "steered") && t.status == Status::Completed
    });
    thread.draft.replace_text("hold");
    thread.send();
    pump(&mut thread, |t| {
        t.status == Status::Running && t.messages.iter().filter(|m| m.text == "hold").count() == 2
    });
    thread.stop();
    pump(&mut thread, |t| !t.status.busy());
    assert!(thread.can_compact());
    thread.compact(Some("keep the plan".into()));
    pump(&mut thread, |t| {
        !t.native.compactions.is_empty() && !t.native.operating()
    });
    assert!(
        records(&log)
            .iter()
            .any(|r| r["request"]["subtype"] == "interrupt")
    );
    assert!(
        records(&log)
            .iter()
            .any(|r| r["message"]["content"][0]["text"] == "/compact keep the plan")
    );
    let mut client = client("full-access");
    client.running = true;
    client.command(json!({"type":"prompt","id":"queued","message":"recover","streamingBehavior":"followUp"})).unwrap();
    assert_eq!(client.queued.len(), 1);
    client.command(json!({"type":"clear_queue"})).unwrap();
    assert!(
        client
            .events
            .iter()
            .any(|r| r["data"]["followUp"] == json!(["recover"]))
    );
}

#[test]
fn model_effort_image_selection_and_live_access_control() {
    let (mut thread, log) = fixture("full-access", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.set_model("claude".into(), "default".into());
    pump(&mut thread, |t| !t.configuring);
    thread.set_thinking("high".into());
    pump(&mut thread, |t| !t.configuring);
    thread
        .pi
        .as_ref()
        .unwrap()
        .send(json!({"type":"prompt","id":"image","message":"image",
        "images":[{"mimeType":"image/png","data":"fixture-base64"}]}))
        .unwrap();
    pump(&mut thread, |t| t.status == Status::Completed);
    let records = records(&log);
    // `default` resolves to the manifest's default row and routes its slug, not an alias.
    assert!(
        records
            .iter()
            .any(|r| r["request"]["subtype"] == "set_model"
                && r["request"]["model"] == "claude-opus-5-5")
    );
    assert!(
        records
            .iter()
            .any(|r| r["request"]["settings"]["effortLevel"] == "high")
    );
    assert!(
        records
            .iter()
            .any(|r| r["message"]["content"][1]["source"]["media_type"] == "image/png")
    );
    let mut client = client("full-access");
    client
        .command(json!({"type":"set_access_mode","accessMode":"read-only"}))
        .unwrap();
    assert_eq!(client.access(), "read-only");
    assert_eq!(client.wire.last().unwrap()["request"]["mode"], "dontAsk");
}

#[test]
fn authentication_failure_stops_without_retry() {
    let (thread, log) = fixture("approval-required", &["--auth-fail"]);
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < end);
        if let Ok(Event::Error(error)) = thread
            .pi
            .as_ref()
            .unwrap()
            .events
            .recv_timeout(Duration::from_millis(100))
        {
            assert!(error.contains("`claude auth login`"));
            break;
        }
    }
    assert_eq!(
        records(&log)
            .iter()
            .filter(|r| r["request"]["subtype"] == "initialize")
            .count(),
        1
    );
    assert!(!records(&log).iter().any(|r| r["type"] == "user"));
}

#[test]
fn resume_fixture_replays_saved_transcript_and_failed_resume_has_no_fallback() {
    let (mut thread, _) = fixture("full-access", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.draft.replace_text("Prior prompt");
    thread.send();
    pump(&mut thread, |t| t.status == Status::Completed);
    let id = thread.session.clone().unwrap();
    drop(thread);
    let dir = crate::store::directory().unwrap();
    let log = dir.join("resume-log.jsonl");
    let mut command = Command::new(if cfg!(windows) { "python" } else { "python3" });
    command
        .arg("-u")
        .arg(dir.join("fake_claude.py"))
        .arg("--log")
        .arg(&log)
        .env("CLAUDE_CONFIG_DIR", dir.join("claude-config"));
    let mut resumed = Thread::new(31);
    resumed.harness = "claude".into();
    resumed.pi = Some(
        start_process(
            command,
            spec(SessionSource::Resume(id.clone()), "read-only"),
            Arc::new(|| {}),
        )
        .unwrap(),
    );
    pump(&mut resumed, |t| t.native.commands_loaded);
    assert_eq!(resumed.session.as_ref(), Some(&id));
    assert!(resumed.messages.iter().any(|m| m.text == "Prior prompt"));
    assert!(resumed.messages.iter().any(|m| m.text == "Hello Claude"));
    assert_eq!(
        resumed
            .tools
            .iter()
            .find(|t| t.id == "nested")
            .unwrap()
            .parent_call
            .as_deref(),
        Some("task")
    );
    assert_eq!(
        resumed
            .tools
            .iter()
            .find(|t| t.id == "task")
            .unwrap()
            .subagents
            .len(),
        1
    );
    let args = records(&log)[0]["argv"].clone();
    assert!(args.as_array().unwrap().contains(&json!("--resume")));
    assert!(!args.as_array().unwrap().contains(&json!("--session-id")));
    let mut missing = client("read-only");
    missing.start.source = SessionSource::Resume(SESSION.into());
    missing.session = SESSION.into();
    missing.config = dir.join("claude-config");
    assert!(
        load_history(&mut missing)
            .unwrap_err()
            .contains("replacement")
    );
}

#[test]
fn interrupt_recovers_only_inputs_proven_cancelled_and_never_resends_accepted_text() {
    let mut client = client("full-access");
    client
        .command(json!({"type":"prompt","id":"p","message":"first"}))
        .unwrap();
    let image_path = crate::store::directory().unwrap().join("recovered.png");
    std::fs::write(
        &image_path,
        include_bytes!("../assets/crew/ember/Normal.png"),
    )
    .unwrap();
    let image = crate::attachments::attach_file(&image_path).unwrap();
    let prompt = crate::attachments::Prompt::new("second".into(), vec![image.clone()]);
    let mut record = crate::attachments::prompt_record(&prompt, false).unwrap();
    record["type"] = json!("prompt");
    record["id"] = json!("s");
    record["streamingBehavior"] = json!("steer");
    record["message"] =
        json!("<shina_memory_diff>\nPrivate memory update\n</shina_memory_diff>\n\nsecond");
    client.command(record).unwrap();
    let sent: Vec<_> = client
        .wire
        .iter()
        .filter(|r| r["type"] == "user")
        .cloned()
        .collect();
    client.command(json!({"type":"abort","id":"stop"})).unwrap();
    let interrupt = client.wire.last().unwrap().clone();
    client
        .incoming(
            json!({"type":"control_response","response":{"subtype":"success",
        "request_id":interrupt["request_id"],"response":{"still_queued":[],
        "cancelled":[sent[1]["uuid"]]}}}),
        )
        .unwrap();
    let mut recovered = client
        .events
        .iter()
        .find(|r| r["command"] == "clear_queue")
        .unwrap()
        .clone();
    crate::attachments::import_history(&mut recovered);
    let mut thread = Thread::new(1);
    thread.harness = "claude".into();
    thread.apply(recovered);
    assert_eq!(thread.draft.text, "second");
    assert_eq!(thread.draft_images.len(), 1);
    assert_eq!(thread.draft_images[0].id, image.id);
    assert_eq!(
        client.wire.iter().filter(|r| r["type"] == "user").count(),
        2
    );
}

#[test]
fn claude_reports_manifest_descriptors_and_applies_options_and_1m_suffix() {
    let mut client = client("full-access");
    client
        .initialized(&json!({
            "hook_on_failure_supported": true,
            "hooks_applied": true,
            "commands": [],
            "models": [
                {"value": "claude-opus-5-5", "displayName": "Claude Opus 5.5", "supportedEffortLevels": ["low", "medium", "high", "xhigh", "max"]},
                {"value": "claude-opus-4-6", "displayName": "Claude Opus 4.6", "supportedEffortLevels": ["low", "medium", "high", "max"]}
            ]
        }))
        .unwrap();
    settings_response(&mut client, json!({"effective":{},"sources":[]}));
    let opus55 = client
        .models
        .iter()
        .find(|m| m["id"] == "claude-opus-5-5")
        .unwrap();
    let ids: Vec<&str> = opus55["capabilities"]["optionDescriptors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["id"].as_str())
        .collect();
    assert_eq!(ids, ["effort", "ultracode", "fastMode"]);
    client
        .command(json!({"type":"set_option","id":"o1","optionId":"fastMode","value":true}))
        .unwrap();
    let req = client.wire.last().unwrap();
    assert_eq!(req["request"]["subtype"], "apply_flag_settings");
    assert_eq!(req["request"]["settings"]["fastMode"], true);
}

#[test]
fn auto_mode_maps_to_claude_auto_permission_mode() {
    let client = Client::new(spec(SessionSource::New, "auto"));
    let mut command = Command::new("claude");
    client.launch(&mut command);
    let args: Vec<String> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let index = args
        .iter()
        .position(|arg| arg == "--permission-mode")
        .unwrap();
    assert_eq!(args[index + 1], "auto");

    let mut client = client;
    client
        .initialized(&json!({
            "hook_on_failure_supported": true,
            "hooks_applied": true,
            "commands": [],
            "models": []
        }))
        .unwrap();
    settings_response(&mut client, json!({"effective":{},"sources":[]}));
    client.wire.clear();
    client
        .command(json!({"type":"set_access_mode","accessMode":"auto"}))
        .unwrap();
    assert_eq!(client.wire[0]["request"]["subtype"], "set_permission_mode");
    assert_eq!(client.wire[0]["request"]["mode"], "auto");
}

#[test]
fn claude_context_breakdown_uses_get_context_usage_categories_on_completed_turn() {
    let mut client = client("read-only");
    client.running = true;
    client
        .incoming(json!({
            "type": "assistant",
            "uuid": "u1",
            "message": {
                "role": "assistant",
                "id": "m1",
                "model": "claude-sonnet-4-6",
                "content": [{"type": "text", "text": "Done"}],
                "usage": {
                    "input_tokens": 10000,
                    "cache_read_input_tokens": 5000,
                    "cache_creation_input_tokens": 2000,
                    "output_tokens": 500
                }
            }
        }))
        .unwrap();
    client.wire.clear();
    client
        .incoming(json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "queued_turn_count": 0,
            "modelUsage": {
                "claude-sonnet-4-6": { "contextWindow": 200000 }
            }
        }))
        .unwrap();
    let req = client
        .wire
        .iter()
        .find(|r| r["request"]["subtype"] == "get_context_usage")
        .expect("expected get_context_usage control_request")
        .clone();
    assert_eq!(req["request"]["detail"], "summary");
    let req_id = req["request_id"].as_str().unwrap().to_owned();

    client.events.clear();
    client
        .incoming(json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": req_id,
                "response": {
                    "categories": [
                        {"name": "System prompt", "tokens": 3200, "kind": "used"},
                        {"name": "Messages", "tokens": 14300, "kind": "used"},
                        {"name": "Autocompact buffer", "tokens": 30000, "kind": "buffer"},
                        {"name": "Free space", "tokens": 152500, "kind": "free"},
                        {"name": "Deferred tools", "tokens": 4100, "kind": "deferred"}
                    ]
                }
            }
        }))
        .unwrap();

    let mut thread = Thread::new(32);
    thread.harness = "claude".into();
    for event in client.events {
        thread.apply(event);
    }
    assert_eq!(
        thread.context.usage.breakdown,
        Some(vec![
            crate::context_breakdown::ContextBreakdownRow {
                label: "System prompt".into(),
                tokens: 3200,
                kind: crate::context_breakdown::BreakdownRowKind::Used,
            },
            crate::context_breakdown::ContextBreakdownRow {
                label: "Messages".into(),
                tokens: 14300,
                kind: crate::context_breakdown::BreakdownRowKind::Used,
            },
            crate::context_breakdown::ContextBreakdownRow {
                label: "Autocompact buffer".into(),
                tokens: 30000,
                kind: crate::context_breakdown::BreakdownRowKind::Buffer,
            },
        ])
    );
}

/// KSG `mergeClaudeProcessEnvironment`: CPA/provider env vars inherit
/// untouched, a nested session's variables are stripped, and the entrypoint is set.
#[test]
fn process_preserves_cpa_environment_and_strips_nested_session_vars() {
    let dir = crate::store::directory().unwrap();
    let cli = dir.join("claude-entry.js");
    std::fs::write(&cli, "// fixture").unwrap();
    let saved = std::env::var_os("CLAUDE_CONFIG_DIR");
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.join("claude-config")) };
    let command = resolve_process(vec![cli]).unwrap();
    match saved {
        Some(value) => unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", value) },
        None => unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") },
    }
    // CLIPROXYAPI/provider variables are inherited, never removed.
    for key in [
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_GIT_BASH_PATH",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
    ] {
        assert!(
            command.get_envs().all(|(name, _)| name != key),
            "{key} was removed"
        );
    }
    for removed in CLAUDE_SESSION_VARIABLES {
        assert!(
            command
                .get_envs()
                .any(|(name, value)| name == *removed && value.is_none()),
            "{removed} is still set"
        );
    }
    assert_eq!(
        command
            .get_envs()
            .find(|(name, _)| *name == "CLAUDE_CODE_ENTRYPOINT")
            .and_then(|(_, value)| value),
        Some(std::ffi::OsStr::new("shina"))
    );
    assert_eq!(
        command
            .get_envs()
            .find(|(name, _)| *name == "CLAUDE_CODE_STARTUP_FAILURE_RESULTS")
            .and_then(|(_, value)| value),
        Some(std::ffi::OsStr::new("1"))
    );
}
