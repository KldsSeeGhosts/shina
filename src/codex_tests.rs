use super::*;
use crate::model::{AccessMode, Status, Thread};

#[test]
fn live_child_updates_the_originating_collab_tool_after_spawn_completes() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    let mut thread = Thread::new(1);
    let spawn = json!({"type":"collabAgentToolCall","id":"spawn","tool":"spawnAgent",
        "receiverThreadIds":["child"],"prompt":"Read the docs"});
    client.item(&spawn, false, true);
    for event in std::mem::take(&mut client.events) {
        thread.apply(event);
    }
    let runs = crate::agents::runs(&thread);
    assert_eq!(runs[0].provider_id, Some("child"));
    assert_eq!(runs[0].brief, "Read the docs");
    assert_eq!(crate::agents::active_count(&thread), 1);

    client.item(&spawn, true, true);
    client
        .child_notification(
            "item/started",
            &json!({"threadId":"child","item":{
                "id":"read","type":"commandExecution","command":"git status"
            }}),
            "spawn",
        )
        .unwrap();
    for event in std::mem::take(&mut client.events) {
        thread.apply(event);
    }
    let runs = crate::agents::runs(&thread);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].activity, Some("git status"));
    assert_eq!(runs[0].provider_id, Some("child"));
    assert_eq!(crate::agents::active_count(&thread), 1);
    client
        .child_notification(
            "turn/completed",
            &json!({"threadId":"child","turn":{"status":"completed"}}),
            "spawn",
        )
        .unwrap();
    for event in std::mem::take(&mut client.events) {
        thread.apply(event);
    }
    assert_eq!(crate::agents::active_count(&thread), 0);
}

#[test]
fn approval_decisions_keep_cancel_session_and_legacy_wire_meanings() {
    for (decision, expected) in [
        ("accept", json!("accept")),
        ("acceptForSession", json!("acceptForSession")),
        ("acceptAlways", json!("acceptForSession")),
        ("decline", json!("decline")),
        ("cancel", json!("cancel")),
    ] {
        let mut client = Client::new(Start::discovery(
            "codex",
            crate::store::directory().unwrap(),
        ));
        client.answer_decision(
            Approval {
                id: json!(19),
                method: "item/commandExecution/requestApproval".into(),
                params: json!({}),
            },
            decision,
        );
        assert_eq!(client.wire.last().unwrap()["result"]["decision"], expected);
    }
}

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

fn spec(source: SessionSource, access: &str) -> Start {
    Start {
        id: "codex".into(),
        workspace: crate::store::directory().unwrap(),
        source,
        instructions: Some("Captain profile\nUser-curated essential memory: use Rust".into()),
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
    for (mode, approval, reviewer, sandbox) in [
        ("approval-required", "untrusted", "user", "readOnly"),
        ("auto-accept-edits", "on-request", "user", "workspaceWrite"),
        ("auto", "on-request", "auto_review", "workspaceWrite"),
        ("full-access", "never", "user", "dangerFullAccess"),
        ("read-only", "never", "user", "readOnly"),
    ] {
        let mut client = Client::new(spec(SessionSource::New, mode));
        client.session = Some("fixture".into());
        client.ready = true;
        client
            .command(json!({"type":"prompt","message":"test"}))
            .unwrap();
        assert_eq!(client.wire[0]["params"]["approvalPolicy"], approval);
        assert_eq!(client.wire[0]["params"]["approvalsReviewer"], reviewer);
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
    assert!(thread.draft.text.is_empty());
    assert!(thread.native.queue_held);
    assert_eq!(
        thread.native.queue[0].prompt.text,
        "Keep this queued prompt"
    );
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
fn codex_token_usage_notification_produces_estimated_and_inferred_breakdown() {
    let mut thread = Thread::new(11);
    thread.harness = "codex".into();
    thread.apply(json!({
        "type": "message_start",
        "message": { "role": "user", "content": [{ "type": "text", "text": &"a".repeat(3500) }] }
    }));
    thread.apply(json!({
        "type": "tool_execution_start",
        "toolCallId": "call-1",
        "toolName": "bash",
        "args": { "command": "ls" }
    }));
    thread.apply(json!({
        "type": "tool_execution_end",
        "toolCallId": "call-1",
        "toolName": "bash",
        "isError": false,
        "result": { "content": [{ "type": "text", "text": &"o".repeat(7000) }] }
    }));

    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    client
        .notification(
            "thread/tokenUsage/updated",
            &json!({
                "tokenUsage": {
                    "modelContextWindow": 200000,
                    "last": { "totalTokens": 10000 }
                }
            }),
        )
        .unwrap();
    for event in client.events {
        thread.apply(event);
    }

    let breakdown = thread
        .context
        .usage
        .breakdown
        .as_ref()
        .expect("expected estimated breakdown");
    assert!(breakdown.iter().any(|r| {
        r.label == "System" && r.kind == crate::context_breakdown::BreakdownRowKind::Inferred
    }));
    assert!(breakdown.iter().any(|r| {
        r.label == "Messages" && r.kind == crate::context_breakdown::BreakdownRowKind::Estimated
    }));
    assert!(breakdown.iter().any(|r| {
        r.label == "Tool output" && r.kind == crate::context_breakdown::BreakdownRowKind::Estimated
    }));
    assert_eq!(breakdown.iter().map(|r| r.tokens).sum::<u64>(), 10000);
}

#[test]
fn codex_fresh_session_issues_thread_start_and_not_resume() {
    let mut client = Client::new(Start {
        prompt_suggestions: false,
        metadata_only: false,
        id: "codex".into(),
        workspace: PathBuf::from("."),
        source: SessionSource::New,
        instructions: None,
        shina: crate::mcp::INSTRUCTIONS,
        link: Some(crate::backend::HostLink {
            url: "http://127.0.0.1:1/mcp".into(),
            token: "test-token".into(),
            extension: PathBuf::new(),
            access_mode: "workspace".into(),
        }),
        owner: Arc::default(),
        context_sent: false,
    });
    client.initialize();
    let init_id = client.wire[0]["id"].clone();
    client
        .incoming(json!({"id": init_id, "result": {"userAgent": "codex/0.120.0"}}))
        .unwrap();
    let models_id = client
        .wire
        .iter()
        .find(|msg| msg["method"] == "model/list")
        .unwrap()["id"]
        .clone();
    client
        .incoming(json!({"id": models_id, "result": {"data": []}}))
        .unwrap();
    let config_id = client
        .wire
        .iter()
        .find(|msg| msg["method"] == "config/read")
        .unwrap()["id"]
        .clone();
    client
        .incoming(json!({"id": config_id, "result": {"config": {}}}))
        .unwrap();
    let methods: Vec<&str> = client
        .wire
        .iter()
        .filter_map(|msg| msg["method"].as_str())
        .collect();
    assert!(methods.contains(&"thread/start"), "{methods:?}");
    assert!(!methods.contains(&"thread/resume"), "{methods:?}");
}

/// KSG `applyCodexModelCatalog` over a real `model/list`: manifest rows take
/// manifest names and the legacy split, unknown rows keep their wire data, and
/// models newer than the CLI become update-required entries.
#[test]
fn probe_applies_the_manifest_catalog_to_model_list() {
    let dir = crate::store::directory().unwrap();
    let log = dir.join("probe-manifest.jsonl");
    let script = dir.join("probe.py");
    std::fs::write(&script, include_str!("../assets/tests/fake_codex.py")).unwrap();
    let mut command = Command::new(if cfg!(windows) { "python" } else { "python3" });
    command.arg("-u").arg(script).arg("--log").arg(&log).args([
        "--manifest-models",
        "--cli-version",
        "0.150.0",
    ]);
    let start = Start::discovery("codex", dir);
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
                Some(true),
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
            Event::Error(error) => panic!("Codex manifest probe failed: {error}"),
            Event::Exited => break,
            Event::Diagnostic(_) => {}
        }
    }
    let updates: Vec<&Value> = output
        .iter()
        .filter(|r| r["type"] == "harness_update")
        .collect();
    let version = updates.iter().find(|r| r["field"] == "version").unwrap();
    assert_eq!(version["value"], json!("0.150.0"));
    // Manifest models the CLI predates, in manifest order.
    let required = updates
        .iter()
        .find(|r| r["field"] == "updateRequiredModels")
        .unwrap()["value"]
        .clone();
    assert_eq!(
        required,
        json!([
            {"slug":"gpt-6.1-sol","name":"GPT-6.1 Sol","minVersion":"0.153.0"},
            {"slug":"gpt-6-luna","name":"GPT-6 Luna","minVersion":"0.155.0"},
        ])
    );
    let models = output
        .iter()
        .find(|r| r["command"] == "get_available_models")
        .unwrap()["data"]["models"]
        .clone();
    let models = models.as_array().unwrap();
    // Discovery order and wire ids are preserved; the unknown slug survives.
    let ids: Vec<&str> = models.iter().filter_map(|m| m["id"].as_str()).collect();
    assert_eq!(
        ids,
        ["gpt-6-astra", "zzz-fixture", "openai.gpt-6-sol", "gpt-5.5"]
    );
    // The manifest classifies legacy rows without replacing the app-server labels.
    let names: Vec<&str> = models.iter().filter_map(|m| m["name"].as_str()).collect();
    assert_eq!(
        names,
        [
            "GPT-6 Astra Wire",
            "Fixture Unknown",
            "GPT-6 Sol Wire",
            "GPT-5.5 Wire"
        ]
    );
    assert!(models.iter().all(|m| m.get("badge").is_none()));
    assert!(models[0].get("isLegacy").is_none());
    assert!(models[1].get("isLegacy").is_none());
    assert_eq!(models[2]["isLegacy"], true);
    assert_eq!(models[3]["isLegacy"], true);
    assert_eq!(models[0]["isDefault"], true);
    // The app-server supplies capabilities, even for manifest-classified rows.
    assert_eq!(
        models[0]["thinkingLevels"],
        json!(["low", "medium", "high"])
    );
    let descriptors = models[0]["capabilities"]["optionDescriptors"]
        .as_array()
        .unwrap();
    let effort = descriptors
        .iter()
        .find(|d| d["id"] == "reasoningEffort")
        .expect("reasoning descriptor");
    assert_eq!(effort["currentValue"], json!("medium"));
    assert_eq!(
        effort["options"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|o| o["id"].as_str())
            .collect::<Vec<_>>(),
        ["low", "medium", "high"]
    );
    assert!(descriptors.iter().any(|d| d["id"] == "serviceTier"));
    assert_eq!(models[1]["thinkingLevels"], json!(["xhigh"]));
    assert_eq!(models[1]["contextWindow"], json!(32000));
}

/// CPA wiring: provider env vars inherit untouched, Shina internals are
/// removed, and the originator matches KSG's `CODEX_INTERNAL_ORIGINATOR_OVERRIDE`.
#[test]
fn process_preserves_cpa_environment_and_marks_the_shina_originator() {
    struct Saved(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for Saved {
        fn drop(&mut self) {
            for (key, value) in self.0.drain(..) {
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }
    let saved = Saved(
        [
            "SHINA_CODEX_CLI",
            "CODEX_HOME",
            "OPENAI_BASE_URL",
            "OPENAI_API_KEY",
            crate::mcp::URL_ENV,
            crate::mcp::TOKEN_ENV,
        ]
        .into_iter()
        .map(|key| (key, std::env::var_os(key)))
        .collect(),
    );
    let dir = crate::store::directory().unwrap();
    let cli = dir.join("codex-entry.js");
    std::fs::write(&cli, "// fixture").unwrap();
    unsafe {
        std::env::set_var("SHINA_CODEX_CLI", &cli);
        std::env::set_var("CODEX_HOME", dir.join("codex-home"));
        std::env::set_var("OPENAI_BASE_URL", "http://127.0.0.1:8317/v1");
        std::env::set_var("OPENAI_API_KEY", "fixture-cpa-key");
        std::env::set_var(crate::mcp::URL_ENV, "http://127.0.0.1:9/mcp");
        std::env::set_var(crate::mcp::TOKEN_ENV, "other-thread-token");
    }
    let command = process().unwrap();
    // CPA/provider variables are inherited, never removed.
    for key in [
        "CODEX_HOME",
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "CPA_API_KEY",
        "ANTHROPIC_BASE_URL",
        "OPENAI_MODEL",
    ] {
        assert!(
            command.get_envs().all(|(name, _)| name != key),
            "{key} was removed"
        );
    }
    // Thread-scoped Shina variables never reach Codex.
    for removed in [
        crate::mcp::URL_ENV,
        crate::mcp::TOKEN_ENV,
        "SHINA_RUNTIME_MODE",
        "SHINA_PI_BRIDGE_PATH",
    ] {
        assert!(
            command
                .get_envs()
                .any(|(name, value)| name == removed && value.is_none()),
            "{removed} is still set"
        );
    }
    assert_eq!(
        command
            .get_envs()
            .find(|(name, _)| *name == "CODEX_INTERNAL_ORIGINATOR_OVERRIDE")
            .and_then(|(_, value)| value),
        Some(std::ffi::OsStr::new("shina"))
    );
    let args: Vec<String> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert!(args.iter().any(|arg| arg == "app-server"));
    assert!(
        args.windows(2)
            .any(|pair| pair == ["-c", "thread_unload_delay_secs=0"]),
        "KSG's app-server unload override is missing: {args:?}"
    );
    drop(saved);
}

/// Custom CPA slugs keep their wire ids and stay selectable even when
/// `model/list` never advertised them.
#[test]
fn cpa_and_custom_model_slugs_stay_selectable() {
    let row = model_value(
        &json!({"model":"opencode-go/glm-5.3","displayName":"GLM 5.3"}),
        None,
    )
    .unwrap();
    assert_eq!(row["id"], "opencode-go/glm-5.3");
    assert_eq!(row["name"], "GLM 5.3");
    for slug in [
        "gpt-6.2-sol",
        "gpt-6.2-astra",
        "gpt-6.1-terra",
        "cursor-grok-4.6",
        "opencode-go/deepseek-v4.1-flash",
        "opencode-go/glm-5.3",
    ] {
        let mut client = Client::new(spec(SessionSource::New, "full-access"));
        client.ready = true;
        client
            .command(json!({"type":"set_model","id":"pick","modelId":slug}))
            .unwrap();
        assert_eq!(client.model["id"], slug, "{slug} was rejected");
        let reply = client
            .events
            .iter()
            .find(|event| event["command"] == "set_model")
            .expect("set_model reply");
        assert_eq!(reply["success"], true);
        assert_eq!(reply["data"]["id"], slug);
    }
}
