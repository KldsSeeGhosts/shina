use super::*;

#[test]
fn simultaneous_completions_are_delivered_exactly_once_to_idle_or_busy_parents() {
    for status in [
        Status::Ready,
        Status::Completed,
        Status::Running,
        Status::Waiting,
    ] {
        let mut state = State::new(".".into(), false);
        let (commands, _parent_events) = fake_pi(&mut state.threads[0], status);
        state.threads[0].draft.replace_text("Unsent user draft");
        let (first, _first_commands, first_events) = child(&mut state);
        let (second, _second_commands, second_events) = child(&mut state);
        for events in [&first_events, &second_events] {
            events
                .send(Event::Record(
                    json!({"type":"message_end","message":{"role":"assistant","content":"PONG"}}),
                ))
                .unwrap();
            events
                .send(Event::Record(
                    json!({"type":"agent_settled","aborted":false}),
                ))
                .unwrap();
        }
        poll(&mut state, 0.0, &mut Tx::default());
        assert_eq!(state.threads[0].pending_injections.len(), 2);
        let first_prompt = commands.try_recv().unwrap();
        assert_eq!(first_prompt["type"], "prompt");
        assert_eq!(first_prompt["streamingBehavior"], "followUp");
        assert!(
            first_prompt["message"]
                .as_str()
                .unwrap()
                .contains("task #2 finished")
        );
        assert!(first_prompt["message"].as_str().unwrap().contains("PONG"));
        assert!(
            commands.try_recv().is_err(),
            "await acceptance before the next injection"
        );
        acknowledge(&mut state.threads[0], &first_prompt);
        poll(&mut state, 0.0, &mut Tx::default());
        let second_prompt = commands.try_iter().find(|c| c["type"] == "prompt").unwrap();
        assert!(
            second_prompt["message"]
                .as_str()
                .unwrap()
                .contains("task #3 finished")
        );
        acknowledge(&mut state.threads[0], &second_prompt);
        for index in [first, second] {
            state.threads[index].apply(json!({"type":"agent_settled","aborted":false}));
        }
        poll(&mut state, 0.0, &mut Tx::default());
        assert!(state.threads[0].pending_injections.is_empty());
        assert!(commands.try_iter().all(|c| c["type"] != "prompt"));
        assert_eq!(state.threads[0].draft.text, "Unsent user draft");
        assert_eq!(
            state.threads[0]
                .messages
                .iter()
                .filter(|m| m.role == "Shina")
                .count(),
            2
        );
    }
}

#[test]
fn explicit_delegation_selection_applies_before_the_first_prompt_and_is_reported() {
    let mut state = State::new(".".into(), false);
    state.demo = true;
    state.current_mut().harness = "pi".into();
    let fast = model_with_levels("fast", &["off", "low"]);
    let deep = model_with_levels(
        "deep",
        &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
    );
    state.catalog.models = vec![fast.clone(), deep.clone()];
    state.catalog.default_model = Some(fast);
    state.catalog.default_thinking = "low".into();
    let result = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Inspect the test fixture",
                "target":{"provider":"cpa","model":"deep","options":{"reasoning":"max"}}
            }),
        )
        .unwrap();
    assert_eq!(result["selection"]["model"], "deep");
    assert_eq!(result["selection"]["options"]["reasoning"], "max");
    let index = state.threads.len() - 1;
    let child_id = state.threads[index].id;
    {
        let child = &state.threads[index];
        assert_eq!(child.selected_model, Some(("cpa".into(), "deep".into())));
        assert_eq!(child.selected_thinking.as_deref(), Some("max"));
        assert_eq!(child.model_id, "deep");
        assert_eq!(child.thinking, "max");
        assert_eq!(
            child.thinking_levels.last().map(String::as_str),
            Some("max")
        );
        assert_eq!(child.models.len(), 2);
    }
    let status = state
        .mcp_call(1, "task_status", &json!({"thread_id":child_id}))
        .unwrap();
    assert_eq!(status["selection"]["model"], "deep");
    assert_eq!(status["selection"]["options"]["reasoning"], "max");
    let read = state
        .mcp_call(1, "read_thread", &json!({"threadId":child_id}))
        .unwrap();
    assert_eq!(read["thread"]["selection"]["model"], "deep");
    let list = state
        .mcp_call(1, "list_threads", &json!({"allProjects":true}))
        .unwrap();
    let listed = list["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["threadId"] == json!(child_id))
        .unwrap();
    assert_eq!(listed["selection"]["model"], "deep");
    assert_eq!(listed["selection"]["options"]["reasoning"], "max");

    // Mock a fresh Pi handshake. The child must acknowledge model and reasoning before
    // its injected brief becomes a prompt.
    let (commands, events) = preparing_pi(&mut state.threads[index]);
    for record in [
        json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
            "isStreaming":false,"model":{"provider":"cpa","id":"fast"},"thinkingLevel":"low"
        }}),
        json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}),
    ] {
        events.send(Event::Record(record)).unwrap();
    }
    poll(&mut state, 0.0, &mut Tx::default());
    let model = commands.try_recv().unwrap();
    assert_eq!(model["type"], "set_model");
    assert_eq!(model["modelId"], "deep");
    assert!(
        commands.try_recv().is_err(),
        "the prompt must wait for the model acknowledgement"
    );
    state.threads[index].apply(json!({"type":"response","command":"set_model","success":true,"data":{"provider":"cpa","id":"deep"}}));
    let thinking = loop {
        let command = commands.try_recv().expect("reasoning command");
        assert_ne!(
            command["type"], "prompt",
            "the prompt must wait for the reasoning choice"
        );
        if command["type"] == "set_thinking_level" {
            break command;
        }
    };
    assert_eq!(thinking["level"], "max");
    assert!(
        commands.try_recv().is_err(),
        "the prompt must wait for the reasoning acknowledgement"
    );
    state.threads[index]
        .apply(json!({"type":"response","command":"set_thinking_level","success":true}));
    let verification_commands: Vec<_> = commands.try_iter().collect();
    assert!(
        verification_commands
            .iter()
            .all(|command| command["type"] != "prompt"),
        "the prompt must wait for startup to finish"
    );
    let verification = verification_commands
        .iter()
        .rev()
        .find(|command| command["type"] == "get_state")
        .expect("Pi must confirm its actual selection");
    state.poll_tasks(Instant::now());
    assert!(commands.try_recv().is_err());
    state.threads[index].apply(json!({
        "type":"response","id":verification["id"],"command":"get_state","success":true,
        "data":{"model":{"provider":"cpa","id":"deep"},"thinkingLevel":"max"}
    }));
    state.poll_tasks(Instant::now());
    let prompt = commands.try_recv().unwrap();
    assert_eq!(prompt["type"], "prompt");
    assert!(
        prompt["message"]
            .as_str()
            .unwrap()
            .contains("Inspect the test fixture")
    );
    assert_eq!(state.threads[index].model_id, "deep");
    assert_eq!(state.threads[index].thinking, "max");
}

#[test]
fn cursor_startup_waits_for_saved_options_before_driving_the_prompt() {
    let mut thread = Thread::new(2);
    thread.harness = "cursor".into();
    let (commands, _events) = preparing_pi(&mut thread);
    thread.state_loaded = true;
    thread.history_loaded = true;
    thread.set_option(
        "contextWindow".into(),
        crate::model_selection::OptionValue::Str("2m".into()),
    );
    thread.drive_session();
    let choose = commands.try_recv().unwrap();
    assert_eq!(choose["type"], "set_option");
    assert_eq!(choose["optionId"], "contextWindow");
    assert_eq!(choose["value"], "2m");
    assert!(thread.initializing);
    assert!(thread.configuring);
    assert!(commands.try_recv().is_err());
    thread.apply(json!({"type":"response","id":choose["id"],
        "command":"set_option","success":true}));
    assert!(!thread.initializing);
    assert!(!thread.configuring);
}

#[test]
fn invalid_delegation_targets_are_rejected_without_creating_children() {
    let mut state = State::new(".".into(), false);
    state.demo = true;
    state.threads[0].harness = "pi".into();
    let fast = model_with_levels("fast", &["off", "low"]);
    let deep = model_with_levels(
        "deep",
        &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
    );
    state.catalog.models = vec![fast, deep];
    state.catalog.default_model = Some(state.catalog.models[0].clone());
    let before = state.threads.len();
    let cases = [
        ("null target", Value::Null),
        ("wrong target type", json!("deep")),
        ("missing model", json!({"provider":"cpa"})),
        ("empty model", json!({"provider":"cpa","model":"  "})),
        ("wrong model type", json!({"provider":"cpa","model":7})),
        ("wrong provider type", json!({"provider":7,"model":"fast"})),
        (
            "direct provider",
            json!({"provider":"google","model":"fast"}),
        ),
        (
            "unknown target setting",
            json!({"provider":"cpa","model":"fast","role":"review"}),
        ),
        (
            "wrong options type",
            json!({"provider":"cpa","model":"fast","options":[]}),
        ),
        (
            "unknown option",
            json!({"provider":"cpa","model":"fast","options":{"temperature":"hot"}}),
        ),
        (
            "wrong reasoning type",
            json!({"provider":"cpa","model":"fast","options":{"reasoning":5}}),
        ),
        (
            "empty reasoning",
            json!({"provider":"cpa","model":"fast","options":{"reasoning":" "}}),
        ),
        (
            "conflicting aliases",
            json!({"provider":"cpa","model":"fast","options":{"reasoning":"low","thinking":"high"}}),
        ),
        ("unknown model", json!({"provider":"cpa","model":"missing"})),
    ];
    for (label, target) in cases {
        let error = state
            .mcp_call(
                1,
                "delegate_task",
                &json!({"prompt":"No child","target":target}),
            )
            .unwrap_err();
        assert!(!error.is_empty(), "{label}");
        assert_eq!(
            state.threads.len(),
            before,
            "{label} must not create a child: {error}"
        );
    }
    let error = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"No child",
                "target":{"provider":"cpa","model":"fast","options":{"reasoning":"max"}}
            }),
        )
        .unwrap_err();
    assert!(error.contains("Supported levels: off, low"), "{error}");
    assert!(
        error.contains("will not substitute or clamp"),
        "an explicit level must never be clamped: {error}"
    );
    assert_eq!(state.threads.len(), before);
    state.threads[0].machine = "remote".into();
    let error = state
        .mcp_call(1, "delegate_task", &json!({"prompt":"No child"}))
        .unwrap_err();
    assert!(
        error.contains("only on the local machine"),
        "unsupported hosts must fail before a child starts: {error}"
    );
    assert_eq!(state.threads.len(), before);
}

#[test]
fn delegation_inherits_pinned_selection_and_overrides_only_given_fields() {
    let mut state = State::new(".".into(), false);
    state.demo = true;
    state.threads[0].harness = "pi".into();
    state.threads[0].model_provider = "cpa".into();
    state.threads[0].model_id = "fast".into();
    state.threads[0].thinking = "low".into();
    state.threads[0].selected_model = Some(("cpa".into(), "deep".into()));
    state.threads[0].selected_thinking = Some("max".into());
    let before = state.threads.len();
    let error = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt":"No child","target":{"model":"deep"}}),
        )
        .unwrap_err();
    assert!(error.contains("discovery is not ready"), "{error}");
    assert_eq!(state.threads.len(), before);

    let result = state
        .mcp_call(1, "delegate_task", &json!({"prompt":"Inherit my choice"}))
        .unwrap();
    let child = state.threads.last().unwrap();
    assert_eq!(child.selected_model, Some(("cpa".into(), "deep".into())));
    assert_eq!(child.selected_thinking.as_deref(), Some("max"));
    assert_eq!(result["selection"]["model"], "deep");
    assert_eq!(result["selection"]["options"]["reasoning"], "max");

    let fast = model_with_levels("fast", &["off", "low"]);
    let deep = model_with_levels(
        "deep",
        &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
    );
    state.catalog.models = vec![fast, deep];
    state.catalog.default_model = Some(state.catalog.models[0].clone());
    state.catalog.default_thinking = "low".into();
    let error = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({"prompt":"No child","target":{"model":"fast"}}),
        )
        .unwrap_err();
    assert!(
        error.contains("Reasoning level max is not supported by CPA model fast"),
        "{error}"
    );
    assert_eq!(state.threads.len(), before + 1);

    let result = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Use the alias",
                "target":{"provider":"cpa","model":"fast","options":{"thinking":"low"}}
            }),
        )
        .unwrap();
    assert_eq!(result["selection"]["model"], "fast");
    assert_eq!(result["selection"]["options"]["reasoning"], "low");
    let child = state.threads.last().unwrap();
    assert_eq!(child.selected_model, Some(("cpa".into(), "fast".into())));
    assert_eq!(child.selected_thinking.as_deref(), Some("low"));
}

#[test]
fn antigravity_delegation_accepts_a_model_id_with_a_thinking_level() {
    let mut state = State::new(".".into(), false);
    state.demo = true;
    state.threads[0].harness = "antigravity".into();
    state.threads[0].model_provider = "antigravity".into();
    state.threads[0].model_id = "gemini-3.8-flash".into();
    state.provider_availability.last = Some(crate::harness::Availability {
        claude: false,
        antigravity: true,
        cursor: false,
    });
    state.test_connect_pi = Some(Box::new(|_, _| None));
    // KSG `antigravitySuffixedModel`: Antigravity 1.3 appends the thinking
    // level to the model id; the delegation maps it to the family plus the
    // level as its reasoning option.
    let mut flash = model_with_levels("gemini-3.8-flash", &["off", "low", "medium", "high"]);
    flash.provider = "antigravity".into();
    state.antigravity_catalog.models = vec![flash];
    let result = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Route the level",
                "target":{"model":"gemini-3.8-flash-high"}
            }),
        )
        .unwrap();
    assert_eq!(result["selection"]["model"], "gemini-3.8-flash");
    assert_eq!(result["selection"]["options"]["reasoning"], "high");
    let child = state.threads.last().unwrap();
    assert_eq!(
        child.selected_model,
        Some(("antigravity".into(), "gemini-3.8-flash".into()))
    );
    assert_eq!(child.selected_thinking.as_deref(), Some("high"));
    // An explicit reasoning option still wins over the suffix.
    let result = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Override the suffix",
                "target":{"model":"gemini-3.8-flash-high","options":{"reasoning":"low"}}
            }),
        )
        .unwrap();
    assert_eq!(result["selection"]["model"], "gemini-3.8-flash");
    assert_eq!(result["selection"]["options"]["reasoning"], "low");
    // A suffix that is not an advertised level still fails the catalog check.
    let error = state
        .mcp_call(
            1,
            "delegate_task",
            &json!({
                "prompt":"Bad level",
                "target":{"model":"gemini-3.8-flash-ultra"}
            }),
        )
        .unwrap_err();
    assert!(error.contains("is not in the current catalog"), "{error}");
}

#[test]
fn orchestrator_capabilities_report_catalog_defaults_caller_choice_and_limits() {
    let mut state = State::new(".".into(), false);
    let fast = model_with_levels("fast", &["off", "low"]);
    let deep = model_with_levels(
        "deep",
        &["off", "minimal", "low", "medium", "high", "xhigh", "max"],
    );
    state.catalog.models = vec![fast.clone(), deep.clone()];
    state.catalog.default_model = Some(fast);
    state.catalog.default_thinking = "low".into();
    state.threads[0].harness = "pi".into();
    state.threads[0].selected_model = Some(("cpa".into(), "deep".into()));
    state.threads[0].selected_thinking = Some("max".into());
    let capabilities = state
        .mcp_call(1, "orchestrator_capabilities", &json!({}))
        .unwrap();
    assert_eq!(capabilities["ready"], true);
    assert_eq!(capabilities["discovery"]["state"], "ready");
    assert_eq!(capabilities["discovery"]["source"], "catalog");
    assert_eq!(capabilities["discovery"]["error"], Value::Null);
    let models = capabilities["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    let deep_entry = models.iter().find(|model| model["id"] == "deep").unwrap();
    assert_eq!(
        deep_entry["reasoning_levels"],
        json!(["off", "minimal", "low", "medium", "high", "xhigh", "max"])
    );
    assert_eq!(capabilities["default_selection"]["model"], "fast");
    assert_eq!(
        capabilities["default_selection"]["options"]["reasoning"],
        "low"
    );
    assert_eq!(capabilities["caller_selection"]["model"], "deep");
    assert_eq!(
        capabilities["caller_selection"]["options"]["reasoning"],
        "max"
    );
    assert_eq!(capabilities["inherit_selection"]["model"], "deep");
    assert_eq!(capabilities["runtime"]["harness"], "pi");
    assert_eq!(
        capabilities["runtime"]["providers"],
        json!(["cpa", "codex", "claude", "cursor"])
    );
    assert_eq!(capabilities["runtime"]["hosts"], json!(["local"]));
    assert_eq!(capabilities["runtime"]["machine"], "local");
    assert_eq!(capabilities["runtime"]["host_supported"], true);
    assert_eq!(capabilities["constraints"]["direct_providers"], false);
    assert_eq!(capabilities["constraints"]["remote_hosts"], true);
}

#[test]
fn only_a_childs_parent_can_cancel_or_send_and_cancellation_finishes_waits() {
    let mut state = State::new(".".into(), false);
    let (_commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
    let (index, commands, events) = child(&mut state);
    let mut task = Task::new(&state.threads[index], "A bounded task", None);
    task.state = TaskState::Running;
    let attempt = task.attempt_id.clone();
    state.tasks.push(task);
    let mut stranger = Thread::new(3);
    stranger.status = Status::Ready;
    state.threads.push(stranger);
    for caller in [2, 3] {
        for tool in ["task_cancel", "task_send"] {
            let error = state
                .mcp_call(
                    caller,
                    tool,
                    &json!({"thread_id":2,"message":"Correction","client_request_id":"stranger"}),
                )
                .unwrap_err();
            assert!(error.contains("Only the parent"));
        }
    }
    assert!(
        state
            .mcp_call(
                1,
                "task_cancel",
                &json!({"thread_id":3,"client_request_id":"cancel-stranger","attempt_id":attempt})
            )
            .is_err()
    );
    assert!(commands.try_recv().is_err());
    state
        .mcp_call(
            1,
            "task_send",
            &json!({"thread_id":2,"message":"Correction","client_request_id":"send-correction"}),
        )
        .unwrap();
    state.poll_tasks(Instant::now());
    let prompt = commands.try_recv().unwrap();
    assert_eq!(prompt["streamingBehavior"], "followUp");
    assert!(prompt["message"].as_str().unwrap().contains("Correction"));
    acknowledge(&mut state.threads[index], &prompt);
    assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
    let (reply, answer) = mpsc::channel();
    state.mcp_request(crate::mcp::Request {
        thread: 1,
        tool: "task_status".into(),
        args: json!({"thread_id":2,"wait_seconds":600}),
        reply,
    });
    state
        .mcp_call(
            1,
            "task_cancel",
            &json!({"thread_id":2,"client_request_id":"cancel-1","attempt_id":attempt}),
        )
        .unwrap();
    assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
    assert_eq!(commands.try_recv().unwrap()["type"], "abort");
    events
        .send(Event::Record(
            json!({"type":"agent_settled","aborted":true}),
        ))
        .unwrap();
    poll(&mut state, 0.0, &mut Tx::default());
    let result = answer.try_recv().unwrap().unwrap();
    assert_eq!(result["status"], "cancelled");
    assert_eq!(result["finished"], true);
    assert_eq!(state.threads[index].status, Status::Ready);
    assert!(
        state
            .mcp_call(2, "delegate_task", &json!({"prompt":"Nested"}))
            .unwrap_err()
            .contains("cannot delegate")
    );
    state
        .mcp_call(
            1,
            "task_send",
            &json!({"thread_id":2,"message":"Retry","client_request_id":"send-retry"}),
        )
        .unwrap();
    state.poll_tasks(Instant::now());
    assert_eq!(state.threads[index].status, Status::Running);
    assert_eq!(
        state
            .mcp_call(1, "task_status", &json!({"thread_id":2}))
            .unwrap()["finished"],
        false
    );
    assert!(commands.try_iter().any(|c| c["type"] == "prompt"));
}

#[test]
fn final_message_replaces_deltas_without_duplicating_text() {
    let mut thread = Thread::new(1);
    thread.apply(json!({"type":"message_start","message":{"role":"assistant"}}));
    thread.apply(json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Part"}}));
    thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Complete"}]}}));
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].text, "Complete");
}
#[test]
fn a_confirmation_waits_for_the_user_and_is_never_auto_approved() {
    let mut thread = Thread::new(1);
    thread.apply(json!({"type":"extension_ui_request","id":"permission","method":"confirm","title":"Run command?"}));
    assert_eq!(thread.status, Status::Waiting);
    assert_eq!(thread.request.as_ref().unwrap()["id"], "permission");
    thread.apply(json!({"type":"agent_settled","aborted":false}));
    assert!(thread.request.is_none());
}
#[test]
fn reconnect_restores_tool_calls_and_results_without_duplicates() {
    let mut thread = Thread::new(1);
    let history = json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
        {"role":"user","content":"Check the file"},
        {"role":"assistant","content":[{"type":"toolCall","id":"read-1","name":"read","arguments":{"path":"example.txt"}}]},
        {"role":"toolResult","toolCallId":"read-1","toolName":"read","content":[{"type":"text","text":"saved output"}],"isError":false},
        {"role":"assistant","content":[{"type":"text","text":"Done"}]}
    ]}});
    thread.apply(history.clone());
    thread.apply(history);
    assert_eq!(thread.messages.len(), 3);
    assert_eq!(thread.tools.len(), 1);
    assert_eq!(thread.tools[0].detail, "example.txt");
    assert_eq!(thread.tools[0].output, "saved output");
    assert!(thread.tools[0].done && !thread.tools[0].failed);
}
#[test]
fn restored_tools_preserve_block_order_and_tool_only_turns() {
    let mut thread = Thread::new(1);
    let history = json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[
        {"role":"user","content":"Read the files"},
        {"role":"assistant","content":[
            {"type":"text","text":"First"},
            {"type":"toolCall","id":"a","name":"read","arguments":{"path":"a.txt"}},
            {"type":"toolCall","id":"b","name":"read","arguments":{"path":"b.txt"}},
            {"type":"text","text":"Then"},
            {"type":"toolCall","id":"c","name":"bash","arguments":{"command":"pwd"}},
            {"type":"text","text":"Done"}
        ]},
        {"role":"toolResult","toolCallId":"b","content":[{"type":"text","text":"b contents"}],"isError":false,"durationMs":40},
        {"role":"toolResult","toolCallId":"a","content":[{"type":"text","text":"a contents"}],"isError":false},
        {"role":"assistant","content":[{"type":"toolCall","id":"d","name":"read","arguments":{"path":"d.txt"}}]},
        {"role":"assistant","content":[{"type":"text","text":"Final reply"}]}
    ]}});
    thread.apply(history.clone());
    thread.tools[0].expanded = true;
    thread.apply(history);
    assert_eq!(thread.messages.len(), 4);
    assert_eq!(thread.messages[1].text, "First\nThen\nDone");
    let anchors: Vec<_> = thread.tools.iter().map(|tool| tool.anchor).collect();
    assert_eq!(
        anchors,
        vec![
            ToolAnchor {
                message: 1,
                offset: 5
            },
            ToolAnchor {
                message: 1,
                offset: 5
            },
            ToolAnchor {
                message: 1,
                offset: 10
            },
            ToolAnchor {
                message: 2,
                offset: 0
            },
        ]
    );
    assert_eq!(thread.tools[1].output, "b contents");
    assert_eq!(thread.tools[1].duration_ms, Some(40));
    assert_eq!(
        thread.tools[0].duration_ms, None,
        "restored timings are not invented"
    );
    assert!(thread.tools[2].failed, "missing saved results stay visible");
    assert!(
        !thread.tools[0].expanded,
        "restoring rebuilds transient disclosure state"
    );
}

#[test]
fn thinking_deltas_use_content_indices_and_completed_blocks_replace_them() {
    let mut thread = Thread::new(1);
    thread.apply(json!({"type":"agent_start"}));
    thread.apply(json!({"type":"message_start","message":{"role":"assistant"}}));
    for event in [
        json!({"type":"thinking_start","contentIndex":0}),
        json!({"type":"thinking_delta","contentIndex":0,"delta":"Partial reasoning"}),
        json!({"type":"thinking_end","contentIndex":0,"content":"Complete reasoning"}),
        json!({"type":"text_delta","contentIndex":1,"delta":"I'll read the file."}),
        json!({"type":"thinking_start","contentIndex":2}),
        json!({"type":"thinking_delta","contentIndex":2,"delta":"Second thought"}),
    ] {
        thread.apply(json!({"type":"message_update","assistantMessageEvent":event}));
    }
    assert_eq!(thread.thoughts.len(), 2);
    assert_eq!(thread.thoughts[0].text, "Complete reasoning");
    assert!(thread.thoughts[0].done);
    assert!(thread.thoughts[0].duration_ms.is_some());
    assert!(!thread.thoughts[1].done);
    thread.apply(
        json!({"type":"message_end","message":{"role":"assistant","content":[
            {"type":"thinking","thinking":"Authoritative reasoning","thinkingSignature":"opaque"},
            {"type":"text","text":"I'll read the file."},
            {"type":"thinking","thinking":"Complete second thought"},
            {"type":"toolCall","id":"read","name":"read","arguments":{"path":"file.txt"}}
        ]}}),
    );
    thread.apply(json!({"type":"tool_execution_start","toolCallId":"read","toolName":"read","args":{"path":"file.txt"}}));
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.thoughts.len(), 2);
    assert_eq!(thread.thoughts[0].text, "Authoritative reasoning");
    assert_eq!(thread.thoughts[1].text, "Complete second thought");
    assert!(thread.thoughts[1].done);
    assert_eq!(thread.thoughts[1].anchor, thread.tools[0].anchor);
    assert!(thread.thoughts[1].block < thread.tools[0].block);
    assert_eq!(thread.tools.len(), 1);
}

#[test]
fn rejected_prompt_returns_to_the_draft_and_does_not_claim_it_was_sent() {
    let mut thread = Thread::new(1);
    thread.messages.push(Message {
        id: uuid::Uuid::new_v4().to_string(),
        role: "You",
        text: "Try this".into(),
        images: Vec::new(),
        context: None,
    });
    thread.pending_prompt = Some(("1-1".into(), Prompt::new("Try this".into(), vec![]), 0));
    thread.status = Status::Running;
    thread.apply(json!({"type":"response","id":"1-1","command":"prompt","success":false,"error":"No model configured"}));
    assert_eq!(thread.status, Status::Failed);
    assert_eq!(thread.draft.text, "Try this");
    assert!(thread.messages.is_empty());
    assert!(thread.queued_prompt.is_none());
}
#[test]
fn aborted_message_is_not_a_provider_failure() {
    let mut thread = Thread::new(1);
    thread.status = Status::Running;
    thread.apply(json!({"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"aborted","errorMessage":"Request was aborted"}}));
    thread.apply(json!({"type":"agent_settled","aborted":true}));
    assert_eq!(thread.status, Status::Ready);
    assert!(thread.error.is_none());
    thread.apply(json!({"type":"response","command":"abort","success":true}));
    assert_eq!(thread.status, Status::Ready);
}

#[test]
fn paste_prompt_limit_fallback_and_removed_names_follow_ksg() {
    let mut state = State::new(".".into(), false);
    state.current_mut().draft.replace_text(&"x".repeat(119_999));
    assert!(state.fold_paste("😀"));
    assert_eq!(state.current().draft_images[0].name, "pasted-text.txt");
    assert_eq!(
        state.current().draft_images[0].source,
        Some(attachments::AttachmentSource::PastedText)
    );
    state.current_mut().draft_images.clear();
    state.current_mut().draft.replace_text("");
    assert!(state.fold_paste(&"x".repeat(crate::composer_paste::THRESHOLD)));
    assert_eq!(state.current().draft_images[0].name, "pasted-text-2.txt");
    let attachment = state.current().draft_images[0].clone();
    state.current_mut().draft_images = vec![attachment; 100];
    state.current_mut().draft.replace_text("kept");
    assert!(!state.fold_paste(&"x".repeat(crate::composer_paste::THRESHOLD)));
    state.current_mut().draft.replace_text(&"x".repeat(120_000));
    let before = state.current().draft.text.clone();
    assert!(state.fold_paste("a"));
    assert_eq!(state.current().draft.text, before);
    assert!(
        state
            .current()
            .attachment_error
            .as_deref()
            .unwrap()
            .starts_with("Pasted text is too large for this message")
    );
}

#[test]
fn rich_draft_survives_ui_restore_and_sends_exact_markdown() {
    let mut state = State::new(".".into(), false);
    let edit = &mut state.current_mut().draft;
    edit.replace_text("bold\none\ntwo\nquote\ncode");
    edit.set_selection(0, 4);
    edit.toggle_mark(rune::Mark::Bold);
    edit.set_blocks(vec![
        rune::Block::Paragraph,
        rune::Block::Numbered(0),
        rune::Block::Numbered(0),
        rune::Block::Quote,
        rune::Block::Code,
    ]);
    let ui = state.ui_snapshot();
    let mut restored = State::new(".".into(), false);
    restored.restore_ui(&ui).unwrap();
    let expected = "**bold**\n1. one\n2. two\n> quote\n```\ncode\n```";
    assert_eq!(
        crate::composer_rich::markdown(&restored.current().draft),
        expected
    );
    assert!(restored.current().draft.is_rich());
    restored.current_mut().send();
    assert_eq!(restored.current().messages[0].text, expected);
    assert!(restored.current().draft.is_rich());
    let mut legacy = ui;
    legacy["drafts"][state.current().id.to_string()] = json!("plain old draft");
    state.restore_ui(&legacy).unwrap();
    assert!(state.current().draft.is_rich());
    assert_eq!(state.current().draft.text, "plain old draft");
}

#[test]
fn failed_send_restores_rich_document_pills_and_later_rich_edits() {
    let mut thread = Thread::new(1);
    let original_image = test_attachment("original.png", 1);
    let later_image = test_attachment("later.png", 2);
    thread.draft_images.push(original_image.clone());
    thread.draft = crate::composer_rich::draft("Bold reference");
    thread.draft.set_selection(0, 4);
    thread.draft.toggle_mark(rune::Mark::Bold);
    let token = crate::composer_tokens::Token::Skill {
        name: "review".into(),
        harness: "pi".into(),
    };
    thread.draft.replace_with_pill(5..14, token.id(), "review");
    let original = crate::composer_rich::markdown(&thread.draft);
    thread.send();
    thread.draft.replace_text("Later");
    thread.draft.set_selection(0, 5);
    thread.draft.toggle_mark(rune::Mark::Italic);
    thread.draft_images.push(later_image.clone());
    thread.disconnect();
    assert_eq!(
        crate::composer_rich::markdown(&thread.draft),
        format!("{original}\n\n*Later*")
    );
    assert_eq!(thread.draft.pills().count(), 1);
    assert_eq!(thread.draft.text, "Bold review\n\nLater");
    assert!(thread.messages.is_empty());
    assert_eq!(thread.draft_images, vec![original_image, later_image]);
}

#[test]
fn send_without_a_connection_is_immediate_and_dispatches_the_submitted_draft_once() {
    let mut state = State::new(".".into(), false);
    state.current_mut().draft.replace_text("Original draft");
    state.current_mut().send();
    assert_eq!(state.current().status, Status::Running);
    assert_eq!(state.current().messages[0].text, "Original draft");
    assert!(state.current().draft.text.is_empty());
    let (pi, commands, events) = Pi::test_connection();
    state.current_mut().pi = Some(pi);
    state.current_mut().initializing = true;
    events.send(Event::Record(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}))).unwrap();
    let mut tx = Tx::default();
    poll(&mut state, 0.0, &mut tx);
    assert!(
        commands.try_recv().is_err(),
        "history alone must not start work"
    );
    state
        .current_mut()
        .draft
        .replace_text("Revised before connection");
    events.send(Event::Record(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"isStreaming":false,"model":{"provider":"cpa","id":"model-a"}}}))).unwrap();
    poll(&mut state, 0.0, &mut tx);
    assert_eq!(commands.try_recv().unwrap()["type"], "get_session_stats");
    let prompt = commands.try_recv().unwrap();
    assert_eq!(prompt["type"], "prompt");
    assert_eq!(prompt["message"], "Original draft");
    assert_eq!(state.current().status, Status::Running);
    assert_eq!(state.current().draft.text, "Revised before connection");
    assert!(state.current().queued_prompt.is_none());
    assert_eq!(state.current().messages.len(), 1);
    poll(&mut state, 0.0, &mut tx);
    assert!(commands.try_recv().is_err(), "submit exactly once");
}
#[test]
fn stopping_cancels_a_dialog_clears_the_queue_and_then_aborts() {
    let (pi, commands, _events) = Pi::test_connection();
    let mut thread = Thread::new(1);
    thread.pi = Some(pi);
    thread.apply(json!({"type":"extension_ui_request","id":"dialog-1","method":"confirm"}));
    thread.stop();
    let cancel = commands.try_recv().unwrap();
    assert_eq!(cancel["type"], "extension_ui_response");
    assert_eq!(cancel["id"], "dialog-1");
    assert_eq!(cancel["cancelled"], true);
    assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
    assert_eq!(commands.try_recv().unwrap()["type"], "abort");
    assert!(thread.request.is_none());
}

#[test]
fn offline_picker_choices_are_applied_before_the_first_prompt() {
    let mut thread = Thread::new(1);
    thread.harness = "pi".into();
    thread.set_model("cpa".into(), "chosen".into());
    thread.set_thinking("high".into());
    assert_eq!(thread.model_id, "chosen");
    assert_eq!(thread.thinking, "high");
    assert!(thread.pi.is_none());
    thread.draft.replace_text("Use my choices");
    thread.send();
    let (pi, commands, _events) = Pi::test_connection();
    thread.pi = Some(pi);
    thread.initializing = true;
    thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
        "model":{"provider":"cpa","id":"default"},"thinkingLevel":"low","sessionFile":"saved.jsonl"
    }}));
    assert_eq!(
        thread.model_id, "chosen",
        "startup must not replace the saved choice"
    );
    thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}));
    let model = commands.try_recv().unwrap();
    assert_eq!(model["type"], "set_model");
    assert_eq!(model["modelId"], "chosen");
    assert!(
        commands.try_recv().is_err(),
        "wait for model acknowledgement"
    );
    thread.apply(json!({"type":"response","command":"set_model","success":true,"data":{"provider":"cpa","id":"chosen"}}));
    assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
    assert_eq!(
        commands.try_recv().unwrap()["type"],
        "get_available_thinking_levels"
    );
    assert_eq!(
        commands.try_recv().unwrap()["type"],
        "get_session_stats",
        "a model change refreshes the live context window"
    );
    let thinking = commands.try_recv().unwrap();
    assert_eq!(thinking["type"], "set_thinking_level");
    assert_eq!(thinking["level"], "high");
    assert!(
        commands.try_recv().is_err(),
        "wait for thinking acknowledgement"
    );
    thread.apply(json!({"type":"response","command":"set_thinking_level","success":true}));
    assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
    assert_eq!(commands.try_recv().unwrap()["type"], "set_session_name");
    assert_eq!(commands.try_recv().unwrap()["type"], "prompt");
    assert!(!thread.initializing);
    assert_eq!(thread.session.as_deref(), Some("saved.jsonl"));
}
