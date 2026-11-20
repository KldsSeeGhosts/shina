use super::*;
use std::time::{Duration, Instant};

fn workspace() -> PathBuf {
    crate::store::directory().unwrap()
}

fn wait_for_exit(terminals: &Terminals, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = terminals.exit_status_of(id) {
            return status;
        }
        assert!(Instant::now() < deadline, "terminal {id} did not exit");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn session_updates_route_child_work_to_the_subagent_context_parent() {
    let notification = json!({"sessionId":"root-session","update":{"sessionUpdate":"tool_call",
        "toolCallId":"shared-tool","title":"Child work",
        "_meta":{"cognition.ai/subagent_context":{"parentAgentId":"child-1"}}}});
    let routed = normalize_session_update(&notification);
    assert_eq!(routed["sessionId"], "child-1");
    assert_eq!(routed["update"]["toolCallId"], "shared-tool");
    // The root agent id and missing or non-string parents keep the wire session.
    for meta in [
        json!({"cognition.ai/subagent_context":{"parentAgentId":"root"}}),
        json!({"cognition.ai/subagent_context":{}}),
        json!({"cognition.ai/subagent_context":{"parentAgentId":42}}),
        json!({"cognition.ai/subagent_context":["child-1"]}),
        json!(null),
    ] {
        let notification = json!({"sessionId":"root-session","update":{"sessionUpdate":"agent_message_chunk",
            "content":{"type":"text","text":"hi"},"_meta":meta}});
        assert_eq!(
            normalize_session_update(&notification)["sessionId"],
            "root-session",
            "{meta}"
        );
    }
}

#[test]
fn streaming_message_ids_lift_into_message_and_thought_chunks_only() {
    for kind in ["agent_message_chunk", "agent_thought_chunk"] {
        let notification = json!({"sessionId":"s","update":{"sessionUpdate":kind,
            "content":{"type":"text","text":"hi"},
            "_meta":{"cognition.ai/streamingMessageId":"msg-1"}}});
        let routed = normalize_session_update(&notification);
        assert_eq!(routed["update"]["messageId"], "msg-1", "{kind}");
    }
    for (kind, id) in [
        ("tool_call", json!("msg-1")),
        ("agent_message_chunk", json!(42)),
        ("agent_message_chunk", Value::Null),
    ] {
        let notification = json!({"sessionId":"s","update":{"sessionUpdate":kind,
            "content":{"type":"text","text":"hi"},
            "_meta":{"cognition.ai/streamingMessageId":id}}});
        let routed = normalize_session_update(&notification);
        assert!(routed["update"]["messageId"].is_null(), "{kind} {id}");
    }
}

#[test]
fn tool_call_titles_come_from_inference_names_only_when_generic() {
    for (title, expected) in [
        (Value::Null, "read_file"),
        (json!("Tool"), "read_file"),
        (json!(""), "read_file"),
        (json!("Read file"), "Read file"),
    ] {
        let mut tool =
            json!({"toolCallId":"t","_meta":{"cognition.ai/inferenceToolName":"read_file"}});
        tool["title"] = title.clone();
        assert_eq!(normalize_tool_call(&tool)["title"], expected, "{title}");
    }
    // Without an inference name or with a non-string one the title is untouched.
    assert!(normalize_tool_call(&json!({"toolCallId":"t"}))["title"].is_null());
    let wrong_type = json!({"toolCallId":"t","title":"Tool",
        "_meta":{"cognition.ai/inferenceToolName":42}});
    assert_eq!(normalize_tool_call(&wrong_type)["title"], "Tool");
}

#[test]
fn subagent_updates_extract_the_full_lifecycle() {
    let started = json!({"toolCallId":"t","_meta":{"cognition.ai/subagent_started":{
        "agentId":"child-1","task":"Research the API","title":"Researcher","model":"swe-2"}}});
    let update = extract_subagent_update(&started).unwrap();
    assert_eq!(update.native_task_id, "child-1");
    assert_eq!(update.child_session_id, "child-1");
    assert_eq!(update.prompt, "Research the API");
    assert_eq!(update.title.as_deref(), Some("Researcher"));
    assert_eq!(update.model.as_deref(), Some("swe-2"));
    assert_eq!(update.status, "running");
    assert!(update.result.is_none());

    // Completion wins identity and status; started metadata still fills the prompt.
    for (success, status) in [(json!(true), "completed"), (json!(false), "failed")] {
        let tool = json!({"toolCallId":"t","_meta":{
            "cognition.ai/subagent_started":{"agentId":"child-1","task":"Research","title":"Researcher"},
            "cognition.ai/subagent_completed":{"agentId":"child-1","success":success,"summary":"Done."}}});
        let update = extract_subagent_update(&tool).unwrap();
        assert_eq!(update.status, status, "{success}");
        assert_eq!(update.result.as_deref(), Some("Done."));
        assert_eq!(update.prompt, "Research");
    }

    let only_completed =
        json!({"_meta":{"cognition.ai/subagent_completed":{"agentId":"child-9","success":true}}});
    let update = extract_subagent_update(&only_completed).unwrap();
    assert_eq!(update.status, "completed");
    assert!(update.prompt.is_empty() && update.title.is_none() && update.model.is_none());
    assert!(update.result.is_none());

    for meta in [
        json!({"cognition.ai/subagent_started":{}}),
        json!({"cognition.ai/subagent_started":{"agentId":7}}),
        json!({"cognition.ai/subagent_completed":{"agentId":null}}),
        json!({"cognition.ai/other":{"agentId":"x"}}),
        json!(null),
    ] {
        assert!(
            extract_subagent_update(&json!({"_meta":meta})).is_none(),
            "{meta}"
        );
    }
}

#[test]
fn plan_bypass_is_never_an_answerable_permission_option() {
    let request = json!({"options":[
        {"optionId":"plan_bypass","kind":"allow_once","name":"Bypass"},
        {"optionId":"plan_accept_edits","kind":"allow_once","name":"Accept edits"},
        {"optionId":" plan_bypass ","kind":"allow_always","name":"Spaced bypass"},
        {"optionId":"plan_reject","kind":"reject_once","name":"Reject"},
        {"kind":"allow_once","name":"Unnamed"}
    ]});
    let options = answerable_permission_options(&request);
    assert_eq!(options.len(), 3);
    assert_eq!(options[0]["optionId"], "plan_accept_edits");
    assert_eq!(options[1]["optionId"], "plan_reject");
    assert!(options[2]["optionId"].is_null());
    assert!(answerable_permission_options(&json!({"options":[]})).is_empty());
    assert!(answerable_permission_options(&json!({})).is_empty());
}

#[test]
fn the_builtin_agent_runs_devin_acp_with_credential_passthrough_names() {
    let agent = agent(PathBuf::from("devin"));
    assert_eq!(agent.id, "devin");
    assert_eq!(agent.name, "Devin");
    assert_eq!(agent.command, "devin");
    assert_eq!(agent.args, ["acp"]);
    assert_eq!(agent.env, ["DEVIN_TOKEN", "DEVIN_API_KEY"]);
    // Hermetic like antigravity::cli_available: discovery never consults the host.
    assert!(discover_command().is_none());
}

#[test]
fn registry_cache_prefers_the_newest_install_for_the_platform() {
    let root = workspace().join("tools");
    let platform = platform_targets()[0].clone();
    let foreign = if platform == "windows-x86_64" {
        "linux-x86_64"
    } else {
        "windows-x86_64"
    };
    let executable = if cfg!(windows) { "devin.exe" } else { "devin" };
    for version in ["3000.12.57", "3000.12.9", "3000.12.100"] {
        let directory = root.join("devin").join(version).join(&platform).join("bin");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(executable), b"binary").unwrap();
    }
    // A newer version published for another platform does not win.
    let foreign_install = root
        .join("devin")
        .join("9999.0.0")
        .join(foreign)
        .join("bin");
    std::fs::create_dir_all(&foreign_install).unwrap();
    std::fs::write(foreign_install.join("devin"), b"binary").unwrap();
    let found = cached_command_in(&root).unwrap();
    assert_eq!(
        found,
        root.join("devin")
            .join("3000.12.100")
            .join(&platform)
            .join("bin")
            .join(executable)
    );
    assert!(cached_command_in(&workspace().join("no-such-tools")).is_none());
}

#[test]
fn natural_order_compares_version_parts_numerically() {
    assert_eq!(
        natural_order("3000.12.100", "3000.12.9"),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        natural_order("3000.9.0", "3000.12.0"),
        std::cmp::Ordering::Less
    );
    assert_eq!(natural_order("v2", "v2"), std::cmp::Ordering::Equal);
}

#[test]
fn terminals_run_shell_source_capture_output_and_report_exit() {
    let terminals = Terminals::default();
    let id = terminals
        .create(
            &json!({"sessionId":"s1","command":"echo hello-devin"}),
            &workspace(),
        )
        .unwrap();
    let status = wait_for_exit(&terminals, &id);
    assert_eq!(status["exitCode"], 0);
    assert!(status["signal"].is_null());
    let output = terminals
        .output(&json!({"sessionId":"s1","terminalId":id}))
        .unwrap();
    assert!(
        output["output"].as_str().unwrap().contains("hello-devin"),
        "{output}"
    );
    assert_eq!(output["truncated"], false);
    assert_eq!(output["exitStatus"]["exitCode"], 0);
}

#[test]
fn terminals_accept_an_executable_and_args_without_a_shell() {
    let (program, args) = if cfg!(windows) {
        ("cmd", vec!["/d", "/s", "/c", "echo", "direct-args"])
    } else {
        ("sh", vec!["-c", "echo direct-args"])
    };
    let terminals = Terminals::default();
    let id = terminals
        .create(
            &json!({"sessionId":"s1","command":program,"args":args}),
            &workspace(),
        )
        .unwrap();
    wait_for_exit(&terminals, &id);
    let output = terminals
        .output(&json!({"sessionId":"s1","terminalId":id}))
        .unwrap();
    assert!(
        output["output"].as_str().unwrap().contains("direct-args"),
        "{output}"
    );
}

#[test]
fn terminal_calls_are_scoped_to_the_owning_session() {
    let terminals = Terminals::default();
    let id = terminals
        .create(
            &json!({"sessionId":"s1","command":"echo scoped"}),
            &workspace(),
        )
        .unwrap();
    wait_for_exit(&terminals, &id);
    let params = json!({"sessionId":"other","terminalId":id});
    assert!(
        terminals
            .output(&params)
            .unwrap_err()
            .contains("unknown terminal ID")
    );
    assert!(
        terminals
            .kill(&params)
            .unwrap_err()
            .contains("unknown terminal ID")
    );
    assert!(
        terminals
            .release(&params)
            .unwrap_err()
            .contains("unknown terminal ID")
    );
    assert!(
        terminals
            .exit_status(&params)
            .unwrap_err()
            .contains("unknown terminal ID")
    );
}

#[test]
fn releasing_a_terminal_ends_calls_but_keeps_the_snapshot() {
    let terminals = Terminals::default();
    let id = terminals
        .create(
            &json!({"sessionId":"s1","command":"echo kept"}),
            &workspace(),
        )
        .unwrap();
    wait_for_exit(&terminals, &id);
    terminals
        .release(&json!({"sessionId":"s1","terminalId":id}))
        .unwrap();
    assert!(
        terminals
            .output(&json!({"sessionId":"s1","terminalId":id}))
            .unwrap_err()
            .contains("unknown terminal ID")
    );
    let snapshot = terminals.output_snapshot("s1", &id).unwrap();
    assert!(snapshot.contains("kept"), "{snapshot}");
    // Snapshots stay scoped to the owning session.
    assert!(terminals.output_snapshot("other", &id).is_none());
}

#[test]
fn killing_a_terminal_stops_the_process() {
    let script = if cfg!(windows) {
        "timeout /t 60 /nobreak"
    } else {
        "sleep 60"
    };
    let terminals = Terminals::default();
    let id = terminals
        .create(&json!({"sessionId":"s1","command":script}), &workspace())
        .unwrap();
    terminals
        .kill(&json!({"sessionId":"s1","terminalId":id}))
        .unwrap();
    let status = wait_for_exit(&terminals, &id);
    assert!(
        !status["exitCode"].is_null() || !status["signal"].is_null(),
        "{status}"
    );
}

#[test]
fn terminal_output_truncates_at_the_request_limit() {
    let script = if cfg!(windows) {
        "for /l %i in (1,1,100) do @echo 0123456789"
    } else {
        "i=0; while [ $i -lt 100 ]; do echo 0123456789; i=$((i+1)); done"
    };
    let terminals = Terminals::default();
    let id = terminals
        .create(
            &json!({"sessionId":"s1","command":script,"outputByteLimit":128}),
            &workspace(),
        )
        .unwrap();
    wait_for_exit(&terminals, &id);
    let output = terminals
        .output(&json!({"sessionId":"s1","terminalId":id}))
        .unwrap();
    assert_eq!(output["truncated"], true);
    assert!(
        output["output"].as_str().unwrap().len() <= 128,
        "{}",
        output["output"].as_str().unwrap().len()
    );
}

#[test]
fn terminal_environments_drop_bridge_credentials_and_apply_request_entries() {
    // Unique name keeps the mutation local to this test even under parallel runs.
    unsafe { std::env::set_var("SHINA_DEVIN_TEST_FLAG", "1") };
    let merged = terminal_environment(&[json!({"name":"DEVIN_TEST_MARK","value":"kept"})]);
    unsafe { std::env::remove_var("SHINA_DEVIN_TEST_FLAG") };
    assert!(
        !merged
            .iter()
            .any(|(name, _)| name == "SHINA_DEVIN_TEST_FLAG")
    );
    assert!(
        merged
            .iter()
            .any(|(name, value)| name == "DEVIN_TEST_MARK" && value == "kept")
    );
    assert!(
        merged
            .iter()
            .any(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("PATH"))
    );
}

#[cfg(windows)]
#[test]
fn request_environment_replaces_inherited_names_case_insensitively() {
    let merged = terminal_environment(&[json!({"name":"path","value":"request-path"})]);
    let entries: Vec<_> = merged
        .iter()
        .filter(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("path"))
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1, "request-path");
}
