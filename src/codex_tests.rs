use super::*;
use crate::model::{AccessMode, Status, Thread};

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
