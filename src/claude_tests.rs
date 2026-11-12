use super::*;
use crate::model::{AccessMode, Status, Thread};

const SESSION: &str = "40000000-0000-4000-8000-000000000031";

fn spec(source: SessionSource, access: &str) -> Start {
    Start {
        id: "claude".into(),
        workspace: crate::store::directory().unwrap(),
        source,
        instructions: Some("Captain profile\nEssential memory: use Rust".into()),
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

fn client(access: &str) -> Client {
    let mut client = Client::new(spec(SessionSource::New, access));
    client
        .initialized(
            &json!({"hook_on_failure_supported":true,"hooks_applied":true,
        "commands":[],"models":[{"value":"default","supportedEffortLevels":["low","high"]}]}),
        )
        .unwrap();
    client.events.clear();
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
    assert_eq!(thread.models[0].thinking_levels, ["low", "high"]);
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
        "Bearer fixture-thread-token"
    );
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
        for name in ["Read", "Edit", "Bash", "mcp__server__mutate", "UnknownTool"] {
            let mut client = client(access);
            client
                .server_request(
                    &json!({"request_id":"p","request":{"subtype":"can_use_tool",
                "tool_name":name,"tool_use_id":"call","input":{"path":"x"}}}),
                )
                .unwrap();
            let ask = access == "approval-required" && name != "Read"
                || access == "auto-accept-edits" && !matches!(name, "Read" | "Edit");
            assert_eq!(!client.approvals.is_empty(), ask, "{access} {name}");
            if ask {
                client
                    .command(json!({"type":"extension_ui_response","id":"p","confirmed":true}))
                    .unwrap();
            }
            let expected = if access == "read-only" && name != "Read" {
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
    for name in [
        "Write",
        "Bash",
        "PowerShell",
        "Task",
        "Agent",
        "Skill",
        "mcp__shina__delegate_task",
        "ExitPlanMode",
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
            "deny"
        );
        assert!(client.approvals.is_empty());
    }
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
    assert!(
        records
            .iter()
            .any(|r| r["request"]["subtype"] == "set_model")
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
fn resume_history_reconstructs_branch_and_never_replaces_missing_session() {
    let mut client = client("read-only");
    client.start.source = SessionSource::Resume(SESSION.into());
    client.session = SESSION.into();
    // read_history is tested independently so tests never consult real Claude history.
    let path = crate::store::directory().unwrap().join("history.jsonl");
    std::fs::write(&path,format!("{}\n{}\n",
        json!({"type":"user","uuid":"u","parentUuid":null,"message":{"content":"Prior prompt"}}),
        json!({"type":"assistant","uuid":"a","parentUuid":"u","message":{"content":[{"type":"text","text":"Prior reply"}]}}))).unwrap();
    for record in read_history(&path).unwrap() {
        client.message(&record, false);
    }
    assert_eq!(client.history[1]["content"][0]["text"], "Prior reply");
    assert!(
        !json!(client.history)
            .to_string()
            .contains("fixture-thread-token")
    );
    let mut command = Command::new("fixture");
    client.launch(&mut command);
    let args: Vec<_> = command
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(args.windows(2).any(|a| a == ["--resume", SESSION]));
    assert!(!args.iter().any(|a| a == "--session-id"));
    assert!(
        start_process(
            Command::new("fixture"),
            spec(SessionSource::Resume("../invalid".into()), "read-only"),
            Arc::new(|| {})
        )
        .is_err()
    );
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
            assert!(error.contains("Sign in with the Claude CLI"));
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
fn fixture_skill_discovery_and_prompt_translation() {
    let workspace = crate::store::directory().unwrap();
    let path = workspace.join(".claude/skills/review");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("SKILL.md"),
        "---\ndescription: Review the patch\n---\nInstructions",
    )
    .unwrap();
    assert!(
        discover_skills(&workspace.join("config"), &workspace)
            .iter()
            .any(|c| c["name"] == "skill:review")
    );
    assert_eq!(
        input_blocks(&json!({"message":"/skill:review now"})).unwrap()[0]["text"],
        "/review now"
    );
    assert_eq!(
        resolve_process(vec![PathBuf::from("/definitely-missing/shina-claude")]).unwrap_err(),
        MISSING
    );
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
fn failed_turn_is_not_retried_and_compaction_failure_closes_progress() {
    let (mut thread, log) = fixture("full-access", &[]);
    pump(&mut thread, |t| t.native.commands_loaded);
    thread.draft.replace_text("error");
    thread.send();
    pump(&mut thread, |t| t.status == Status::Failed);
    assert!(
        thread
            .error
            .as_deref()
            .unwrap()
            .contains("Fixture execution failed")
    );
    assert_eq!(
        records(&log)
            .iter()
            .filter(|r| r["message"]["content"][0]["text"] == "error")
            .count(),
        1
    );
    let mut client = client("full-access");
    client
        .command(json!({"type":"compact","id":"c","customInstructions":"fail"}))
        .unwrap();
    assert!(client.compacting);
    client
        .incoming(json!({"type":"result","is_error":true,"errors":["Compaction failed"]}))
        .unwrap();
    assert!(!client.compacting);
    assert!(
        client
            .events
            .iter()
            .any(|r| r["type"] == "compaction_end" && r["errorMessage"] == "Compaction failed")
    );
    assert!(
        client
            .events
            .iter()
            .any(|r| r["command"] == "compact" && r["success"] == false)
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
fn approved_plan_restores_access_before_followup_and_withdrawn_question_closes() {
    let mut client = client("full-access");
    client.running = true;
    client
        .server_request(
            &json!({"request_id":"plan","request":{"subtype":"can_use_tool",
        "tool_name":"ExitPlanMode","tool_use_id":"exit","input":{"plan":"# Approved plan"}}}),
        )
        .unwrap();
    client
        .server_request(
            &json!({"request_id":"unsafe","request":{"subtype":"can_use_tool",
        "tool_name":"Bash","input":{"command":"echo unsafe"}}}),
        )
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["response"]["response"]["behavior"],
        "deny"
    );
    client
        .command(json!({"type":"extension_ui_response","id":"plan","confirmed":true}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["response"]["response"]["behavior"],
        "deny"
    );
    assert_eq!(client.queued.len(), 1);
    client
        .incoming(json!({"type":"result","is_error":false}))
        .unwrap();
    let request = client.wire.last().unwrap().clone();
    assert_eq!(request["request"]["mode"], "bypassPermissions");
    client
        .incoming(
            json!({"type":"control_response","response":{"subtype":"success",
        "request_id":request["request_id"],"response":{}}}),
        )
        .unwrap();
    assert!(
        client.wire.last().unwrap()["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Implement")
    );
    client
        .server_request(
            &json!({"request_id":"question","request":{"subtype":"can_use_tool",
        "tool_name":"AskUserQuestion","input":{"questions":[{"question":"Choose","options":[]}]}}}),
        )
        .unwrap();
    client
        .incoming(json!({"type":"control_cancel_request","request_id":"question"}))
        .unwrap();
    assert!(!client.approvals.contains_key("question"));
    let mut thread = Thread::new(1);
    thread.harness = "claude".into();
    thread.apply(
        json!({"type":"extension_ui_request","id":"question","method":"input",
        "questions":[{"question":"Choose","options":[]}]}),
    );
    assert!(thread.request.is_some());
    thread.apply(json!({"type":"extension_ui_cancel","id":"question"}));
    assert!(thread.request.is_none());
    assert!(thread.native.questionnaire.is_none());
}

#[test]
fn blocking_hook_is_required_and_secret_errors_are_redacted() {
    let mut client = Client::new(spec(SessionSource::New, "read-only"));
    assert!(
        client
            .initialized(&json!({"hook_on_failure_supported":false}))
            .is_err()
    );
    client.initialize();
    let id = client.wire.last().unwrap()["request_id"].clone();
    let error = client
        .incoming(
            json!({"type":"control_response","response":{"subtype":"error",
        "request_id":id,"error":"Authentication failed fixture-thread-token"}}),
        )
        .unwrap_err();
    assert!(!error.contains("fixture-thread-token"));
    assert!(error.contains("[redacted]"));
}

#[test]
fn automatic_compaction_stop_and_bounded_input_queues() {
    let mut client = client("full-access");
    client.running = true;
    client
        .incoming(json!({"type":"system","subtype":"status","status":"compacting"}))
        .unwrap();
    client.command(json!({"type":"abort","id":"stop"})).unwrap();
    client
        .incoming(json!({"type":"result","is_error":false}))
        .unwrap();
    assert_eq!(
        client
            .events
            .iter()
            .filter(|r| r["type"] == "compaction_start")
            .count(),
        1
    );
    assert_eq!(
        client
            .events
            .iter()
            .filter(|r| r["type"] == "compaction_end" && r["aborted"] == true)
            .count(),
        1
    );
    client.running = true;
    for _ in 0..33 {
        client
            .command(json!({"type":"prompt","message":"queued","streamingBehavior":"followUp"}))
            .unwrap();
    }
    assert_eq!(client.queued.len(), 32);
    assert!(client.events.iter().any(|r| {
        r["success"] == false
            && r["error"]
                .as_str()
                .is_some_and(|s| s.contains("queue is full"))
    }));
}

#[test]
fn compaction_status_reports_failure_even_when_result_reports_success() {
    let mut client = client("full-access");
    client.command(json!({"type":"compact","id":"c"})).unwrap();
    client
        .incoming(json!({"type":"system","subtype":"status","status":null,
        "compact_result":"failed","compact_error":"Usage window exhausted"}))
        .unwrap();
    assert!(!client.compacting);
    client
        .incoming(json!({"type":"result","subtype":"success","is_error":false}))
        .unwrap();
    assert_eq!(
        client
            .events
            .iter()
            .filter(|e| e["type"] == "compaction_end")
            .count(),
        1
    );
    assert!(
        client
            .events
            .iter()
            .any(|e| e["command"] == "compact" && e["success"] == false)
    );
    assert!(
        client.events.iter().any(|e| e["type"] == "message_end"
            && e["message"]["errorMessage"] == "Usage window exhausted")
    );
}

#[test]
fn plugin_and_managed_scope_skills_use_metadata_not_credentials() {
    let directory = crate::store::directory().unwrap();
    let config = directory.join("config");
    let plugin = directory.join("plugin");
    std::fs::create_dir_all(config.join("plugins")).unwrap();
    std::fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
    std::fs::create_dir_all(plugin.join("custom/review")).unwrap();
    std::fs::write(
        config.join("settings.json"),
        "{ // comments are accepted by Claude\n\"enabledPlugins\":{\"fixture@market\":true,},}",
    )
    .unwrap();
    std::fs::write(
        config.join("plugins/installed_plugins.json"),
        json!({"plugins":{
            "fixture@market":[{"scope":"managed","installPath":plugin}]
        }})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        plugin.join(".claude-plugin/plugin.json"),
        r#"{"name":"custom-name","skills":["./custom"]}"#,
    )
    .unwrap();
    std::fs::write(
        plugin.join("custom/review/SKILL.md"),
        "---\ndescription: >\n  Review changes\n  carefully\n---\nBody",
    )
    .unwrap();
    let skills = discover_skills(&config, &directory);
    let skill = skills
        .iter()
        .find(|s| s["name"] == "skill:custom-name:review")
        .unwrap();
    assert_eq!(
        skill["description"].as_str().unwrap().trim(),
        "Review changes carefully"
    );
    assert!(!skills.iter().any(|s| s["name"] == "skill:fixture:review"));
    assert!(skill_frontmatter("---\ndescription: A label: with a colon\n---\nBody").is_some());
}

#[test]
#[ignore = "One paid real Claude prompt, requires an already authenticated CLI"]
fn real_claude_pong_smoke() {
    // Uses an isolated Shina profile and workspace, but the CLI's existing auth.
    let mut thread = Thread::new(31);
    thread.harness = "claude".into();
    thread.native.access = AccessMode::ReadOnly;
    let mut start = spec(SessionSource::New, "read-only");
    start.link = Some(
        crate::mcp::server(Arc::new(|| {}))
            .map(|server| crate::backend::HostLink {
                url: server.url.clone(),
                token: server.token_for(31),
                extension: PathBuf::new(),
                access_mode: "read-only".into(),
            })
            .unwrap(),
    );
    thread.pi = Some(super::start(start, Arc::new(|| {})).unwrap());
    pump(&mut thread, |t| t.native.commands_loaded);
    thread
        .draft
        .replace_text("Reply only PONG. Do not use tools.");
    thread.send();
    pump_for(
        &mut thread,
        |t| t.status == Status::Completed,
        Duration::from_secs(90),
    );
    assert!(thread.messages.iter().any(|m| m.text.contains("PONG")));
}
