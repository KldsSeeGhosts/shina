use super::*;
use crate::model::{AccessMode, Status, Thread};

#[test]
fn prompt_free_probe_paginates_models_and_discovers_workspace_skills() {
    for flags in [
        vec![],
        vec!["--paginated-models"],
        vec!["--skills-unsupported"],
        vec!["--legacy-version"],
        vec!["--model-failure"],
    ] {
        let dir = crate::store::directory().unwrap();
        let log = dir.join(format!("probe-{}.jsonl", flags.join("-")));
        let script = dir.join("probe.py");
        std::fs::write(&script, include_str!("../assets/tests/fake_codex.py")).unwrap();
        let mut command = Command::new(if cfg!(windows) { "python" } else { "python3" });
        command
            .arg("-u")
            .arg(script)
            .arg("--log")
            .arg(&log)
            .args(&flags);
        let models = !flags.is_empty();
        let workspace = if models {
            dir.clone()
        } else {
            dir.join("changed-workspace")
        };
        let start = Start::discovery("codex", workspace.clone());
        let owner = start.owner.clone();
        let pi = Pi::worker(
            Arc::new(|| {}),
            owner,
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
        );
        let mut output = vec![];
        loop {
            match pi
                .events
                .recv_timeout(Duration::from_secs(10))
                .expect("probe deadline")
            {
                Event::Record(record) => output.push(record),
                Event::Error(error) => {
                    assert!(flags.contains(&"--model-failure"), "{error}");
                    assert!(error.contains("Fixture model failure"));
                    break;
                }
                Event::Exited => break,
                Event::Diagnostic(_) => {}
            }
        }
        let wire = records(&log);
        assert_eq!(
            wire.iter().filter(|r| r["method"] == "initialize").count(),
            1
        );
        assert!(!wire.iter().any(|r| matches!(
            r["method"].as_str(),
            Some("thread/start" | "thread/resume" | "turn/start")
        )));
        if !models {
            assert!(!wire.iter().any(|record| record["method"] == "model/list"));
            assert!(wire.iter().any(|record| record["method"] == "skills/list"
                && record["params"]["cwds"] == json!([workspace])));
        }
        if flags.contains(&"--paginated-models") {
            let models = &output
                .iter()
                .find(|r| r["command"] == "get_available_models")
                .unwrap()["data"]["models"];
            assert_eq!(models.as_array().unwrap().len(), 2);
            assert_eq!(models[0]["thinkingLevels"], json!(["low", "high"]));
            assert_eq!(models[0]["serviceTiers"][1]["id"], "fast");
            assert!(wire.iter().any(|r| r["params"]["cursor"] == "second-page"));
        }
        if !flags.contains(&"--model-failure") {
            let commands = &output
                .iter()
                .find(|r| r["command"] == "get_commands")
                .unwrap()["data"]["commands"];
            assert_eq!(
                commands
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["name"] == "goal"),
                !flags.contains(&"--legacy-version")
            );
        }
    }
}

#[test]
fn native_goals_feedback_and_service_tiers_never_become_prompts() {
    let (mut thread, log) = fixture(SessionSource::New, "read-only", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    for (name, arguments) in [
        ("goal", "Implement fixture"),
        ("goal", "pause"),
        ("goal", "resume"),
        ("goal", ""),
        ("goal", "clear"),
        ("feedback", "Fixture feedback"),
    ]
    .into_iter()
    {
        thread.command("native_command", json!({"name":name,"arguments":arguments}));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline);
            if let Ok(Event::Record(record)) = thread
                .pi
                .as_ref()
                .unwrap()
                .events
                .recv_timeout(Duration::from_millis(50))
            {
                let done = record["command"]
                    == if name == "goal" {
                        "native_goal"
                    } else {
                        "native_feedback"
                    };
                thread.apply(record);
                if done {
                    break;
                }
            }
        }
    }
    assert!(!records(&log).iter().any(|r| r["method"] == "turn/start"));
    thread.set_service_tier("fast".into());
    thread.draft.replace_text("Fixture turn");
    thread.send();
    pump(&mut thread, |t| t.status == Status::Completed);
    let wire = records(&log);
    assert_eq!(
        wire.iter()
            .filter(|record| record["method"] == "thread/settings/update")
            .count(),
        2
    );
    let turn = wire.iter().find(|r| r["method"] == "turn/start").unwrap();
    assert_eq!(turn["params"]["serviceTierForTurn"], "fast");
    assert!(
        turn["params"].get("serviceTier").is_none(),
        "override is turn-only"
    );

    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.ready = true;
    client.native_extensions = true;
    client.session = Some("fixture".into());
    for message in ["/goal objective", "/feedback issue"] {
        client
            .command(json!({"type":"prompt","id":"direct","message":message}))
            .unwrap();
    }
    assert!(!client.wire.iter().any(|r| r["method"] == "turn/start"));
    let setting = client
        .wire
        .iter()
        .find(|record| record["method"] == "thread/settings/update")
        .unwrap()["id"]
        .clone();
    client.wire.clear();
    client
        .command(json!({"type":"abort","id":"cancel-activation"}))
        .unwrap();
    client.incoming(json!({"id":setting,"result":{}})).unwrap();
    assert!(
        !client
            .wire
            .iter()
            .any(|record| record["method"] == "thread/goal/set")
    );
}

#[test]
fn stopping_a_native_goal_pauses_it_and_unsolicited_turns_report_running() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.ready = true;
    client.native_extensions = true;
    client.session = Some("fixture".into());
    client
        .notification(
            "thread/goal/updated",
            &json!({"goal":{"objective":"Fixture","status":"active"}}),
        )
        .unwrap();
    client
        .notification("turn/started", &json!({"turn":{"id":"goal-turn"}}))
        .unwrap();
    assert!(
        client
            .events
            .iter()
            .any(|record| record["type"] == "agent_start")
    );
    client.command(json!({"type":"abort","id":"stop"})).unwrap();
    assert!(
        client
            .wire
            .iter()
            .any(|record| record["method"] == "thread/goal/set"
                && record["params"]["status"] == "paused"
                && record["params"]["origin"] == "user")
    );
    assert!(
        client
            .wire
            .iter()
            .any(|record| record["method"] == "turn/interrupt")
    );
    assert!(
        !client
            .wire
            .iter()
            .any(|record| record["method"] == "turn/start")
    );
    client.wire.clear();
    client
        .command(
            json!({"type":"native_command","name":"goal","arguments":"pause","id":"pause-running"}),
        )
        .unwrap();
    assert!(
        client
            .wire
            .iter()
            .any(|record| record["method"] == "thread/goal/set"
                && record["params"]["status"] == "paused")
    );
}

#[test]
fn native_working_directory_changes_invalidate_and_refresh_skills() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.ready = true;
    client.skill_commands = vec![json!({"name":"$old-project"})];
    let cwd = client.start.workspace.join("new-project");
    client
        .notification(
            "thread/settings/updated",
            &json!({"threadSettings":{"cwd":cwd}}),
        )
        .unwrap();
    assert!(client.skill_commands.is_empty());
    let request = client.wire.last().unwrap().clone();
    assert_eq!(request["method"], "skills/list");
    assert_eq!(request["params"], json!({"cwds":[cwd],"forceReload":true}));
    client.incoming(json!({"id":request["id"],"result":{"data":[{"cwd":cwd,"skills":[{"name":"new-project","enabled":true}]}]}})).unwrap();
    assert_eq!(client.skill_commands[0]["name"], "$new-project");
}

#[test]
fn codex_fork_and_revert_use_native_turn_boundaries_and_refresh_history() {
    let (mut thread, log) = fixture(
        SessionSource::Fork("codex-fixture-thread".into()),
        "read-only",
        &[],
    );
    pump(&mut thread, |t| t.native.commands_loaded);
    assert_eq!(thread.session.as_deref(), Some("codex-fixture-fork"));
    thread.request_fork_points(crate::model::ConversationMode::Rewind);
    pump(&mut thread, |t| !t.native.fork_points.is_empty());
    assert_eq!(thread.native.fork_points[0]["entryId"], "old");
    thread.rewind("old".into());
    pump(&mut thread, |t| {
        !t.configuring && !t.initializing && t.messages.is_empty()
    });
    assert_eq!(thread.draft.text, "Prior prompt");
    assert_eq!(thread.session.as_deref(), Some("codex-fixture-fork"));
    let wire = records(&log);
    let fork = wire.iter().find(|r| r["method"] == "thread/fork").unwrap();
    assert_eq!(fork["params"]["threadId"], "codex-fixture-thread");
    let revert = wire
        .iter()
        .find(|r| r["method"] == "thread/revert")
        .unwrap();
    assert_eq!(
        revert["params"],
        json!({"threadId":"codex-fixture-fork","beforeTurnId":"old"})
    );
    assert!(!wire.iter().any(|r| r["method"] == "turn/start"));
}

#[test]
fn schema_item_fixture_preserves_tools_results_and_generic_items() {
    use crate::work::ToolKind::*;
    let (mut thread, _) = fixture(
        SessionSource::New,
        "read-only",
        &["--all-items", "--automatic-compaction"],
    );
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.draft.replace_text("Every item");
    thread.send();
    pump(&mut thread, |t| {
        t.status == Status::Completed && t.harness_state["usageLimits"]["windows"].is_array()
    });
    for (id, kind) in [
        ("hook", Other),
        ("function", Other),
        ("files", FileChange),
        ("mcp", Other),
        ("dynamic", Other),
        ("collab", Subagent),
        ("activity", Subagent),
        ("web", WebSearch),
        ("image", Read),
        ("sleep", Other),
        ("generated", Other),
        ("review-in", Other),
        ("review-out", Other),
        ("codex-plan-plan-item", Plan),
        ("cmd", Command),
    ] {
        let tool = thread
            .tools
            .iter()
            .find(|tool| tool.id == id)
            .unwrap_or_else(|| panic!("Missing {id}"));
        assert_eq!(tool.kind, Some(kind), "{id}");
    }
    let tool = |id: &str| thread.tools.iter().find(|tool| tool.id == id).unwrap();
    assert_eq!(tool("mcp").name, "fixture-server/lookup");
    assert_eq!(tool("mcp").output, "MCP result");
    assert_eq!(tool("mcp").structured["structuredContent"]["answer"], 42);
    assert_eq!(tool("dynamic").name, "fixture-namespace/lookup");
    assert_eq!(tool("dynamic").output, "Dynamic result");
    assert_eq!(tool("function").output, "Function result");
    assert_eq!(tool("files").changes.len(), 2);
    assert_eq!(tool("files").changes[0].old_path.as_deref(), Some("old.rs"));
    assert_eq!(tool("files").changes[0].path, "new.rs");
    assert_eq!(tool("collab").subagents[0]["output"], "Child result");
    assert_eq!(
        tool("web").structured["webResults"][0]["url"],
        "https://example.com"
    );
    assert_eq!(tool("cmd").exit_code, Some(0));
    assert_eq!(tool("child:nested").parent_call.as_deref(), Some("collab"));
    assert_eq!(tool("child:nested").output, "child directory");
    assert_eq!(thread.thoughts[0].text, "Checking");
    assert!(!thread.native.compactions.is_empty());
    assert_eq!(
        thread.harness_state["usageLimits"]["windows"][0]["usedPercent"],
        92.0
    );
    thread.command("get_messages", json!({}));
    let mut restored = Thread::new(32);
    loop {
        match thread
            .pi
            .as_ref()
            .unwrap()
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            Event::Record(mut record) if record["command"] == "get_messages" => {
                record["id"] = json!("history");
                restored.apply(record);
                break;
            }
            Event::Record(record) => thread.apply(record),
            event => panic!(
                "Unexpected history event: {}",
                match event {
                    Event::Error(error) => error,
                    _ => "Disconnected".into(),
                }
            ),
        }
    }
    for live in &thread.tools {
        let saved = restored
            .tools
            .iter()
            .find(|tool| tool.id == live.id)
            .unwrap_or_else(|| panic!("Replay lost {}", live.id));
        assert_eq!(saved.kind, live.kind, "{}", live.id);
        assert_eq!(saved.changes, live.changes, "{}", live.id);
        assert_eq!(saved.parent_call, live.parent_call, "{}", live.id);
        assert_eq!(saved.output, live.output, "{}", live.id);
    }
}

#[test]
fn sparse_usage_updates_keep_other_windows_and_auth_errors_name_the_command() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client
        .notification(
            "account/rateLimits/updated",
            &json!({"rateLimits":{
        "primary":{"usedPercent":90,"windowDurationMins":300,"resetsAt":123},
        "secondary":{"usedPercent":40,"windowDurationMins":10080,"resetsAt":456}}}),
        )
        .unwrap();
    client.events.clear();
    client
        .notification(
            "account/rateLimits/updated",
            &json!({"rateLimits":{
        "primary":{"usedPercent":91,"windowDurationMins":300,"resetsAt":123},"secondary":null}}),
        )
        .unwrap();
    assert_eq!(client.usage_limits["windows"][0]["usedPercent"], 91.0);
    assert_eq!(client.usage_limits["windows"][1]["usedPercent"], 40.0);
    let before = client.usage_limits.clone();
    client
        .notification(
            "account/rateLimits/updated",
            &json!({"rateLimits":{
        "limitId":"other-model","primary":{"usedPercent":100}}}),
        )
        .unwrap();
    assert_eq!(client.usage_limits, before);
    client
        .notification(
            "account/rateLimits/updated",
            &json!({"rateLimits":{
        "individualLimit":{"remainingPercent":20,"resetsAt":789}}}),
        )
        .unwrap();
    assert_eq!(client.usage_limits["windows"][2]["usedPercent"], 80.0);
    assert!(auth_guidance("Not authenticated").contains("`codex login`"));
    assert_eq!(auth_guidance("Other error"), "Other error");
}

#[test]
fn typed_items_survive_live_and_history_reduction() {
    use crate::work::ToolKind::*;
    for (native, expected) in [
        ("commandExecution", Command),
        ("fileChange", FileChange),
        ("fileSearch", FileSearch),
        ("webSearch", WebSearch),
        ("readFile", Read),
        ("listDirectory", List),
        ("collabAgentToolCall", Subagent),
        ("todoList", Plan),
        ("unknown", Other),
    ] {
        let mut client = Client::new(spec(SessionSource::New, "read-only"));
        let item = json!({"id":"typed","type":native,"status":"completed","command":"pwd",
            "changes":[{"path":"old.rs","kind":{"type":"update","move_path":"new.rs"},"diff":"@@ -1 +1 @@\n-old\n+new\n"}],
            "results":[{"fileName":"new.rs","line":2,"preview":"new","url":"https://example.com","title":"Example","snippet":"Text"}]});
        client.item(&item, false, true);
        client.item(&item, true, true);
        let mut live = Thread::new(1);
        for event in client.events {
            live.apply(event);
        }
        let mut restored = Thread::new(2);
        restored.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":client.history}}));
        for thread in [&live, &restored] {
            let tool = &thread.tools[0];
            assert_eq!(tool.kind, Some(expected), "{native}");
            if expected == FileChange {
                assert_eq!(tool.changes[0].path, "new.rs");
                assert_eq!(tool.changes[0].old_path.as_deref(), Some("old.rs"));
                assert_eq!(
                    (tool.changes[0].additions, tool.changes[0].deletions),
                    (1, 1)
                );
            }
            if expected == FileSearch {
                assert_eq!(tool.structured["fileSearchResults"][0]["line"], 2);
            }
            if expected == WebSearch {
                assert_eq!(tool.structured["webResults"][0]["title"], "Example");
            }
        }
    }
}

#[test]
fn plan_updates_replace_the_first_card_in_live_and_replayed_history() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.turn = Some("turn-plan".into());
    client
        .notification(
            "turn/plan/updated",
            &json!({"plan":[{"step":"First","status":"pending"}]}),
        )
        .unwrap();
    client.item(
        &json!({"id":"cmd","type":"commandExecution","command":"pwd","status":"completed"}),
        true,
        true,
    );
    client.notification("turn/plan/updated",&json!({"plan":[{"step":"First","status":"completed"},{"step":"Next","status":"in_progress"}]})).unwrap();
    client.item(
        &json!({"id":"proposal","type":"plan","text":"## Proposed plan\n\nUse **Rust**."}),
        true,
        true,
    );
    let mut live = Thread::new(1);
    for event in client.events {
        live.apply(event);
    }
    let mut restored = Thread::new(2);
    restored.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":client.history}}));
    for t in [&live, &restored] {
        assert_eq!(t.tools.len(), 3);
        assert_eq!(t.tools[0].resolved_kind(), crate::work::ToolKind::Plan);
        assert_eq!(t.tools[1].resolved_kind(), crate::work::ToolKind::Command);
        let plan = crate::work::plan_data(&t.tools[0].input);
        assert_eq!(plan["steps"][0]["status"], "completed");
        assert_eq!(plan["steps"][1]["status"], "running");
        assert!(
            crate::work::plan_data(&t.tools[2].input)["markdown"]
                .as_str()
                .unwrap()
                .contains("**Rust**")
        );
        let saved: crate::model::Tool =
            serde_json::from_value(serde_json::to_value(&t.tools[0]).unwrap()).unwrap();
        assert_eq!(saved.input, t.tools[0].input);
    }
}

#[test]
fn codex_compaction_replay_keeps_its_position_and_interrupted_turns_keep_the_marker() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.replay(&json!({"id":"one","status":"completed","items":[
        {"type":"userMessage","id":"u1","content":[{"type":"text","text":"Before"}]},
        {"type":"agentMessage","id":"a1","text":"First"},
        {"type":"contextCompaction","id":"compact"}]}));
    client.replay(&json!({"id":"two","status":"interrupted","items":[
        {"type":"userMessage","id":"u2","content":[{"type":"text","text":"After"}]},
        {"type":"commandExecution","id":"cmd","command":"pwd","status":"completed"}]}));
    let mut thread = Thread::new(1);
    thread.apply(
        json!({"type":"response","id":"history","command":"get_messages","success":true,
        "data":{"messages":client.history,"compactions":client.compactions}}),
    );
    assert_eq!(
        crate::work::compaction_anchor(&thread, &thread.native.compactions[0]).message,
        2
    );
    assert!(thread.work.last().unwrap().stopped);
}

#[test]
fn codex_provider_retries_replace_one_event_and_recover_or_fail() {
    for failed in [false, true] {
        let mut client = Client::new(spec(SessionSource::New, "read-only"));
        client
            .notification("turn/started", &json!({"turn":{"id":"retry"}}))
            .unwrap();
        for attempt in [1, 2] {
            client
                .notification(
                    "error",
                    &json!({"willRetry":true,"attempt":attempt,"maxAttempts":5,
                "error":{"message":"Temporary error"}}),
                )
                .unwrap();
        }
        if failed {
            assert!(
                client
                    .notification(
                        "error",
                        &json!({"willRetry":false,"error":{"message":"Final error"}})
                    )
                    .is_err()
            );
        } else {
            client
                .notification(
                    "turn/completed",
                    &json!({"turn":{"id":"retry","status":"completed","items":[]}}),
                )
                .unwrap();
        }
        let mut thread = Thread::new(1);
        for event in client.events {
            thread.apply(event);
        }
        let events = crate::work::retry_events(&thread);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["attempt"], 2);
        assert_eq!(events[0]["maxAttempts"], 5);
        assert_eq!(events[0]["running"], false);
        assert_eq!(events[0]["success"], !failed);
        if failed {
            assert_eq!(thread.error.as_deref(), Some("Final error"));
        }
    }
}

fn spec(source: SessionSource, access: &str) -> Start {
    Start {
        id: "codex".into(),
        workspace: crate::store::directory().unwrap(),
        source,
        instructions: Some("Captain profile\nUser-curated essential memory: use Rust".into()),
        link: Some(crate::backend::HostLink {
            url: "http://127.0.0.1:12345/mcp".into(),
            token: "fixture-thread-token".into(),
            extension: PathBuf::new(),
            access_mode: access.into(),
        }),
        owner: Arc::default(),
        context_sent: false,
    }
}

fn fixture(source: SessionSource, access: &str, flags: &[&str]) -> (Thread, PathBuf) {
    let dir = crate::store::directory().unwrap();
    let script = dir.join("fake_codex.py");
    std::fs::write(&script, include_str!("../assets/tests/fake_codex.py")).unwrap();
    let log = dir.join(format!(
        "codex-{}.jsonl",
        if matches!(source, SessionSource::New) {
            "new"
        } else {
            "resume"
        }
    ));
    let mut command = Command::new(if cfg!(windows) { "python" } else { "python3" });
    command
        .arg("-u")
        .arg(script)
        .arg("--log")
        .arg(&log)
        .args(flags);
    let mut thread = Thread::new(31);
    thread.harness = "codex".into();
    thread.native.access = if access == "read-only" {
        AccessMode::ReadOnly
    } else {
        AccessMode::Supervised
    };
    thread.pi = Some(start_process(command, spec(source, access), Arc::new(|| {})).unwrap());
    (thread, log)
}

fn pump(thread: &mut Thread, done: impl Fn(&Thread) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(thread) {
        assert!(
            Instant::now() < deadline,
            "Codex timeout, {:?}: {:?}",
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
            Ok(Event::Error(error)) => panic!("Codex fixture failed: {error}"),
            Ok(Event::Exited) => panic!("Codex fixture exited"),
            _ => {}
        }
    }
}

fn records(log: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn handshake_stream_reasoning_command_plan_usage_and_approval_round_trip() {
    let (mut thread, log) = fixture(SessionSource::New, "approval-required", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    assert_eq!(thread.session.as_deref(), Some("codex-fixture-thread"));
    assert_eq!(thread.models[0].thinking_levels, ["low", "high"]);
    thread.draft.replace_text("approval");
    thread.send();
    pump(&mut thread, |t| t.request.is_some());
    assert_eq!(thread.status, Status::Waiting);
    thread.respond(json!({"confirmed":true}));
    pump(&mut thread, |t| {
        t.status == Status::Completed && t.context.usage.tokens == Some(1234)
    });
    assert!(thread.messages.iter().any(|m| m.text == "Hello Codex"));
    assert_eq!(thread.thoughts[0].text, "Checking");
    assert_eq!(thread.tools[0].name, "command");
    assert!(thread.tools[0].done);
    assert_eq!(thread.tools[0].output, "fixture");
    assert!(thread.harness_state["plan"]["plan"].is_array());
    assert_eq!(thread.context.usage.context_window, Some(200000));
    let log = records(&log);
    assert_eq!(log[0]["method"], "initialize");
    assert_eq!(log[1]["method"], "initialized");
    let turn = log.iter().find(|r| r["method"] == "turn/start").unwrap();
    assert_eq!(turn["params"]["sandboxPolicy"]["type"], "readOnly");
    assert_eq!(turn["params"]["approvalPolicy"], "untrusted");
    assert!(
        log.iter()
            .any(|r| r["id"] == 999 && r["result"]["decision"] == "accept")
    );
}

#[test]
fn a_skill_discovery_authentication_denial_stops_startup_without_a_fallback() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.request(
        "skills/list",
        json!({"cwds":[client.start.workspace]}),
        json!({"type":"skills","id":"commands"}),
    );
    let id = client.wire.last().unwrap()["id"].clone();
    client.wire.clear();
    let error = client
        .incoming(json!({"id":id,"error":{"code":401,"message":"Authentication required"}}))
        .unwrap_err();
    assert!(error.contains("Authentication required"));
    assert!(client.wire.is_empty());
}

#[test]
fn skills_list_is_project_scoped_and_skill_mentions_remain_native_text() {
    let (mut thread, log) = fixture(SessionSource::New, "read-only", &[]);
    pump(&mut thread, |t| !t.native.skills.is_empty());
    assert_eq!(thread.native.skills.len(), 1);
    assert_eq!(thread.native.skills[0].name, "fixture-skill");
    thread.draft.insert_pill(
        crate::composer_tokens::Token::Skill {
            name: "fixture-skill".into(),
            harness: "codex".into(),
        }
        .id(),
        "fixture-skill",
    );
    thread.draft.insert(" respond");
    thread.send();
    pump(&mut thread, |t| t.status == Status::Completed);
    let log = records(&log);
    let request = log.iter().find(|r| r["method"] == "skills/list").unwrap();
    assert_eq!(
        request["params"]["cwds"][0],
        crate::store::directory()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    let turn = log.iter().find(|r| r["method"] == "turn/start").unwrap();
    assert_eq!(turn["params"]["input"][0]["text"], "$fixture-skill respond");
}

#[test]
fn read_only_refuses_write_without_a_dialog_or_a_file() {
    let (mut thread, log) = fixture(SessionSource::New, "read-only", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.draft.replace_text("write");
    thread.send();
    pump(&mut thread, |t| t.status == Status::Completed);
    assert!(thread.request.is_none());
    assert!(
        !crate::store::directory()
            .unwrap()
            .join("fixture.txt")
            .exists()
    );
    let log = records(&log);
    let turn = log.iter().find(|r| r["method"] == "turn/start").unwrap();
    assert_eq!(turn["params"]["approvalPolicy"], "never");
    assert_eq!(turn["params"]["sandboxPolicy"]["type"], "readOnly");
    assert!(
        log.iter()
            .any(|r| r["id"] == 999 && r["result"]["decision"] == "decline")
    );
}

#[test]
fn interrupt_and_resume_rebuild_history_and_refresh_awareness() {
    let (mut thread, log) = fixture(SessionSource::New, "approval-required", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.draft.replace_text("interrupt");
    thread.send();
    pump(&mut thread, |t| {
        t.messages.iter().any(|m| m.text == "interrupt")
    });
    thread.stop();
    pump(&mut thread, |t| !t.status.busy());
    assert!(
        records(&log)
            .iter()
            .any(|r| r["method"] == "turn/interrupt")
    );
    drop(thread);
    let (mut thread, log) = fixture(
        SessionSource::Resume("codex-fixture-thread".into()),
        "read-only",
        &[],
    );
    pump(&mut thread, |t| t.native.commands_loaded);
    assert!(thread.messages.iter().any(|m| m.text == "Prior reply"));
    assert_eq!(thread.native.compactions.len(), 1);
    assert_ne!(thread.native.compactions[0]["running"], true);
    assert!(
        !thread
            .tools
            .iter()
            .any(|tool| tool.name == "contextCompaction")
    );
    let log = records(&log);
    let resume = log.iter().find(|r| r["method"] == "thread/resume").unwrap();
    assert!(
        resume["params"]["developerInstructions"]
            .as_str()
            .unwrap()
            .contains("Captain profile")
    );
    assert!(
        resume["params"]["developerInstructions"]
            .as_str()
            .unwrap()
            .contains("essential memory")
    );
    assert_eq!(
        resume["params"]["config"]["mcp_servers.shina"]["http_headers"]["Authorization"],
        "Bearer fixture-thread-token"
    );
}

#[test]
fn awareness_is_developer_context_on_new_and_resumed_sessions() {
    for source in [SessionSource::New, SessionSource::Resume("saved".into())] {
        let client = Client::new(spec(source, "read-only"));
        let params = client.session_params();
        let context = params["developerInstructions"].as_str().unwrap();
        assert!(context.contains(crate::mcp::INSTRUCTIONS));
        assert!(context.contains("Captain profile"));
        assert!(context.contains("User-curated essential memory"));
        assert!(params.get("baseInstructions").is_none());
        assert_eq!(params["config"]["mcp_servers.shina"]["required"], true);
    }
}

#[test]
fn attachments_models_efforts_and_memory_diff_use_exact_protocol_items() {
    let directory = crate::store::directory().unwrap();
    let file = directory.join("notes.txt");
    std::fs::write(&file, "notes").unwrap();
    let folder = directory.join("folder");
    std::fs::create_dir(&folder).unwrap();
    let files = vec![
        crate::attachments::attach_file(&file).unwrap(),
        crate::attachments::attach_file(&folder).unwrap(),
    ];
    let prompt = crate::attachments::Prompt::new(
        "<shina_memory_diff>new memory</shina_memory_diff>\ninspect".into(),
        files,
    );
    let mut record = crate::attachments::prompt_record(&prompt, false).unwrap();
    record["images"] = json!([{"type":"image","mimeType":"image/png","data":"fixture"}]);
    let input = codex_input(&record).unwrap();
    assert!(
        input[0]["text"]
            .as_str()
            .unwrap()
            .contains("<shina_memory_diff>")
    );
    assert!(input[0]["text"].as_str().unwrap().contains("notes.txt"));
    assert!(input[0]["text"].as_str().unwrap().contains("folder"));
    assert_eq!(
        input[1],
        json!({"type":"image","url":"data:image/png;base64,fixture"})
    );
    let (mut thread, log) = fixture(SessionSource::New, "approval-required", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.set_thinking("low".into());
    pump(&mut thread, |t| !t.configuring);
    thread.draft.replace_text("settings");
    thread.send();
    pump(&mut thread, |t| t.status == Status::Completed);
    let log = records(&log);
    let turn = log.iter().find(|r| r["method"] == "turn/start").unwrap();
    assert_eq!(turn["params"]["effort"], "low");
    assert_eq!(turn["params"]["model"], "fixture-model");
}

#[test]
fn authentication_denial_stops_without_retry_or_configuration_changes() {
    let (thread, log) = fixture(SessionSource::New, "read-only", &["--auth-denial"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline);
        if let Ok(Event::Error(error)) = thread
            .pi
            .as_ref()
            .unwrap()
            .events
            .recv_timeout(Duration::from_millis(100))
        {
            assert!(error.contains("Authentication required"));
            break;
        }
    }
    assert_eq!(
        records(&log)
            .iter()
            .filter(|r| r["method"] == "thread/start")
            .count(),
        1
    );
}

#[test]
fn runtime_modes_match_ksg_and_read_only_cannot_escalate() {
    for (mode, approval, sandbox) in [
        ("approval-required", "untrusted", "readOnly"),
        ("auto-accept-edits", "on-request", "workspaceWrite"),
        ("full-access", "never", "dangerFullAccess"),
        ("read-only", "never", "readOnly"),
    ] {
        let mut client = Client::new(spec(SessionSource::New, mode));
        client.session = Some("fixture".into());
        client.ready = true;
        client
            .command(json!({"type":"prompt","message":"test"}))
            .unwrap();
        assert_eq!(client.wire[0]["params"]["approvalPolicy"], approval);
        assert_eq!(client.wire[0]["params"]["sandboxPolicy"]["type"], sandbox);
    }
}

#[test]
fn native_questions_use_the_existing_questionnaire_and_do_not_auto_answer() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.session = Some("fixture".into());
    client.ready = true;
    client.incoming(json!({"id":42,"method":"item/tool/requestUserInput","params":{
        "threadId":"fixture","questions":[{"id":"choice","header":"Choice","question":"Which?",
            "options":[{"label":"One","description":"First"},{"label":"Two","description":"Second"}]}]
    }})).unwrap();
    assert!(client.wire.is_empty());
    let request = client.events.pop().unwrap();
    assert_eq!(request["method"], "input");
    let mut thread = Thread::new(1);
    thread.harness = "codex".into();
    thread.apply(request.clone());
    assert!(thread.native.questionnaire.is_some());
    client
        .command(json!({"type":"extension_ui_response","id":request["id"],
        "value":"{\"choice\":\"Two\",\"extra\":\"not allowed\"}"}))
        .unwrap();
    assert_eq!(
        client.wire[0],
        json!({"id":42,"result":{"answers":{"choice":{"answers":["Two"]}}}})
    );
}

#[test]
fn only_completed_turns_prove_a_final_reply_in_recovered_history() {
    for (status, reason) in [
        ("completed", "stop"),
        ("interrupted", "aborted"),
        ("failed", "error"),
        ("inProgress", "aborted"),
    ] {
        let mut client = Client::new(spec(SessionSource::New, "read-only"));
        client.replay(&json!({"status":status,"items":[
            {"type":"userMessage","id":"user","content":[{"type":"text","text":"Task"}]},
            {"type":"agentMessage","id":"comment","text":"Partial reply"}
        ]}));
        assert_eq!(client.history.last().unwrap()["stopReason"], reason);
    }
}

#[test]
fn failed_start_redacts_the_thread_token_and_does_not_retry() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.request(
        "thread/start",
        client.session_params(),
        json!({"type":"start"}),
    );
    let id = client.wire[0]["id"].clone();
    let error = client
        .incoming(json!({"id":id,"error":{"code":401,
        "message":"Authentication denied fixture-thread-token"}}))
        .unwrap_err();
    assert!(!error.contains("fixture-thread-token"));
    assert!(error.contains("[redacted]"));
    assert_eq!(client.wire.len(), 1);
}

#[test]
fn cancelling_an_in_flight_turn_start_sends_one_interrupt() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.session = Some("fixture".into());
    client.ready = true;
    client
        .command(json!({"type":"prompt","id":"prompt","message":"Wait"}))
        .unwrap();
    let id = client.wire[0]["id"].clone();
    client.command(json!({"type":"abort","id":"stop"})).unwrap();
    client
        .incoming(json!({"id":id,"result":{"turn":{"id":"turn"}}}))
        .unwrap();
    client
        .incoming(
            json!({"method":"turn/started","params":{"threadId":"fixture","turn":{"id":"turn"}}}),
        )
        .unwrap();
    assert_eq!(
        client
            .wire
            .iter()
            .filter(|r| r["method"] == "turn/interrupt")
            .count(),
        1
    );
}

#[test]
fn image_turns_and_local_image_replay_keep_typed_attachments() {
    let (mut thread, log) = fixture(SessionSource::New, "read-only", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    let path = crate::store::directory().unwrap().join("portrait.png");
    std::fs::write(&path, include_bytes!("../assets/crew/ember/Normal.png")).unwrap();
    let image = crate::attachments::attach_file(&path).unwrap();
    thread.draft_images.push(image);
    thread.draft.replace_text("Describe the image");
    thread.send();
    pump(&mut thread, |t| t.status == Status::Completed);
    let log = records(&log);
    let turn = log.iter().find(|r| r["method"] == "turn/start").unwrap();
    assert_eq!(turn["params"]["input"][1]["type"], "image");
    assert!(
        turn["params"]["input"][1]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    let replayed = replay_input(&json!([{"type":"localImage","path":path}]));
    assert_eq!(replayed[0]["type"], "image");
    assert!(replayed[0]["shinaAttachment"].is_object());
}

#[test]
fn manual_compaction_leaves_usage_unknown_until_a_fresh_report() {
    let (mut thread, log) = fixture(SessionSource::New, "read-only", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.apply(json!({"type":"context_usage","data":{"contextUsage":{
        "tokens":90000,"contextWindow":200000}}}));
    assert_eq!(thread.context.usage.percent, Some(45.0));
    assert!(thread.can_compact());
    thread.compact(Some("Keep the plan".into()));
    pump(&mut thread, |t| {
        t.native
            .compactions
            .last()
            .is_some_and(|r| r["running"] == true)
    });
    assert_eq!(
        thread.context.phase,
        crate::context::ContextPhase::Compacting
    );
    assert_eq!(thread.context.usage.percent, None);
    assert!(!thread.can_compact());
    pump(&mut thread, |t| !t.status.busy() && !t.native.operating());
    assert_eq!(thread.native.compactions.len(), 1);
    assert_eq!(
        thread.context.phase,
        crate::context::ContextPhase::AwaitingUsage
    );
    assert_eq!(thread.context.usage.tokens, None);
    assert!(
        thread
            .diagnostics
            .iter()
            .any(|note| note.contains("instructions were ignored"))
    );
    let log = records(&log);
    let request = log
        .iter()
        .find(|r| r["method"] == "thread/compact/start")
        .unwrap();
    assert_eq!(
        request["params"],
        json!({"threadId":"codex-fixture-thread"})
    );
    assert!(
        !log.iter()
            .any(|r| matches!(r["method"].as_str(), Some("turn/start" | "turn/steer")))
    );
    thread.draft.replace_text("Fresh usage");
    thread.send();
    pump(&mut thread, |t| {
        t.status == Status::Completed && t.context.usage.tokens == Some(1234)
    });
    assert_eq!(thread.context.phase, crate::context::ContextPhase::Idle);
    assert_eq!(thread.context.usage.context_window, Some(200000));
}

#[test]
fn compact_acknowledgement_does_not_release_a_racing_prompt() {
    let (mut thread, log) = fixture(SessionSource::New, "read-only", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.compact(None);
    pump(&mut thread, |t| !t.native.compactions.is_empty());
    // Bypass Shina's local waiting queue to exercise the transport's race guard.
    thread
        .pi
        .as_ref()
        .unwrap()
        .send(json!({
            "type":"prompt","id":"racing-prompt","message":"Wait for compaction",
            "streamingBehavior":"steer"
        }))
        .unwrap();
    pump(&mut thread, |t| !t.native.follow_ups.is_empty());
    assert_eq!(
        thread.context.phase,
        crate::context::ContextPhase::Compacting
    );
    assert!(!records(&log).iter().any(|r| r["method"] == "turn/steer"));
    pump(&mut thread, |t| {
        t.status == Status::Completed && t.context.usage.tokens == Some(1234)
    });
    let log = records(&log);
    assert_eq!(
        log.iter().filter(|r| r["method"] == "turn/start").count(),
        1
    );
    assert!(!log.iter().any(|r| r["method"] == "turn/steer"));
    assert_eq!(thread.native.compactions.len(), 1);
}

#[test]
fn ui_prompts_wait_locally_for_manual_compaction() {
    let (mut thread, log) = fixture(SessionSource::New, "read-only", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.compact(None);
    thread.draft.replace_text("Queued in Shina");
    thread.send_during_run();
    assert!(thread.draft.text.is_empty());
    pump(&mut thread, |t| {
        t.status == Status::Completed && t.messages.iter().any(|m| m.text == "Hello Codex")
    });
    assert!(records(&log).iter().any(|r| r["method"] == "turn/start"));
    assert_eq!(thread.native.compactions.len(), 1);
}

#[test]
fn native_compaction_failures_replace_progress_and_allow_retry() {
    for flag in ["--compact-failure", "--compact-rejection"] {
        let (mut thread, _) = fixture(SessionSource::New, "read-only", &[flag]);
        pump(&mut thread, |t| t.native.commands_loaded);
        thread.compact(None);
        pump(&mut thread, |t| {
            !t.status.busy() && !t.native.operating() && t.error.is_some()
        });
        assert_eq!(thread.native.compactions.len(), 1, "{flag}");
        assert!(
            thread.native.compactions[0]["errorMessage"]
                .as_str()
                .unwrap()
                .contains("Fixture compaction")
        );
        assert_eq!(thread.context.usage.percent, None);
        assert!(thread.can_compact());
    }
}

#[test]
fn automatic_and_legacy_compaction_notifications_use_the_same_rows() {
    for flags in [
        vec!["--automatic-compaction"],
        vec!["--automatic-compaction", "--legacy-compaction"],
        vec!["--automatic-compaction", "--compact-failure"],
    ] {
        let (mut thread, log) = fixture(SessionSource::New, "read-only", &flags);
        pump(&mut thread, |t| t.native.commands_loaded);
        thread.draft.replace_text("Automatic compaction");
        thread.send();
        if !flags.contains(&"--legacy-compaction") {
            pump(&mut thread, |t| {
                t.context.phase == crate::context::ContextPhase::Compacting
            });
            assert_eq!(thread.context.usage.percent, None);
        }
        pump(&mut thread, |t| !t.status.busy());
        assert_eq!(thread.native.compactions.len(), 1, "{flags:?}");
        assert_ne!(thread.native.compactions[0]["running"], true);
        assert_eq!(
            thread.native.compactions[0]["errorMessage"].is_string(),
            flags.contains(&"--compact-failure")
        );
        assert!(
            !thread
                .tools
                .iter()
                .any(|tool| tool.name == "contextCompaction")
        );
        assert!(
            !records(&log)
                .iter()
                .any(|r| r["method"] == "thread/compact/start")
        );
    }
}

#[test]
fn stop_interrupts_native_compaction_and_recovers_queued_input() {
    let (mut thread, log) = fixture(SessionSource::New, "read-only", &["--hold-compaction"]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.compact(None);
    pump(&mut thread, |t| !t.native.compactions.is_empty());
    thread.native.follow_up = true;
    thread.draft.replace_text("Keep this queued prompt");
    thread.send_during_run();
    thread.stop();
    pump(&mut thread, |t| !t.status.busy() && !t.native.operating());
    assert!(thread.draft.text.contains("Keep this queued prompt"));
    assert_eq!(thread.native.compactions.len(), 1);
    assert_eq!(thread.native.compactions[0]["aborted"], true);
    assert!(
        thread.pi.is_some(),
        "Codex supports turn/interrupt without retiring the process"
    );
    assert!(
        records(&log)
            .iter()
            .any(|r| r["method"] == "turn/interrupt")
    );
    assert_eq!(thread.context.usage.percent, None);
}

#[test]
fn cancelling_an_in_flight_compaction_start_interrupts_once() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.session = Some("fixture".into());
    client.ready = true;
    client
        .command(json!({"type":"compact","id":"compact"}))
        .unwrap();
    let request_id = client.wire[0]["id"].clone();
    client.command(json!({"type":"abort","id":"stop"})).unwrap();
    client
        .incoming(json!({"id":request_id,"result":{}}))
        .unwrap();
    assert!(client.compact_request.is_some());
    for _ in 0..2 {
        client
            .incoming(json!({"method":"turn/started","params":{
            "threadId":"fixture","turn":{"id":"compact-turn"}}}))
            .unwrap();
    }
    assert_eq!(
        client
            .wire
            .iter()
            .filter(|r| r["method"] == "turn/interrupt")
            .count(),
        1
    );
}

#[test]
fn compaction_only_turns_do_not_change_prior_reply_outcomes() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.replay(&json!({"id":"answer-turn","status":"completed","items":[
        {"type":"agentMessage","id":"answer","text":"Prior reply"}
    ]}));
    client.replay(&json!({"id":"compact-turn","status":"failed",
        "error":{"message":"Compaction failed"},"items":[
        {"type":"contextCompaction","id":"compact"}
    ]}));
    assert_eq!(client.history.last().unwrap()["stopReason"], "stop");
    assert_eq!(client.compactions[0]["errorMessage"], "Compaction failed");
}

#[test]
fn resumed_in_progress_compaction_accepts_completion_and_fresh_usage() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.replay(&json!({"id":"turn","status":"inProgress","items":[
        {"type":"contextCompaction","id":"compact"}
    ]}));
    assert!(client.compacting);
    client.item(
        &json!({"type":"contextCompaction","id":"compact"}),
        true,
        true,
    );
    assert!(!client.compacting);
    client
        .notification(
            "thread/tokenUsage/updated",
            &json!({"tokenUsage":{
                "last":{"totalTokens":1000},"modelContextWindow":200000
            }}),
        )
        .unwrap();
    assert_eq!(client.usage["tokens"], 1000);
    assert_eq!(client.compactions.len(), 1);
}

#[test]
fn legacy_notification_before_item_completion_does_not_duplicate_or_erase_fresh_usage() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client.turn = Some("turn".into());
    let item = json!({"type":"contextCompaction","id":"compact"});
    client.item(&item, false, true);
    client
        .notification("thread/compacted", &json!({"turnId":"turn"}))
        .unwrap();
    client
        .notification(
            "thread/tokenUsage/updated",
            &json!({"tokenUsage":{
                "last":{"totalTokens":1000},"modelContextWindow":200000
            }}),
        )
        .unwrap();
    client.item(&item, true, true);
    assert_eq!(client.compactions.len(), 1);
    assert_eq!(
        client
            .events
            .iter()
            .filter(|event| event["type"] == "compaction_end")
            .count(),
        1
    );
    assert_eq!(client.usage["tokens"], 1000);
}

#[test]
#[ignore = "One paid Codex prompt and native compaction in a temporary workspace, with the installed provider login"]
fn real_codex_pong_smoke() {
    let directory = crate::store::directory().unwrap();
    let local = directory.join("local-app-data");
    std::fs::create_dir_all(&local).unwrap();
    let mut command = process().unwrap();
    command.env("LOCALAPPDATA", &local);
    let mut start = spec(SessionSource::New, "read-only");
    let server = crate::mcp::server(Arc::new(|| {})).unwrap();
    let link = start.link.as_mut().unwrap();
    link.url = server.url.clone();
    link.token = server.token_for(31);
    start.instructions = None;
    let session = start_process(command, start, Arc::new(|| {})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut sent = false;
    let mut compact_sent = false;
    let mut compact_finished = false;
    let mut compact_acknowledged = false;
    let mut answer = String::new();
    loop {
        assert!(Instant::now() < deadline, "Codex smoke timed out");
        match session.events.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Record(record)) => {
                if record["command"] == "get_commands" && !sent {
                    session
                        .send(json!({"type":"prompt","id":"smoke",
                        "message":"Reply with only PONG. Do not use tools or modify files."}))
                        .unwrap();
                    sent = true;
                }
                if record["type"] == "message_update"
                    && record["assistantMessageEvent"]["type"] == "text_delta"
                {
                    answer.push_str(
                        record["assistantMessageEvent"]["delta"]
                            .as_str()
                            .unwrap_or(""),
                    );
                }
                if record["type"] == "compaction_end" {
                    assert!(record["errorMessage"].is_null(), "{record}");
                    compact_finished = true;
                }
                if record["command"] == "compact" {
                    assert_eq!(record["success"], true, "{record}");
                    compact_acknowledged = true;
                }
                if record["type"] == "agent_settled" && !compact_sent {
                    assert_eq!(answer.trim(), "PONG");
                    session
                        .send(json!({"type":"compact","id":"smoke-compact"}))
                        .unwrap();
                    compact_sent = true;
                } else if record["type"] == "agent_settled" && compact_sent {
                    assert!(compact_finished && compact_acknowledged);
                    println!(
                        "Codex real smoke returned PONG and completed native compaction in {:.1}s",
                        120.0
                            - deadline
                                .saturating_duration_since(Instant::now())
                                .as_secs_f32()
                    );
                    break;
                }
            }
            Ok(Event::Error(error)) => panic!("Codex smoke failed: {error}"),
            Ok(Event::Exited) => panic!("Codex smoke exited before settlement"),
            _ => {}
        }
    }
}
