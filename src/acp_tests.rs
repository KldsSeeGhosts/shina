use super::*;
use crate::model::{Status, Thread};
use std::sync::Arc;

#[test]
fn approval_options_and_wire_mapping_do_not_promote_decline_to_persistent_rejection() {
    let request = json!({"id":42,"params":{"sessionId":"session",
    "toolCall":{"toolCallId":"tool","kind":"execute","title":"Run it?"},
    "options":[
        {"optionId":"once","kind":"allow_once","name":"Once"},
        {"optionId":"always","kind":"allow_always","name":"Always"},
        {"optionId":"never","kind":"reject_always","name":"Never"}
    ]}});
    for (decision, expected) in [
        ("accept", json!({"outcome":"selected","optionId":"once"})),
        (
            "acceptForSession",
            json!({"outcome":"selected","optionId":"always"}),
        ),
        ("decline", json!({"outcome":"cancelled"})),
        ("cancel", json!({"outcome":"cancelled"})),
    ] {
        let mut client = Client::new(spec(SessionSource::New));
        client.permission(&request).unwrap();
        let key = client.events.last().unwrap()["id"].clone();
        client
            .command(json!({"type":"extension_ui_response","id":key,"decision":decision}))
            .unwrap();
        assert_eq!(client.wire.last().unwrap()["result"]["outcome"], expected);
    }
}

#[test]
fn provider_approval_option_lists_preserve_warnings_and_persistent_grant_restrictions() {
    let params = json!({"toolCall":{"toolCallId":"tool"},"options":[
        {"kind":"allow_once","optionId":"once"},
        {"kind":"allow_always","optionId":"always","_meta":{"agy.security.warning":{"message":" Check this grant. "}}},
        {"kind":"reject_once","optionId":"no"}
    ]});
    let options = acp_approval_options("antigravity", &params).unwrap();
    assert_eq!(
        options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(),
        ["Allow once", "Allow for this thread", "Deny", "Cancel"]
    );
    assert_eq!(options[1].warning.as_deref(), Some("Check this grant."));
    assert!(
        !acp_approval_options("kiro", &params)
            .unwrap()
            .iter()
            .any(|o| o.decision == "acceptForSession")
    );
    assert!(
        !acp_approval_options("grok", &params)
            .unwrap()
            .iter()
            .any(|o| o.decision == "acceptForSession")
    );
    assert!(acp_approval_options("generic-acp", &params).is_none());
}
#[test]
fn steering_requires_the_exact_identity_and_ownership_advertisement() {
    for advertisement in [
        Value::Null,
        json!(true),
        json!({"version":1}),
        json!({"version":2,"idempotent":true,"holdsPrompt":true}),
        json!({"version":1,"idempotent":false,"holdsPrompt":true}),
        json!({"version":1,"idempotent":true,"holdsPrompt":false}),
    ] {
        assert!(!supports_active_steering(
            &json!({"t3code/steering":advertisement})
        ));
    }
    assert!(supports_active_steering(&json!({"t3code/steering":{
        "version":1,"idempotent":true,"holdsPrompt":true,"futureField":true
    }})));
    let mut start = spec(SessionSource::New);
    start.id = "github-copilot-cli".into();
    let client = Client::new(start);
    assert!(!client.turn_capabilities().supports_active_steering);
    assert!(
        !client
            .turn_capabilities()
            .supports_steering_by_interrupt_restart
    );
}

#[test]
fn native_steering_uses_the_prompt_turn_id_and_never_replays_an_uncertain_delivery() {
    let mut client = Client::new(spec(SessionSource::New));
    client.ready = true;
    client.session = Some("fixture-session".into());
    client.active_steering = true;
    client
        .prompt(json!({"type":"prompt","id":"first","message":"Start"}))
        .unwrap();
    let prompt = client
        .wire
        .iter()
        .find(|request| request["method"] == "session/prompt")
        .unwrap();
    let turn = prompt["params"]["_meta"]["t3code/turnId"].clone();
    assert!(turn.is_string());
    client
        .command(
            json!({"type":"prompt","id":"steer-a","message":"Steer","streamingBehavior":"steer"}),
        )
        .unwrap();
    let steer = client
        .wire
        .iter()
        .find(|request| request["method"] == "_t3/steer")
        .unwrap()
        .clone();
    assert_eq!(steer["params"]["turnId"], turn);
    assert_eq!(steer["params"]["messageId"], "steer-a");
    client
        .incoming(
            json!({"jsonrpc":"2.0","id":steer["id"],"result":{"status":"unknown-future-status"}}),
        )
        .unwrap();
    assert!(client.queued.is_empty());
    assert!(
        client
            .events
            .iter()
            .any(|event| event["type"] == "steer_delivery_unknown")
    );
}

#[test]
fn standard_acp_steering_restarts_after_cancellation_but_stop_wins_the_race() {
    for stop in [false, true] {
        let mut client = Client::new(spec(SessionSource::New));
        client.ready = true;
        client.session = Some("fixture-session".into());
        client
            .prompt(json!({"type":"prompt","id":"first","message":"Start"}))
            .unwrap();
        client.command(json!({"type":"prompt","id":"steer","message":"Redirect","streamingBehavior":"steer"})).unwrap();
        assert!(client.wire.iter().any(|r| r["method"] == "session/cancel"));
        if stop {
            client
                .command(json!({"type":"clear_queue","id":"clear"}))
                .unwrap();
            client.command(json!({"type":"abort","id":"stop"})).unwrap();
        }
        client.settle(true);
        let prompts = client
            .wire
            .iter()
            .filter(|r| r["method"] == "session/prompt")
            .count();
        assert_eq!(prompts, if stop { 1 } else { 2 });
        assert_eq!(
            client
                .events
                .iter()
                .any(|event| event["type"] == "steering_restart"),
            !stop
        );
    }
}

#[test]
fn typed_calls_preserve_acp_kind_and_diff_before_flattening() {
    use crate::work::ToolKind::*;
    for (native, expected) in [
        ("execute", Command),
        ("edit", FileChange),
        ("search", FileSearch),
        ("fetch", WebSearch),
        ("read", Read),
        ("list", List),
        ("subagent", Subagent),
        ("plan", Plan),
        ("other", Other),
    ] {
        let mut client = Client::new(spec(SessionSource::New));
        client.ready = true;
        client.running = true;
        client.update(&json!({"sessionUpdate":"tool_call","toolCallId":"typed","title":"A provider title","kind":native,
            "rawInput":{"command":"pwd","pattern":"foo"},"status":"completed",
            "rawOutput":{"results":[{"fileName":"a.rs","line":3,"preview":"foo"}]},
            "content":[{"type":"diff","path":"a.rs","oldText":"old\n","newText":"new\n"}]}));
        let mut live = Thread::new(1);
        for event in client.events {
            live.apply(event);
        }
        let mut restored = Thread::new(2);
        restored.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":client.history}}));
        for thread in [&live, &restored] {
            let tool = &thread.tools[0];
            assert_eq!(tool.kind, Some(expected), "{native}");
            assert_eq!(tool.changes[0].diff, "@@ -1 +1 @@\n-old\n+new\n");
        }
    }
}

#[test]
fn dollar_commands_are_skills_and_claude_dispatch_ends_with_the_native_invocation() {
    let mut client = Client::new(spec(SessionSource::New));
    client.ready = true;
    client.session = Some("fixture-session".into());
    client.awareness_sent = true;
    client.update(&json!({"sessionUpdate":"available_commands_update",
        "availableCommands":[{"name":"$review","description":"Review changes"}]}));
    assert_eq!(
        crate::composer_skills::from_commands(&client.available_commands())[0].name,
        "review"
    );
    client.claude_skills = vec![crate::composer_skills::Skill {
        name: "review".into(),
        description: String::new(),
        path: Some("/skills/review/SKILL.md".into()),
        user_only: false,
    }];
    let path = crate::store::directory().unwrap().join("notes.txt");
    std::fs::write(&path, "notes").unwrap();
    let attachment = crate::attachments::reference_file(&path).unwrap();
    client
        .prompt(json!({"type":"prompt","message":"please $review these",
        "attachments":[attachment.to_value()]}))
        .unwrap();
    let request = client
        .wire
        .iter()
        .find(|r| r["method"] == "session/prompt")
        .unwrap();
    let blocks = request["params"]["prompt"].as_array().unwrap();
    assert_eq!(blocks[0]["text"], "please /review these");
    assert!(
        blocks[1]["text"]
            .as_str()
            .unwrap()
            .contains("[Attached file")
    );
    assert_eq!(blocks.last().unwrap()["text"], "/review these");
}

#[test]
fn file_prompts_use_negotiated_resources_or_path_text() {
    let directory = crate::store::directory().unwrap();
    let path = directory.join("resource.txt");
    std::fs::write(&path, "Resource body").unwrap();
    let attachment = crate::attachments::attach_file(&path).unwrap();
    for embedded in [false, true] {
        let mut client = Client::new(spec(SessionSource::New));
        client.ready = true;
        client.session = Some("fixture-session".into());
        client.capabilities = json!({"promptCapabilities":{"embeddedContext":embedded}});
        let prompt = crate::attachments::Prompt::new(String::new(), vec![attachment.clone()]);
        let record = crate::attachments::prompt_record(&prompt, false).unwrap();
        client.prompt(record).unwrap();
        let wire = client
            .wire
            .iter()
            .find(|r| r["method"] == "session/prompt")
            .unwrap();
        let content = wire["params"]["prompt"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        if embedded {
            assert_eq!(content[1]["type"], "resource");
            assert_eq!(content[1]["resource"]["text"], "Resource body");
        } else {
            assert_eq!(content[1]["type"], "text");
            assert!(
                content[1]["text"]
                    .as_str()
                    .unwrap()
                    .contains("resource.txt")
            );
        }
        assert!(client.running);
        assert_eq!(client.history[0]["content"][1], content[1]);
    }
}

#[test]
fn acp_plan_updates_keep_the_original_position_and_replay_the_latest_steps() {
    let mut client = Client::new(spec(SessionSource::New));
    client.ready = true;
    client
        .update(&json!({"sessionUpdate":"plan","entries":[{"content":"Read","status":"pending"}]}));
    client.update(&json!({"sessionUpdate":"tool_call","toolCallId":"read","title":"Read","kind":"read","status":"completed","rawInput":{"path":"a.rs"}}));
    client.update(&json!({"sessionUpdate":"plan_update","entries":[{"content":"Read","status":"completed"},{"content":"Write","status":"in_progress"}]}));
    let mut live = Thread::new(1);
    for event in client.events {
        live.apply(event);
    }
    let mut replayed = Thread::new(2);
    replayed.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":client.history}}));
    for t in [&live, &replayed] {
        assert_eq!(t.tools.len(), 2);
        assert_eq!(t.tools[0].resolved_kind(), crate::work::ToolKind::Plan);
        assert_eq!(t.tools[1].resolved_kind(), crate::work::ToolKind::Read);
        let plan = crate::work::plan_data(&t.tools[0].input);
        assert_eq!(plan["steps"][0]["status"], "completed");
        assert_eq!(plan["steps"][1]["status"], "running");
    }
}

#[test]
fn acp_diff_parts_keep_native_unified_patches_without_old_new_text() {
    let patch = "@@ -8 +8 @@\n-old\n+new\n";
    let mut t = Thread::new(1);
    t.apply(json!({"type":"tool_execution_start","toolCallId":"edit","toolName":"Provider title","args":{}}));
    let result = tool_result(
        &json!({"kind":"move","content":[{"type":"diff","path":"new.rs",
        "oldPath":"old.rs","patch":{"format":"unified","text":patch}}]}),
    );
    t.apply(json!({"type":"tool_execution_end","toolCallId":"edit","result":result}));
    assert_eq!(t.tools[0].changes[0].diff, patch);
    assert_eq!(t.tools[0].changes[0].old_path.as_deref(), Some("old.rs"));
    assert_eq!(
        (
            t.tools[0].changes[0].additions,
            t.tools[0].changes[0].deletions
        ),
        (1, 1)
    );
}

fn spec(source: SessionSource) -> Start {
    Start {
        id: "fixture".into(),
        workspace: crate::store::directory().unwrap(),
        source,
        instructions: Some("Captain fixture instructions".into()),
        link: Some(HostLink {
            url: "http://127.0.0.1:12345/mcp".into(),
            token: "fixture-thread-token".into(),
            extension: PathBuf::new(),
            access_mode: "approval-required".into(),
        }),
        owner: Arc::default(),
        context_sent: false,
    }
}

fn fixture(flags: &[&str]) -> (Agent, PathBuf) {
    let dir = crate::store::directory().unwrap();
    let script = dir.join("fake_acp.py");
    std::fs::write(&script, include_str!("../assets/tests/fake_acp.py")).unwrap();
    let log = dir.join(format!("acp-{}.jsonl", flags.join("-").replace("--", "")));
    let mut args = vec![
        "-u".into(),
        script.to_string_lossy().into_owned(),
        "--log".into(),
        log.to_string_lossy().into_owned(),
    ];
    args.extend(flags.iter().map(|s| (*s).into()));
    (
        Agent {
            id: "fixture".into(),
            name: "Fixture".into(),
            command: if cfg!(windows) { "python" } else { "python3" }.into(),
            args,
            env: vec![],
        },
        log,
    )
}

fn pump(thread: &mut Thread, predicate: impl Fn(&Thread) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut diagnostics = vec![];
    while !predicate(thread) {
        assert!(
            Instant::now() < deadline,
            "ACP timed out: {:?}, {}",
            thread.status,
            thread.error.as_deref().unwrap_or("")
        );
        let event = thread
            .pi
            .as_ref()
            .unwrap()
            .events
            .recv_timeout(Duration::from_millis(100));
        match event {
            Ok(Event::Record(record)) => thread.apply(record),
            Ok(Event::Error(error)) => {
                panic!("ACP fixture failed: {error}\n{}", diagnostics.join("\n"))
            }
            Ok(Event::Diagnostic(message)) => diagnostics.push(message),
            Ok(Event::Exited) => panic!("ACP fixture exited unexpectedly"),
            _ => {}
        }
    }
}

fn records(log: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn live_thread(agent: Agent, source: SessionSource) -> Thread {
    let dir = crate::store::directory().unwrap();
    std::fs::write(dir.join("harnesses.json"), json!([{
        "id":agent.id,"name":agent.name,"command":agent.command,"args":agent.args,"env":agent.env,
    }]).to_string()).unwrap();
    let mut thread = Thread::new(31);
    thread.harness = "fixture".into();
    if let SessionSource::Resume(session) = source {
        thread.session = Some(session);
    }
    thread.host = spec(SessionSource::New).link;
    thread.native.access = crate::model::AccessMode::Supervised;
    thread.connect_with_instructions(
        dir,
        Arc::new(|| {}),
        Some("Captain fixture instructions".into()),
    );
    pump(&mut thread, |t| {
        t.native.commands_loaded && !t.harness_state["modes"].is_null()
    });
    thread
}

#[test]
fn fake_agent_initialize_mcp_stream_permission_cancel_and_load_round_trip() {
    let (agent, log) = fixture(&[]);
    let mut thread = live_thread(agent.clone(), SessionSource::New);
    assert_eq!(thread.session.as_deref(), Some("fixture-session"));
    assert_eq!(thread.models[0].provider, "fixture");
    assert_eq!(thread.native.commands[0].name, "fixture");
    assert!(thread.harness_state["plan"]["entries"].is_array());
    assert_eq!(thread.harness_state["modes"]["currentModeId"], "code");
    thread.draft.replace_text("permission");
    thread.send();
    pump(&mut thread, |t| t.request.is_some());
    assert_eq!(thread.status, Status::Waiting);
    assert_eq!(thread.request.as_ref().unwrap()["method"], "confirm");
    thread.respond(json!({"confirmed":true}));
    pump(&mut thread, |t| t.status == Status::Completed);
    assert!(thread.messages.iter().any(|m| m.text == "Hello ACP"));
    assert_eq!(thread.thoughts[0].text, "Checking");
    let write = thread
        .tools
        .iter()
        .find(|t| t.resolved_kind() == crate::work::ToolKind::FileChange)
        .unwrap();
    assert!(write.done);
    assert_eq!(write.output, "Written");
    assert!(thread.harness_context_sent);
    let wire = records(&log);
    let prompt = wire
        .iter()
        .find(|r| r["method"] == "session/prompt")
        .unwrap();
    let context = prompt["params"]["prompt"][0]["text"].as_str().unwrap();
    assert!(context.contains(crate::mcp::INSTRUCTIONS));
    assert!(context.contains("Captain fixture instructions"));
    assert_eq!(
        wire.iter()
            .find(|r| r["id"] == 71 && r.get("result").is_some())
            .unwrap()["result"]["outcome"]["optionId"],
        "yes"
    );

    thread.draft.replace_text("again");
    thread.send();
    pump(&mut thread, |t| t.status == Status::Completed);
    let wire = records(&log);
    assert_eq!(
        wire.iter()
            .rfind(|r| r["method"] == "session/prompt")
            .unwrap()["params"]["prompt"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    thread.draft.replace_text("hold");
    thread.send();
    pump(&mut thread, |t| {
        t.messages.iter().any(|m| m.text == "Waiting")
    });
    thread.stop();
    pump(&mut thread, |t| t.status == Status::Ready);
    assert!(
        records(&log)
            .iter()
            .any(|r| r["method"] == "session/cancel" && r.get("id").is_none())
    );
    thread.disconnect();
    thread.connect_with_instructions(crate::store::directory().unwrap(), Arc::new(|| {}), None);
    pump(&mut thread, |t| {
        t.messages.iter().any(|m| m.text == "Earlier reply")
    });
    assert_eq!(thread.messages[0].text, "Earlier user");
    assert_eq!(
        records(&log)
            .iter()
            .filter(|r| r["method"] == "session/load")
            .count(),
        1
    );
    thread.draft.replace_text("resumed");
    thread.send();
    pump(&mut thread, |t| t.status == Status::Completed);
    assert_eq!(
        records(&log)
            .iter()
            .rfind(|r| r["method"] == "session/prompt")
            .unwrap()["params"]["prompt"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn fake_agent_stdio_fallback_and_resume_without_load() {
    let (agent, log) = fixture(&["--stdio", "--resume"]);
    let mut thread = live_thread(agent, SessionSource::Resume("fixture-session".into()));
    assert!(thread.messages.iter().any(|m| m.text == "Earlier reply"));
    assert!(
        records(&log)
            .iter()
            .any(|r| r["method"] == "session/resume")
    );
    thread.disconnect();
}

#[test]
fn fake_agent_authentication_denial_does_not_authenticate_or_retry() {
    let (agent, log) = fixture(&["--auth-denied"]);
    let pi = start(agent, spec(SessionSource::New), Arc::new(|| {})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline);
        if let Ok(Event::Error(error)) = pi.events.recv_timeout(Duration::from_millis(100)) {
            assert!(error.contains("Authentication required"));
            break;
        }
    }
    let wire = records(&log);
    assert_eq!(
        wire.iter().filter(|r| r["method"] == "session/new").count(),
        1
    );
    assert!(!wire.iter().any(|r| r["method"] == "authenticate"));
}

fn negotiate(client: &mut Client, caps: Value) {
    client.initialize();
    let init = client.wire.pop().unwrap();
    client.incoming(json!({"jsonrpc":"2.0","id":init["id"],"result":{"protocolVersion":1,"agentCapabilities":caps}})).unwrap();
}

fn ready(client: &mut Client) {
    negotiate(
        client,
        json!({"mcpCapabilities":{"http":true},"promptCapabilities":{"image":true},"loadSession":true}),
    );
    let new = client.wire.pop().unwrap();
    assert_eq!(
        new["params"]["mcpServers"][0]["headers"][0]["value"],
        "Bearer fixture-thread-token"
    );
    client
        .incoming(json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"fixture-session"}}))
        .unwrap();
    client.events.clear();
}

#[test]
fn capabilities_images_model_selection_queue_and_unknown_requests() {
    let mut client = Client::new(spec(SessionSource::New));
    ready(&mut client);
    client.command(json!({"type":"prompt","id":"image","message":"look","images":[{"type":"image","mimeType":"image/png","data":"fixture-base64"}]})).unwrap();
    let prompt = client.wire.pop().unwrap();
    assert_eq!(prompt["params"]["prompt"][2]["mimeType"], "image/png");
    client
        .command(
            json!({"type":"prompt","id":"queued","message":"next","streamingBehavior":"followUp"}),
        )
        .unwrap();
    assert_eq!(client.queued.len(), 1);
    assert!(client.wire.is_empty());
    client
        .incoming(json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}}))
        .unwrap();
    assert!(
        client
            .wire
            .iter()
            .any(|r| r["method"] == "session/prompt" && r["params"]["prompt"][0]["text"] == "next")
    );
    client
        .incoming(json!({"jsonrpc":"2.0","id":91,"method":"fs/read_text_file","params":{}}))
        .unwrap();
    assert_eq!(client.wire.last().unwrap()["error"]["code"], -32601);
    client
        .command(json!({"type":"compact","id":"unsupported"}))
        .unwrap();
    assert!(
        client.events.last().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("unavailable for ACP")
    );

    client.configure_models(&json!({"configOptions":[{"category":"model","id":"model","type":"select","currentValue":"a","options":[{"value":"a","name":"A"}]}]}));
    client.running = false;
    client
        .command(json!({"type":"set_model","id":"select","modelId":"a"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["method"],
        "session/set_config_option"
    );
    client.capabilities["promptCapabilities"]["image"] = json!(false);
    client
        .command(json!({"type":"prompt","id":"bad-image","images":[{"type":"image"}]}))
        .unwrap();
    assert_eq!(client.events.last().unwrap()["success"], false);
}

#[test]
fn permissions_honor_access_mode_and_cancellation() {
    for (mode, kind, automatic) in [
        ("approval-required", "execute", false),
        ("approval-required", "read", true),
        ("auto-accept-edits", "edit", true),
        ("auto-accept-edits", "execute", false),
        ("full-access", "execute", true),
    ] {
        let mut start = spec(SessionSource::New);
        start.link.as_mut().unwrap().access_mode = mode.into();
        let mut client = Client::new(start);
        ready(&mut client);
        client.incoming(json!({"jsonrpc":"2.0","id":5,"method":"session/request_permission","params":{
            "sessionId":"fixture-session","toolCall":{"kind":kind,"title":"Fixture"},
            "options":[{"kind":"allow_once","optionId":"yes"},{"kind":"reject_once","optionId":"no"}]
        }})).unwrap();
        if automatic {
            assert_eq!(
                client.wire.last().unwrap()["result"]["outcome"]["optionId"],
                "yes"
            );
        } else {
            let request = client.events.last().unwrap().clone();
            client
                .command(
                    json!({"type":"extension_ui_response","id":request["id"],"confirmed":false}),
                )
                .unwrap();
            assert_eq!(
                client.wire.last().unwrap()["result"]["outcome"]["optionId"],
                "no"
            );
        }
    }
}

#[test]
fn unsupported_resume_and_mcp_and_protocol_versions_fail_clearly() {
    let mut client = Client::new(spec(SessionSource::Resume("fixture-session".into())));
    client.initialize();
    let request = client.wire.pop().unwrap();
    let error = client.incoming(json!({"jsonrpc":"2.0","id":request["id"],"result":{"protocolVersion":1,"agentCapabilities":{"mcpCapabilities":{"http":true}}}})).unwrap_err();
    assert!(error.contains("session/load"));
    let mut client = Client::new(spec(SessionSource::New));
    client.capabilities = json!({"mcpCapabilities":{"stdio":false,"sse":true}});
    assert!(
        client
            .mcp_servers()
            .unwrap_err()
            .contains("neither HTTP nor stdio")
    );
    client.initialize();
    let request = client.wire.pop().unwrap();
    assert!(
        client
            .incoming(json!({"jsonrpc":"2.0","id":request["id"],"result":{"protocolVersion":2}}))
            .unwrap_err()
            .contains("version 1")
    );
}

#[test]
fn rejected_http_mcp_retries_stdio_once_but_authentication_denials_do_not() {
    let mut client = Client::new(spec(SessionSource::New));
    negotiate(
        &mut client,
        json!({"mcpCapabilities":{"http":true,"stdio":true}}),
    );
    let new = client.wire.pop().unwrap();
    client.incoming(json!({"jsonrpc":"2.0","id":new["id"],"error":{"code":-32602,"message":"HTTP MCP unsupported"}})).unwrap();
    let retry = client.wire.pop().unwrap();
    assert_eq!(
        retry["params"]["mcpServers"][0]["args"],
        json!(["--mcp-bridge"])
    );
    assert!(client.incoming(json!({"jsonrpc":"2.0","id":retry["id"],"error":{"code":-32602,"message":"Unsupported MCP"}})).is_err());
    assert!(client.wire.is_empty());
}

#[test]
fn cancellation_releases_pending_permissions_and_clears_internal_followups() {
    let mut client = Client::new(spec(SessionSource::New));
    ready(&mut client);
    client
        .command(json!({"type":"prompt","id":"run","message":"work"}))
        .unwrap();
    client
        .command(json!({"type":"prompt","id":"follow","message":"next"}))
        .unwrap();
    client.incoming(json!({"jsonrpc":"2.0","id":77,"method":"session/request_permission","params":{
        "sessionId":"fixture-session","toolCall":{"kind":"execute"},"options":[{"kind":"allow_once","optionId":"yes"}]
    }})).unwrap();
    assert_eq!(client.permissions.len(), 1);
    client
        .command(json!({"type":"clear_queue","id":"clear"}))
        .unwrap();
    assert!(client.queued.is_empty());
    client
        .command(json!({"type":"abort","id":"cancel"}))
        .unwrap();
    assert!(client.permissions.is_empty());
    assert!(
        client
            .wire
            .iter()
            .any(|r| r["id"] == 77 && r["result"]["outcome"]["outcome"] == "cancelled")
    );
    assert_eq!(client.wire.last().unwrap()["method"], "session/cancel");
}

#[test]
fn http_response_reads_exact_content_length_without_waiting_for_eof() {
    let mut input = std::io::Cursor::new(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}extra");
    assert_eq!(http_response(&mut input).unwrap(), (200, b"{}".to_vec()));
    let mut remaining = String::new();
    input.read_to_string(&mut remaining).unwrap();
    assert_eq!(remaining, "extra");
    for invalid in [
        "HTTP/1.1 200 OK\r\nContent-Length: 9000000\r\n\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{",
    ] {
        assert!(http_response(&mut std::io::Cursor::new(invalid)).is_err());
    }
}

#[test]
fn mcp_stdio_relay_returns_server_instructions_with_thread_authorization() {
    let server = crate::mcp::server(Arc::new(|| {})).unwrap();
    let host = HostLink {
        url: server.url.clone(),
        token: server.token_for(8181),
        extension: PathBuf::new(),
        access_mode: "full-access".into(),
    };
    let result = relay(
        &host,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    )
    .unwrap()
    .unwrap();
    assert_eq!(result["result"]["instructions"], crate::mcp::INSTRUCTIONS);
    let bad = HostLink {
        token: "wrong".into(),
        ..host
    };
    assert!(relay(&bad, &json!({"jsonrpc":"2.0","id":1,"method":"initialize"})).is_err());
}

#[test]
fn replayed_awareness_is_hidden_and_not_repeated_after_resume() {
    let mut client = Client::new(spec(SessionSource::Resume("fixture-session".into())));
    client.update(&json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":format!("<shina_context>\n{}\n</shina_context>", crate::mcp::INSTRUCTIONS)}}));
    assert!(client.history.is_empty());
    assert!(client.awareness_sent);
    ready(&mut client);
    client
        .command(json!({"type":"prompt","id":"next","message":"Continue"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["params"]["prompt"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn acp_reports_option_descriptors_and_applies_config_and_mode_options() {
    let mut client = Client::new(spec(SessionSource::New));
    client.ready = true;
    client.session = Some("acp-session".into());
    client.configure_models(&json!({
        "models": {
            "currentModelId": "m1",
            "availableModels": [{"modelId": "m1", "name": "Model 1"}]
        },
        "configOptions": [
            {
                "id": "thought_level",
                "name": "Thinking",
                "category": "thought_level",
                "type": "select",
                "currentValue": "low",
                "options": [{"value": "low", "name": "Low"}, {"value": "high", "name": "High"}]
            }
        ],
        "modes": {
            "currentModeId": "code",
            "availableModes": [{"id": "code", "name": "Code"}, {"id": "plan", "name": "Plan"}]
        }
    }));
    assert!(
        !client.models[0]["capabilities"]["optionDescriptors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client
        .command(
            json!({"type":"set_option","id":"opt-1","optionId":"_t3/session-mode","value":"plan"}),
        )
        .unwrap();
    assert_eq!(client.wire.last().unwrap()["method"], "session/set_mode");
    assert_eq!(client.wire.last().unwrap()["params"]["modeId"], "plan");

    client
        .command(json!({"type":"set_thinking_level","id":"opt-2","level":"high"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["method"],
        "session/set_config_option"
    );
    assert_eq!(
        client.wire.last().unwrap()["params"]["configId"],
        "thought_level"
    );
    assert_eq!(client.wire.last().unwrap()["params"]["value"], "high");
}
