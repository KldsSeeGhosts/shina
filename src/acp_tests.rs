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
fn approval_option_ids_trim_whitespace_without_changing_literal_interaction_values() {
    let request = json!({"id":42,"params":{"sessionId":"session",
        "toolCall":{"toolCallId":"tool","kind":"execute"},
        "options":[{"optionId":" once ","kind":"allow_once"},
            {"optionId":" allow-edits-session ","kind":"allow_always"},
            {"optionId":" no ","kind":"reject_once"}]}});
    for (decision, expected) in [
        ("accept", "once"),
        ("acceptForSession", "allow-edits-session"),
        ("decline", "no"),
    ] {
        let mut client = Client::new(spec(SessionSource::New));
        client.permission(&request).unwrap();
        let key = client.events.last().unwrap()["id"].clone();
        client
            .command(json!({"type":"extension_ui_response","id":key,"decision":decision}))
            .unwrap();
        assert_eq!(
            client.wire.last().unwrap()["result"]["outcome"]["optionId"],
            expected
        );
    }
    assert!(
        acp_approval_options("grok", &request["params"])
            .unwrap()
            .iter()
            .any(|o| o.decision == "acceptForSession")
    );
    let mut blank = request;
    blank["params"]["options"][0]["optionId"] = json!("   ");
    let mut client = Client::new(spec(SessionSource::New));
    client.permission(&blank).unwrap();
    let key = client.events.last().unwrap()["id"].clone();
    client
        .command(json!({"type":"extension_ui_response","id":key,"decision":"accept"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["result"]["outcome"]["outcome"],
        "cancelled"
    );
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
fn an_aborted_settle_keeps_queued_followups_for_the_next_settled_turn() {
    let mut client = Client::new(spec(SessionSource::New));
    ready(&mut client);
    client
        .command(json!({"type":"prompt","id":"run","message":"work"}))
        .unwrap();
    client
        .command(json!({"type":"prompt","id":"follow","message":"next"}))
        .unwrap();
    assert_eq!(client.queued.len(), 1);
    client.command(json!({"type":"abort","id":"stop"})).unwrap();
    let prompt = client
        .wire
        .iter()
        .find(|r| r["method"] == "session/prompt")
        .unwrap()
        .clone();
    client
        .incoming(json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"cancelled"}}))
        .unwrap();
    assert_eq!(client.queued.len(), 1);
    assert_eq!(
        client
            .events
            .iter()
            .filter(|e| e["type"] == "agent_settled")
            .count(),
        1
    );
    let prompts = |client: &Client| {
        client
            .wire
            .iter()
            .filter(|r| r["method"] == "session/prompt")
            .count()
    };
    assert_eq!(prompts(&client), 1);

    // Stop followed by new work: the next settled turn drains the retained queue.
    client
        .command(json!({"type":"prompt","id":"new","message":"fresh"}))
        .unwrap();
    assert_eq!(prompts(&client), 2);
    let new = client.wire.last().unwrap().clone();
    client
        .incoming(json!({"jsonrpc":"2.0","id":new["id"],"result":{"stopReason":"end_turn"}}))
        .unwrap();
    assert_eq!(prompts(&client), 3);
    assert!(client.queued.is_empty());
    assert_eq!(
        client.wire.last().unwrap()["params"]["prompt"][0]["text"],
        "next"
    );
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

fn devin_client() -> Client {
    let mut start = spec(SessionSource::New);
    start.id = "devin".into();
    let mut client = Client::new(start);
    client.ready = true;
    client.session = Some("root-session".into());
    client
}

#[test]
fn devin_initialization_advertises_terminals_and_cognition_capabilities() {
    let mut start = spec(SessionSource::New);
    start.id = "devin".into();
    let mut client = Client::new(start);
    client.initialize();
    let request = client
        .wire
        .iter()
        .find(|r| r["method"] == "initialize")
        .unwrap();
    let capabilities = &request["params"]["clientCapabilities"];
    assert_eq!(capabilities["terminal"], true);
    assert_eq!(capabilities["_meta"]["cognition.ai/subagentSupport"], true);
    assert_eq!(capabilities["_meta"]["cognition.ai/messageGrouping"], true);
    // Other ACP agents keep the unadvertised defaults.
    let mut client = Client::new(spec(SessionSource::New));
    client.initialize();
    let request = client
        .wire
        .iter()
        .find(|r| r["method"] == "initialize")
        .unwrap();
    assert_eq!(request["params"]["clientCapabilities"]["terminal"], false);
    assert!(request["params"]["clientCapabilities"]["_meta"].is_null());
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
fn devin_subagent_tools_project_the_subagent_lifecycle() {
    let mut client = devin_client();
    client.running = true;
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"root-session","update":{"sessionUpdate":"tool_call","toolCallId":"spawn",
            "title":"Tool","kind":"subagent","status":"in_progress",
            "_meta":{"cognition.ai/subagent_started":{"agentId":"child-1","task":"Research","title":"Researcher","model":"swe-2"},
                "cognition.ai/inferenceToolName":"delegate_task"}}}}))
        .unwrap();
    let start = client
        .events
        .iter()
        .find(|event| event["type"] == "tool_execution_start")
        .unwrap();
    // The started meta's title wins over even the inferred tool name.
    assert_eq!(start["toolName"], "Researcher");
    assert_eq!(start["toolKind"], "subagent");
    assert!(client.devin_children.contains("child-1"));
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"root-session","update":{"sessionUpdate":"tool_call_update","toolCallId":"spawn",
            "status":"completed",
            "_meta":{"cognition.ai/subagent_completed":{"agentId":"child-1","success":true,"summary":"Finished."}}}}}))
        .unwrap();
    let end = client
        .events
        .iter()
        .find(|event| event["type"] == "tool_execution_end")
        .unwrap();
    assert_eq!(end["isError"], false);
    assert!(
        end["result"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| part["text"] == "Finished."),
        "{}",
        end["result"]
    );
}

#[test]
fn devin_permission_requests_never_answer_plan_bypass() {
    let permission_record = |options: Value| {
        json!({"id":9,"params":{"sessionId":"root-session","title":"Finish the plan?",
            "toolCall":{"toolCallId":"plan-exit","title":"Plan","kind":"plan"},
            "options":options}})
    };
    let options = json!([
        {"optionId":"plan_bypass","kind":"allow_once","name":"Bypass permissions"},
        {"optionId":"plan_accept_edits","kind":"allow_once","name":"Accept edits"},
        {"optionId":"plan_reject","kind":"reject_once","name":"Reject"}]);
    // Full access auto-answers the first answerable allow option, not the bypass.
    let mut client = devin_client();
    client.start.link.as_mut().unwrap().access_mode = "full-access".into();
    client
        .permission(&permission_record(options.clone()))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["result"]["outcome"]["optionId"],
        "plan_accept_edits"
    );
    // A supervised accept maps to the filtered allow option too.
    let mut client = devin_client();
    client.permission(&permission_record(options)).unwrap();
    let key = client.events.last().unwrap()["id"].clone();
    client
        .command(json!({"type":"extension_ui_response","id":key,"decision":"accept"}))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["result"]["outcome"]["optionId"],
        "plan_accept_edits"
    );
    // A request whose only allow option is the bypass cancels safely.
    let mut client = devin_client();
    client.start.link.as_mut().unwrap().access_mode = "full-access".into();
    client
        .permission(&permission_record(json!([
            {"optionId":"plan_bypass","kind":"allow_once","name":"Bypass"},
            {"optionId":"plan_reject","kind":"reject_once","name":"Reject"}])))
        .unwrap();
    assert_eq!(
        client.wire.last().unwrap()["result"]["outcome"]["outcome"],
        "cancelled"
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

#[test]
fn devin_embedded_terminal_content_resolves_to_output_text() {
    let mut client = devin_client();
    client.running = true;
    let terminal = client
        .terminals
        .create(
            &json!({"sessionId":"root-session","command":"echo embedded-out"}),
            &client.start.workspace,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while client.terminals.exit_status_of(&terminal).is_none() {
        assert!(Instant::now() < deadline, "embedded terminal never exited");
        std::thread::sleep(Duration::from_millis(10));
    }
    client
        .incoming(json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"root-session","update":{"sessionUpdate":"tool_call","toolCallId":"shell",
            "title":"Tool","kind":"execute","status":"completed",
            "content":[{"type":"terminal","terminalId":terminal}],
            "_meta":{"cognition.ai/inferenceToolName":"bash"}}}}))
        .unwrap();
    let start = client
        .events
        .iter()
        .find(|event| event["type"] == "tool_execution_start")
        .unwrap();
    // The _meta inference name replaced the generic "Tool" title.
    assert_eq!(start["toolName"], "bash");
    let end = client
        .events
        .iter()
        .find(|event| event["type"] == "tool_execution_end")
        .unwrap();
    let texts: Vec<&str> = end["result"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect();
    assert!(
        texts.iter().any(|text| text.contains("embedded-out")),
        "{texts:?}"
    );
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
fn interactive_acp_requires_an_mcp_bridge() {
    let (agent, _log) = fixture(&["--antigravity"]);
    let mut start = spec(SessionSource::New);
    start.link = None;
    let Err(error) = super::start(agent, start, Arc::new(|| {})) else {
        panic!("Interactive ACP must not start without an MCP bridge");
    };
    assert!(error.contains("ACP requires Shina's MCP bridge"), "{error}");
}

/// KSG AntigravityAcpSupport: initialize, eager authenticate, session/new with
/// model+thinking_level config options, combined `family-level` selection.
#[test]
fn antigravity_fake_agent_authenticate_config_options_and_combined_model() {
    let (mut agent, log) = fixture(&["--antigravity"]);
    agent.id = "antigravity".into();
    let dir = crate::store::directory().unwrap();
    std::fs::write(
        dir.join("harnesses.json"),
        json!([{"id":agent.id,"name":agent.name,"command":agent.command,
            "args":agent.args,"env":agent.env}])
        .to_string(),
    )
    .unwrap();
    let mut thread = Thread::new(32);
    thread.harness = "antigravity".into();
    thread.host = spec(SessionSource::New).link;
    thread.native.access = crate::model::AccessMode::Supervised;
    thread.connect_with_instructions(
        dir.clone(),
        Arc::new(|| {}),
        Some("Captain fixture instructions".into()),
    );
    pump(&mut thread, |t| {
        t.native.commands_loaded && !t.harness_state["modes"].is_null()
    });
    let wire = records(&log);
    let methods: Vec<&str> = wire.iter().filter_map(|r| r["method"].as_str()).collect();
    let position = |m: &str| methods.iter().position(|x| *x == m).unwrap();
    assert!(position("initialize") < position("authenticate"));
    assert!(position("authenticate") < position("session/new"));
    let session_new = wire.iter().find(|r| r["method"] == "session/new").unwrap();
    assert!(session_new["params"]["_meta"]["antigravity"]["systemPrompt"]["append"].is_string());
    // The session builds the combined 1.3-style ids T3 selects.
    let ids: Vec<&str> = thread.models.iter().map(|m| m.id.as_str()).collect();
    for id in [
        "gemini-3.8-flash-high",
        "gemini-3.8-flash-medium",
        "gemini-3.8-flash-low",
        "gemini-4-carbon",
    ] {
        assert!(ids.contains(&id), "{id} not in {ids:?}");
    }
    // `gemini-3.8-flash-high` is the session's current model; picking a different
    // level still sends family then level, like KSG's applyAntigravityAcpModelSelection.
    crate::harness::send(
        thread.pi.as_ref().unwrap(),
        json!({"type":"set_model","id":"pick","modelId":"gemini-3.8-flash-medium"}),
    )
    .unwrap();
    pump(&mut thread, |t| t.model_id == "gemini-3.8-flash-medium");
    let sets: Vec<Value> = records(&log)
        .into_iter()
        .filter(|r| r["method"] == "session/set_config_option")
        .collect();
    assert_eq!(sets.len(), 2);
    assert_eq!(
        sets[0]["params"],
        json!({"sessionId":"fixture-session","configId":"model","value":"gemini-3.8-flash"})
    );
    assert_eq!(
        sets[1]["params"],
        json!({"sessionId":"fixture-session","configId":"thinking_level","value":"medium"})
    );

    // Cross-family combined pick: family config first, thinking level second.
    crate::harness::send(
        thread.pi.as_ref().unwrap(),
        json!({"type":"set_model","id":"pick2","modelId":"gemini-4-carbon-high"}),
    )
    .unwrap();
    pump(&mut thread, |t| t.model_id == "gemini-4-carbon-high");
    let sets: Vec<Value> = records(&log)
        .into_iter()
        .filter(|r| r["method"] == "session/set_config_option")
        .collect();
    assert_eq!(sets.len(), 4);
    assert_eq!(
        sets[2]["params"],
        json!({"sessionId":"fixture-session","configId":"model","value":"gemini-4-carbon"})
    );
    assert_eq!(sets[3]["params"]["configId"], "thinking_level");
    assert_eq!(sets[3]["params"]["value"], "high");
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
    // initialize and tools/list ride the same UI-thread queue as tools/call;
    // answer the queued request the way a running State pump would.
    let pumper = server.clone();
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            if let Some(request) = pumper.next_request() {
                let _ = request.reply.send(Ok(json!({
                    "protocolVersion": crate::mcp::PROTOCOL,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "Shina"},
                    "instructions": crate::mcp::INSTRUCTIONS,
                })));
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
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

/// An unflagged probe spawn pops a console window on Windows for console hosts
/// like agy or claude. The fixture reports its own console handle and pid so the
/// test can check both the launch flags and that the probe reaps the agent.
#[cfg(windows)]
#[test]
fn an_initialize_probe_has_no_console_window_and_reaps_the_agent() {
    let (agent, log) = fixture(&["--record-launch"]);
    let capabilities = probe(&agent).unwrap();
    assert_eq!(capabilities["mcpCapabilities"]["stdio"], true);
    let wire = records(&log);
    let launch = wire
        .iter()
        .find_map(|record| record.get("fixtureLaunch"))
        .expect("fixture launch record");
    assert!(
        launch.get("consoleWindow").is_some_and(Value::is_null),
        "the probe agent inherited or created a console: {launch}"
    );
    assert_eq!(
        wire.iter()
            .filter_map(|record| record["method"].as_str())
            .collect::<Vec<_>>(),
        ["initialize"],
        "the probe must send initialize only: {wire:?}"
    );
    let pid = launch["pid"].as_u64().expect("fixture pid") as u32;
    let deadline = Instant::now() + Duration::from_secs(5);
    while crate::process::is_alive(pid) {
        assert!(
            Instant::now() < deadline,
            "the probe agent outlived the probe"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn acp_fresh_session_issues_session_new_and_not_resume_or_load() {
    let mut client = Client::new(spec(SessionSource::New));
    client.initialize();
    let init_id = client.wire[0]["id"].clone();
    client
        .incoming(json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "result": {
                "protocolVersion": 1,
                "agentCapabilities": {
                    "loadSession": true,
                    "sessionCapabilities": { "resume": {} }
                }
            }
        }))
        .unwrap();
    let methods: Vec<&str> = client
        .wire
        .iter()
        .filter_map(|msg| msg["method"].as_str())
        .collect();
    assert!(methods.contains(&"session/new"), "{methods:?}");
    assert!(!methods.contains(&"session/load"), "{methods:?}");
    assert!(!methods.contains(&"session/resume"), "{methods:?}");
}
