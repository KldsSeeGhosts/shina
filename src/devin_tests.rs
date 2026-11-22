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
