use super::*;

#[test]
fn antigravity_interactions_are_choice_only_questions_even_under_full_access() {
    let mut client = Client::new(spec(SessionSource::New));
    client.start.id = "antigravity".into();
    client.start.link.as_mut().unwrap().access_mode = "full-access".into();
    client.ready = true;
    client.session = Some("fixture-session".into());
    client
        .permission(&json!({"id":71,"params":{"sessionId":"fixture-session",
        "toolCall":{"toolCallId":"interaction_fixture","title":"  Choose **one**. ","kind":"read"},
        "options":[{"kind":"allow_once","optionId":" exact ","name":" First "},
            {"kind":"reject_once","optionId":"second","name":"Second"}]}}))
        .unwrap();
    let request = client
        .events
        .last()
        .expect("question instead of automatic approval")
        .clone();
    assert_eq!(request["method"], "input");
    assert_eq!(request["questions"][0]["allowCustomAnswer"], false);
    assert_eq!(request["questions"][0]["options"][0]["value"], " exact ");
    assert_eq!(request["questions"][0]["question"], "Choose **one**.");
    client
        .command(json!({"type":"extension_ui_response","id":request["id"],
        "value":json!({"interaction_fixture":" exact "}).to_string()}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["result"]["outcome"]["optionId"],
        " exact "
    );
}
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

fn devin_client() -> Client {
    let mut start = spec(SessionSource::New);
    start.id = "devin".into();
    let mut client = Client::new(start);
    client.ready = true;
    client.session = Some("root-session".into());
    client
}

#[test]
fn devin_updates_route_child_work_under_namespaced_tool_ids() {
    let mut client = devin_client();
    client.running = true;
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"root-session","update":{"sessionUpdate":"tool_call",
            "toolCallId":"shared-tool","title":"Root call","kind":"execute","status":"in_progress"}}}))
        .unwrap();
    // The same toolCallId under a subagent context reroutes and namespaces like
    // KSG's `{nativeTaskId}:tool:{toolCallId}`.
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"root-session","update":{"sessionUpdate":"tool_call",
            "toolCallId":"shared-tool","title":"Child call","kind":"execute","status":"completed",
            "_meta":{"cognition.ai/subagent_context":{"parentAgentId":"child-1"}}}}}))
        .unwrap();
    let starts: Vec<&Value> = client
        .events
        .iter()
        .filter(|event| event["type"] == "tool_execution_start")
        .collect();
    let ids: Vec<&str> = starts
        .iter()
        .filter_map(|event| event["toolCallId"].as_str())
        .collect();
    assert!(ids.contains(&"shared-tool"), "{ids:?}");
    assert!(ids.contains(&"child-1:shared-tool"), "{ids:?}");
    // A direct child-session update is accepted once the child id is known.
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"child-1","update":{"sessionUpdate":"tool_call_update",
            "toolCallId":"extra","title":"More","status":"completed"}}}))
        .unwrap();
    assert!(client.tools.contains_key("child-1:extra"));
    // An unknown session still fails.
    assert!(
        client
            .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
                "sessionId":"stranger","update":{"sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"?"}}}}))
            .is_err()
    );
}

#[test]
fn devin_terminal_requests_create_capture_and_wait() {
    let mut client = devin_client();
    client.terminal_request(
        "terminal/create",
        &json!({"id":1,"params":{"sessionId":"root-session","command":"echo from-terminal"}}),
    );
    let terminal = client.wire.last().unwrap()["result"]["terminalId"]
        .as_str()
        .unwrap()
        .to_owned();
    client.terminal_request(
        "terminal/wait_for_exit",
        &json!({"id":2,"params":{"sessionId":"root-session","terminalId":terminal}}),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !client.terminal_waiters.is_empty() {
        client.flush_terminal_waiters();
        assert!(Instant::now() < deadline, "terminal waiter never resolved");
        std::thread::sleep(Duration::from_millis(10));
    }
    let response = client.wire.iter().find(|record| record["id"] == 2).unwrap();
    assert_eq!(response["result"]["exitCode"], 0);
    client.terminal_request(
        "terminal/output",
        &json!({"id":3,"params":{"sessionId":"root-session","terminalId":terminal}}),
    );
    let output = &client.wire.last().unwrap()["result"];
    assert!(
        output["output"].as_str().unwrap().contains("from-terminal"),
        "{output}"
    );
    // A non-devin client never advertised terminals and refuses the methods.
    let mut client = Client::new(spec(SessionSource::New));
    client
        .incoming(
            json!({"jsonrpc":"2.0","id":4,"method":"terminal/create","params":{
            "sessionId":"s","command":"echo no"}}),
        )
        .unwrap();
    assert_eq!(client.wire.last().unwrap()["error"]["code"], -32601);
}

fn spec(source: SessionSource) -> Start {
    Start {
        id: "fixture".into(),
        workspace: crate::store::directory().unwrap(),
        source,
        instructions: Some("Captain fixture instructions".into()),
        shina: crate::mcp::INSTRUCTIONS,
        link: Some(HostLink {
            url: "http://127.0.0.1:12345/mcp".into(),
            token: "fixture-thread-token".into(),
            extension: PathBuf::new(),
            access_mode: "approval-required".into(),
        }),
        owner: Arc::default(),
        context_sent: false,
        prompt_suggestions: false,
        metadata_only: false,
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
            Ok(Event::ProviderJob(_)) => {}
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
fn antigravity_linkless_discovery_returns_models_and_commands() {
    let (mut agent, log) = fixture(&["--antigravity", "--discovery", "--no-mcp"]);
    agent.id = "antigravity".into();
    let start = Start::discovery(&agent.id, crate::store::directory().unwrap());
    let pi = super::start(agent, start, Arc::new(|| {})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut models, mut commands) = (false, false);
    let mut diagnostics = vec![];
    while !models || !commands {
        match pi
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(Event::Record(record)) if record["command"] == "get_available_models" => {
                assert_eq!(record["success"], true, "{record}");
                assert!(
                    record["data"]["models"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|model| model["id"] == "gemini-3.8-flash-high"),
                    "{record}"
                );
                models = true;
            }
            Ok(Event::Record(record)) if record["command"] == "get_commands" => {
                assert_eq!(record["success"], true, "{record}");
                assert_eq!(record["data"]["commands"][0]["name"], "fixture");
                commands = true;
            }
            Ok(Event::Record(_)) => {}
            Ok(Event::Diagnostic(message)) => diagnostics.push(message),
            Ok(Event::ProviderJob(_)) => {}
            Ok(Event::Error(error)) => panic!("ACP discovery failed: {error}\n{diagnostics:?}"),
            Ok(Event::Exited) => panic!("ACP discovery exited before returning its catalog"),
            Err(error) => {
                panic!("ACP discovery did not return its catalog: {error}\n{diagnostics:?}")
            }
        }
    }
    let wire = records(&log);
    assert_eq!(
        wire.iter()
            .filter_map(|r| r["method"].as_str())
            .collect::<Vec<_>>(),
        ["initialize", "authenticate", "session/new"]
    );
    assert_eq!(wire[2]["params"]["mcpServers"], json!([]));
}

#[test]
fn acp_discovery_does_not_retry_or_authenticate_after_a_denial() {
    let (agent, log) = fixture(&["--discovery", "--auth-denied"]);
    let dir = crate::store::directory().unwrap();
    let mut catalog = crate::session::NativeCatalog::default();
    let wake: Waker = Arc::new(|| {});
    assert!(catalog.refresh_acp(&agent, &dir, wake.clone(), false, false));
    let deadline = Instant::now() + Duration::from_secs(10);
    while catalog.loading() {
        assert!(Instant::now() < deadline);
        catalog.poll(&agent.id);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        catalog
            .error
            .as_deref()
            .unwrap()
            .contains("sign in to the agent outside Shina")
    );
    assert!(!catalog.refresh_acp(&agent, &dir, wake, false, false));
    assert_eq!(
        records(&log)
            .iter()
            .map(|record| record["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["initialize", "session/new"]
    );
}

#[test]
fn acp_model_options_remain_per_model_after_late_updates_and_singleton_choices() {
    let mut client = Client::new(spec(SessionSource::New));
    client.ready = true;
    client.configure_models(&json!({
        "models":{"currentModelId":"first","availableModels":[
           {"modelId":"first","name":"First"},
           {"modelId":"second","name":"Second","_meta":{
               "reasoningEffort":"low","reasoningEfforts":[{"value":"low","label":"Quick"},{"value":"high"}]}},
           {"modelId":"none","name":"None","_meta":{"supportsReasoningEffort":false}}
        ]},
        "configOptions":[
           {"id":"r","category":"thinking","type":"select","name":"Thinking","currentValue":"medium",
            "options":[{"value":"medium","name":"Medium"}]},
           {"id":"s","category":"model_config","type":"boolean","name":"Specific","currentValue":true},
           {"id":"shared","type":"boolean","name":"Shared","currentValue":true}
        ]
    }));
    let ids = |row: &Value| {
        row["capabilities"]["optionDescriptors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&client.models[0]), ["r", "s", "shared"]);
    assert_eq!(ids(&client.models[1]), ["shared", "r"]);
    assert_eq!(ids(&client.models[2]), ["shared"]);
    assert_eq!(client.models[1]["thinkingLevels"], json!(["low", "high"]));
    assert_eq!(
        client.models[1]["capabilities"]["optionDescriptors"][1]["options"][0]["label"],
        "Quick"
    );

    client.configure_models(
        &json!({"models":{"currentModelId":"first","availableModels":[
            {"modelId":"first","name":"Provisional"}
        ]}}),
    );
    assert_eq!(client.models.len(), 3, "a legacy singleton is provisional");
    assert_eq!(ids(&client.models[1]), ["shared", "r"]);
    client.update(
        &json!({"sessionUpdate":"config_option_update","configOptions":[
            {"id":"model","type":"select","category":"model","currentValue":"second",
             "options":[{"value":"second","name":"Only second"}]},
        ]}),
    );
    assert_eq!(
        client.models.len(),
        1,
        "an explicit singleton replaces the previous inventory"
    );
    assert_eq!(client.model["id"], "second");
    assert_eq!(
        client.events.last().unwrap()["data"]["models"][0]["name"],
        "Only second"
    );
    assert_eq!(
        client.events.last().unwrap()["data"]["models"][0]["isDefault"],
        true
    );
}

/// KSG applyAntigravityAcpModelSelection, gateway branch: family-level picks send
/// the family's default effort route through the model config option, the
/// adapter-owned reasoningEffort pick re-routes the current family, and a
/// `family-medium` the agent does not list retries as the family itself.
#[test]
fn antigravity_gateway_family_selection_sends_effort_routes() {
    let options = json!([
        {"value":"gemini-4-hydrogen","name":"Gemini 4 Hydrogen"},
        {"value":"gemini-4-hydrogen-high","name":"Gemini 4 Hydrogen"},
        {"value":"gemini-4-hydrogen-low","name":"Gemini 4 Hydrogen"},
        {"value":"claude-opus-5-5","name":"Claude Opus 5.5"},
        {"value":"claude-opus-5-5-high","name":"Claude Opus 5.5 (High)"}]);
    let model_option = |current: &str| {
        json!({"id":"model","name":"Model","category":"model","type":"select",
            "currentValue":current,"options":options})
    };
    let mut client = Client::new(spec(SessionSource::New));
    client.antigravity = true;
    client.antigravity_gateway = true;
    client.gateway_models = "gemini-4-hydrogen-low, gemini-4-hydrogen-medium, gemini-4-hydrogen-high, claude-opus-5-5-high".into();
    client.ready = true;
    client.session = Some("fixture-session".into());
    client.configure_models(&json!({
        "configOptions":[model_option("gemini-4-hydrogen-high")],
        "models":{"currentModelId":"gemini-4-hydrogen-high",
            "availableModels":[{"modelId":"gemini-4-hydrogen-high","name":"Gemini 4 Hydrogen"}]}
    }));
    // Rows nest under the family slug; the current route marks its row default.
    let ids: Vec<&str> = client
        .models
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert_eq!(ids, ["gemini-4-hydrogen", "claude-opus-5-5-high"]);
    assert_eq!(client.model["id"], "gemini-4-hydrogen");
    // A family-level pick sends the default (medium) effort route.
    client
        .command(json!({"type":"set_model","id":"pick","modelId":"gemini-4-hydrogen"}))
        .unwrap();
    let request = client.wire.last().unwrap().clone();
    assert_eq!(request["method"], "session/set_config_option");
    assert_eq!(
        request["params"],
        json!({"sessionId":"fixture-session","configId":"model","value":"gemini-4-hydrogen-medium"})
    );
    // KSG -32602 retry: the agent folds its medium tier into the family option.
    client
        .incoming(json!({"jsonrpc":"2.0","id":request["id"],
            "error":{"code":-32602,"message":"Unknown model ID: gemini-4-hydrogen-medium"}}))
        .unwrap();
    let retry = client.wire.last().unwrap().clone();
    assert_eq!(retry["method"], "session/set_config_option");
    assert_eq!(retry["params"]["value"], "gemini-4-hydrogen");
    client
        .incoming(json!({"jsonrpc":"2.0","id":retry["id"],
            "result":{"configOptions":[model_option("gemini-4-hydrogen")]}}))
        .unwrap();
    let reply = client.events.last().unwrap();
    assert_eq!(reply["id"], "pick");
    assert_eq!(reply["success"], true);
    assert_eq!(reply["data"]["id"], "gemini-4-hydrogen");
    // The adapter-owned effort pick sends its exact route through the model config.
    client
        .command(
            json!({"type":"set_option","id":"effort","optionId":"reasoningEffort","value":"high"}),
        )
        .unwrap();
    let request = client.wire.last().unwrap().clone();
    assert_eq!(
        request["params"],
        json!({"sessionId":"fixture-session","configId":"model","value":"gemini-4-hydrogen-high"})
    );
    client
        .incoming(json!({"jsonrpc":"2.0","id":request["id"],
            "result":{"configOptions":[model_option("gemini-4-hydrogen-high")]}}))
        .unwrap();
    assert_eq!(client.events.last().unwrap()["id"], "effort");
    assert_eq!(client.events.last().unwrap()["success"], true);
    // The effort pick persists for the next selection on the same row...
    client
        .command(json!({"type":"set_model","id":"again","modelId":"gemini-4-hydrogen"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["params"]["value"],
        "gemini-4-hydrogen-high"
    );
    client
        .incoming(
            json!({"jsonrpc":"2.0","id":client.wire.last().unwrap()["id"],
            "result":{"configOptions":[model_option("gemini-4-hydrogen-high")]}}),
        )
        .unwrap();
    // ...but a different row starts from its own default.
    client
        .command(json!({"type":"set_model","id":"opus","modelId":"claude-opus-5-5-high"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["params"]["value"],
        "claude-opus-5-5-high"
    );
    // An unconfigured model is rejected.
    client
        .command(json!({"type":"set_model","id":"nope","modelId":"gpt-6.2-sol"}))
        .unwrap();
    assert_eq!(client.events.last().unwrap()["success"], false);
    // The gateway auth method drives the eager authenticate call.
    let mut client = Client::new(spec(SessionSource::New));
    client.antigravity = true;
    client.antigravity_gateway = true;
    client.initialize();
    let init = client.wire.pop().unwrap();
    client
        .incoming(json!({"jsonrpc":"2.0","id":init["id"],
            "result":{"protocolVersion":1,"agentCapabilities":{}}}))
        .unwrap();
    let auth = client.wire.pop().unwrap();
    assert_eq!(auth["method"], "authenticate");
    assert_eq!(auth["params"]["methodId"], "gateway");
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
    assert!(context.contains(thread.shina_instructions()));
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
/// A grok `session/load` that never answers still starts once its replay goes
/// quiet, and the real response arriving later is adopted when nothing changed
/// the session in between.
fn grok_load_client() -> Client {
    let mut start = spec(SessionSource::Resume("fixture-session".into()));
    start.id = "grok".into();
    let mut client = Client::new(start);
    assert!(client.replay_idle);
    client.capabilities = json!({"mcpCapabilities":{"http":true},"loadSession":true});
    client.start_session().unwrap();
    client
}

fn grok_load_request(client: &mut Client) -> Value {
    let load = client
        .wire
        .iter()
        .find(|r| r["method"] == "session/load")
        .expect("session/load request")
        .clone();
    assert_eq!(load["params"]["sessionId"], "fixture-session");
    load
}

fn grok_replay_chunk(client: &mut Client, text: &str) {
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"fixture-session",
            "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}}}))
        .unwrap();
}

fn grok_provisional_start(client: &mut Client) {
    client.load_gate.as_mut().unwrap().last_activity =
        Some(Instant::now() - Duration::from_secs(3));
    client.check_load_gate(Instant::now()).unwrap();
    assert!(client.ready);
    assert!(client.late_load.is_some());
    assert!(client.load_gate.is_none());
}

#[test]
fn grok_replay_idle_starts_the_session_and_adopts_a_late_load_response() {
    let mut client = grok_load_client();
    let load = grok_load_request(&mut client);
    let gate = client.load_gate.as_ref().expect("load gate");
    assert_eq!(gate.session_id, "fixture-session");
    assert!(
        client.pending[load["id"].as_str().unwrap()]
            .deadline
            .unwrap()
            > Instant::now() + Duration::from_secs(60),
        "session/load keeps the 90 second deadline"
    );

    // Replay traffic marks activity; the gate stays closed until the gap idles.
    grok_replay_chunk(&mut client, "Earlier reply");
    client.check_load_gate(Instant::now()).unwrap();
    assert!(!client.ready);
    grok_provisional_start(&mut client);

    // The provisional start ran the same publish path as a real response.
    assert!(
        client
            .events
            .iter()
            .any(|e| e["command"] == "get_available_models")
    );
    assert!(client.events.iter().any(|e| e["command"] == "get_messages"));
    assert_eq!(client.history[0]["content"][0]["text"], "Earlier reply");
    client.events.clear();

    // The real response is adopted: its catalog replaces the provisional one.
    client
        .incoming(json!({"jsonrpc":"2.0","id":load["id"],"result":{
            "sessionId":"fixture-session",
            "configOptions":[{"category":"model","id":"model","type":"select",
                "currentValue":"grok-real","options":[{"value":"grok-real","name":"Real"}]}]}}))
        .unwrap();
    assert!(client.late_load.is_none());
    assert_eq!(client.models[0]["id"], "grok-real");
    assert!(
        client
            .events
            .iter()
            .any(|e| e["command"] == "get_available_models")
    );
    assert!(!client.events.iter().any(|e| e["command"] == "get_messages"));
}

#[test]
fn grok_late_load_is_dropped_after_a_session_write() {
    let mut client = grok_load_client();
    let load = grok_load_request(&mut client);
    client.configure_models(&json!({"configOptions":[
        {"category":"model","id":"model","type":"select","currentValue":"a",
         "options":[{"value":"a","name":"A"}]}]}));
    grok_replay_chunk(&mut client, "Earlier reply");
    grok_provisional_start(&mut client);

    // A session write makes the still-pending load response stale.
    client.running = false;
    client
        .command(json!({"type":"set_option","id":"opt","optionId":"model","value":"a"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["method"],
        "session/set_config_option"
    );
    client.events.clear();
    client
        .incoming(json!({"jsonrpc":"2.0","id":load["id"],"result":{
            "sessionId":"fixture-session",
            "configOptions":[{"category":"model","id":"model","type":"select",
                "currentValue":"stale","options":[{"value":"stale","name":"Stale"}]}]}}))
        .unwrap();
    assert!(client.late_load.is_none());
    assert_eq!(client.model["id"], "a", "the stale response must not apply");
    assert!(
        !client
            .events
            .iter()
            .any(|e| e["command"] == "get_available_models")
    );
}

#[test]
fn grok_late_load_is_dropped_after_a_config_notification() {
    let mut client = grok_load_client();
    let load = grok_load_request(&mut client);
    grok_replay_chunk(&mut client, "Earlier reply");
    grok_provisional_start(&mut client);

    // A config notification bumps the change count just like a write.
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"fixture-session",
            "update":{"sessionUpdate":"config_option_update","configOptions":[]}}}))
        .unwrap();
    client
        .incoming(json!({"jsonrpc":"2.0","id":load["id"],"result":{
            "sessionId":"fixture-session",
            "configOptions":[{"category":"model","id":"model","type":"select",
                "currentValue":"stale","options":[{"value":"stale","name":"Stale"}]}]}}))
        .unwrap();
    assert!(client.late_load.is_none());
    assert_ne!(client.model["id"], "stale");
}

#[test]
fn grok_late_load_error_keeps_the_provisional_session() {
    let mut client = grok_load_client();
    let load = grok_load_request(&mut client);
    grok_replay_chunk(&mut client, "Earlier reply");
    grok_provisional_start(&mut client);
    client
        .incoming(json!({"jsonrpc":"2.0","id":load["id"],
            "error":{"code":-32603,"message":"gone"}}))
        .unwrap();
    assert!(client.late_load.is_none());
    assert!(client.ready);
}

#[test]
fn replay_idle_never_fires_for_a_non_grok_load() {
    let mut client = Client::new(spec(SessionSource::Resume("fixture-session".into())));
    assert!(!client.replay_idle);
    client.capabilities = json!({"mcpCapabilities":{"http":true},"loadSession":true});
    client.start_session().unwrap();
    // The gate still tracks the load for its candidate config snapshot.
    client.load_gate.as_mut().unwrap().last_activity =
        Some(Instant::now() - Duration::from_secs(3));
    client.check_load_gate(Instant::now()).unwrap();
    assert!(!client.ready);
    assert!(client.late_load.is_none());
}

#[test]
fn grok_load_reply_after_its_deadline_is_ignored() {
    let mut client = grok_load_client();
    let load = grok_load_request(&mut client);
    grok_replay_chunk(&mut client, "Earlier reply");
    grok_provisional_start(&mut client);
    client
        .check_request_deadlines(Instant::now() + SESSION_LOAD_TIMEOUT)
        .unwrap();
    client.events.clear();
    client
        .incoming(json!({"jsonrpc":"2.0","id":load["id"],"result":{
            "configOptions":[{"category":"model","id":"model","type":"select",
                "currentValue":"too-late","options":[{"value":"too-late","name":"Too late"}]}]}}))
        .unwrap();
    assert!(client.ready);
    assert!(client.events.is_empty());
    assert_ne!(client.model["id"], "too-late");
}

#[test]
fn get_state_reports_the_reasoning_level_the_agent_applied() {
    let mut client = Client::new(spec(SessionSource::New));
    ready(&mut client);
    client.configure_models(&json!({"configOptions":[
        {"category":"thought_level","id":"thought_level","type":"select",
         "currentValue":"medium","options":[{"value":"low","name":"Low"},{"value":"medium","name":"Medium"},{"value":"high","name":"High"}]}]}));
    client.running = false;
    client
        .command(json!({"type":"set_thinking_level","id":"think","level":"high"}))
        .unwrap();
    let request = client.wire.pop().unwrap();
    assert_eq!(request["method"], "session/set_config_option");
    // A bare acknowledgement without echoed configOptions still records the level.
    client
        .incoming(json!({"jsonrpc":"2.0","id":request["id"],"result":{}}))
        .unwrap();
    assert_eq!(client.state()["thinkingLevel"], "high");
    // A delegated child's get_state verification reads the same value.
}

#[test]
fn local_droid_pairs_a_select_and_custom_answer_into_one_question() {
    let mut client = Client::new(spec(SessionSource::New));
    client.flavor = Flavor::detect("local-agent", r"C:\tools\DROID.CMD");
    assert_eq!(client.flavor, Flavor::Droid);
    ready(&mut client);
    client.running = true;
    client.incoming(json!({"jsonrpc":"2.0","id":42,"method":"elicitation/create","params":{
        "mode":"form","requestedSchema":{"required":["question_1","question_2"],
        "properties":{
            "question_1":{"type":"string","title":"Format","description":"Which format?",
                "oneOf":[{"const":"option_1","title":"Plain text"},{"const":"other","title":"Other"}]},
            "question_1_other":{"type":"string"},
            "question_2":{"type":"string","title":"Output","description":"Which output?",
                "oneOf":[{"const":"option_1","title":"HTML"},{"const":"other","title":"Other"}]},
            "question_2_other":{"type":"string"}
        }}}})).unwrap();
    let event = client.events.last().unwrap();
    assert_eq!(event["questions"].as_array().unwrap().len(), 2);
    assert_eq!(
        event["questions"][1]["options"].as_array().unwrap().len(),
        1
    );
    client
        .command(json!({"type":"extension_ui_response","id":event["id"],
        "value":"{\"question_1\":\"option_1\",\"question_2\":\"Markdown\"}"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap(),
        &json!({"jsonrpc":"2.0","id":42,
        "result":{"action":"accept","content":{"question_1":"option_1",
            "question_2":"other","question_2_other":"Markdown"}}})
    );
}

#[test]
fn local_glm_and_vibe_enable_their_provider_specific_events() {
    let mut client = Client::new(spec(SessionSource::New));
    client.flavor = Flavor::detect("local-agent", "/tools/glm");
    assert_eq!(client.flavor, Flavor::Glm);
    ready(&mut client);
    client
        .command(json!({"type":"prompt","id":"1","message":"hello"}))
        .unwrap();
    let id = client.wire.last().unwrap()["id"].clone();
    client
        .incoming(
            json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"end_turn",
        "usage":{"totalTokens":3000,"inputTokens":2500,"outputTokens":500}}}),
        )
        .unwrap();
    assert!(
        client
            .events
            .iter()
            .any(|event| event["field"] == "lastTurnUsage"
                && event["value"]
                    == json!({"totalTokens":3000,"inputTokens":2500,"outputTokens":500}))
    );

    client.flavor = Flavor::detect("local-agent", "/tools/mistral-vibe-acp");
    assert_eq!(client.flavor, Flavor::Vibe);
    client
        .command(json!({"type":"prompt","id":"2","message":"hello"}))
        .unwrap();
    let id = client.wire.last().unwrap()["id"].clone();
    client
        .incoming(
            json!({"jsonrpc":"2.0","method":"_session/retrying","params":{
        "sessionId":client.session,"category":"rate_limited","attempt":1,"detail":"retrying"}}),
        )
        .unwrap();
    assert!(
        client
            .events
            .iter()
            .any(|event| event["field"] == "providerRetry"
                && event["value"]["class"] == "usage_limit")
    );
    assert!(
        client
            .incoming(json!({"jsonrpc":"2.0","id":id,
        "error":{"code":-31001,"message":"limit reached"}}))
            .unwrap_err()
            .contains("rate limit")
    );
    assert_eq!(Flavor::detect("factory-droid", "runner"), Flavor::Droid);
    assert_eq!(
        Flavor::detect("local-agent", "our-glm-wrapper"),
        Flavor::Generic
    );
}

#[test]
fn an_existing_cursor_session_accepts_a_refreshed_catalog_model_and_its_native_tier() {
    let mut start = spec(SessionSource::New);
    start.id = "cursor".into();
    let mut client = Client::new(start);
    ready(&mut client);
    let model = |id: &str| {
        json!({"id":"model","category":"model","type":"select",
        "currentValue":id,"options":[{"value":id,"name":id}]})
    };
    client.configure_models(&json!({"configOptions":[model("old-model")]}));
    client
        .command(json!({"type":"set_model","id":"pick","modelId":"new-model",
        "fromCursorCatalog":true}))
        .unwrap();
    let request = client.wire.last().unwrap();
    assert_eq!(request["params"]["value"], "new-model");
    client.incoming(json!({"jsonrpc":"2.0","id":request["id"],"result":{"configOptions":[
        model("new-model"),{"id":"context","type":"select","category":"model_config",
            "currentValue":"200k","options":[{"value":"200k","name":"200K"},{"value":"2m","name":"2M Max"}]}]}})).unwrap();
    assert_eq!(client.model["id"], "new-model");
    client
        .command(json!({"type":"set_option","id":"tier","optionId":"contextWindow","value":"2m"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["params"],
        json!({"sessionId":"fixture-session","configId":"context","value":"2m"})
    );
}

#[test]
fn delayed_replay_updates_after_readiness_do_not_overwrite_live_controls_or_emit_deltas() {
    let mut client = grok_load_client();
    let _load = grok_load_request(&mut client);
    grok_replay_chunk(&mut client, "Startup history");
    grok_provisional_start(&mut client);
    // Live session moves to a newer model and clears events.
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"fixture-session",
            "update":{"sessionUpdate":"config_option_update","configOptions":[
                {"category":"model","id":"model","type":"select","currentValue":"live-model",
                 "options":[{"value":"live-model","name":"Live"}]}]}}}))
        .unwrap();
    assert_eq!(client.model["id"], "live-model");
    let changes_before = client.session_changes;
    let history_before = client.history.clone();
    client.events.clear();

    // Delayed replay config and text must not overwrite live state or stream as live output.
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"fixture-session",
            "_meta":{"isReplay":true},
            "update":{"sessionUpdate":"config_option_update","configOptions":[
                {"category":"model","id":"model","type":"select","currentValue":"stale-replay",
                 "options":[{"value":"stale-replay","name":"Stale"}]}]}}}))
        .unwrap();
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"fixture-session",
            "_meta":{"isReplay":true},
            "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Replayed delta"}}}}))
        .unwrap();

    assert_eq!(client.model["id"], "live-model");
    assert_eq!(client.session_changes, changes_before);
    assert_eq!(client.history, history_before);
    assert!(
        client.events.is_empty(),
        "delayed replay must emit no live events: {:?}",
        client.events
    );
}

#[test]
fn republished_seed_models_are_marked_not_advertised_live() {
    let mut client = Client::new(spec(SessionSource::New));
    client.models = vec![json!({
        "id": "seeded-from-elsewhere",
        "name": "Seeded",
        "provider": "acp",
        "capabilities": {
            "optionDescriptors": [{
                "kind": "select",
                "id": "context",
                "label": "Context",
                "options": [{"id": "1m", "label": "1M", "isDefault": true}],
                "currentValue": "1m"
            }]
        }
    })];
    ready(&mut client);
    client.events.clear();
    client.configure_models(&json!({}));
    client.publish_models();
    let published = client
        .events
        .iter()
        .find(|event| event["command"] == "get_available_models")
        .expect("models published");
    assert_eq!(published["data"]["advertised"], false);
    assert_eq!(published["data"]["liveOptionDescriptors"], json!([]));
    assert_eq!(
        published["data"]["models"][0]["id"],
        "seeded-from-elsewhere"
    );

    client.events.clear();
    client.configure_models(&json!({"configOptions":[
        {"category":"model","id":"model","type":"select","currentValue":"live-a",
         "options":[{"value":"live-a","name":"Live A"}]}]}));
    client.publish_models();
    let live = client
        .events
        .iter()
        .find(|event| event["command"] == "get_available_models")
        .expect("live models published");
    assert_eq!(live["data"]["advertised"], true);
}

#[test]
fn cursor_set_option_rejects_unadvertised_option_and_choice() {
    let mut start = spec(SessionSource::New);
    start.id = "cursor".into();
    let mut client = Client::new(start);
    ready(&mut client);
    client.configure_models(&json!({"configOptions":[
        {"id":"context","type":"select","category":"model_config","currentValue":"200k",
         "options":[{"value":"200k","name":"200K"}]}]}));
    let wire_before = client.wire.len();
    client.events.clear();

    client
        .command(
            json!({"type":"set_option","id":"bad-val","optionId":"contextWindow","value":"2m"}),
        )
        .unwrap();
    assert_eq!(client.wire.len(), wire_before);
    assert_eq!(client.events.last().unwrap()["success"], false);

    client
        .command(json!({"type":"set_option","id":"missing","optionId":"maxMode","value":true}))
        .unwrap();
    assert_eq!(client.wire.len(), wire_before);
    assert_eq!(client.events.last().unwrap()["success"], false);
}
