use super::*;

#[test]
fn pi_names_map_to_typed_tools_and_patch_details() {
    use crate::work::ToolKind::*;
    for (name, expected) in [
        ("bash", Command),
        ("edit", FileChange),
        ("grep", FileSearch),
        ("web_search", WebSearch),
        ("read", Read),
        ("ls", List),
        ("subagent", Subagent),
        ("update_plan", Plan),
        ("unknown", Other),
    ] {
        let mut t = Thread::new(1);
        t.apply(json!({"type":"tool_execution_start","toolCallId":"typed","toolName":name,"args":{"path":"a.rs"}}));
        t.apply(json!({"type":"tool_execution_end","toolCallId":"typed","result":{"content":[],
            "details":{"patch":"@@ -1 +1 @@\n-old\n+new\n","results":[{"fileName":"a.rs","line":2,"preview":"new","url":"https://example.com"}]}}}));
        let tool = &t.tools[0];
        assert_eq!(tool.kind, Some(expected));
        if expected == FileChange {
            assert_eq!(
                (tool.changes[0].additions, tool.changes[0].deletions),
                (1, 1)
            );
        }
        if expected == FileSearch {
            assert_eq!(tool.structured["fileSearchResults"][0]["line"], 2);
        }
        if expected == WebSearch {
            assert_eq!(
                tool.structured["webResults"][0]["url"],
                "https://example.com"
            );
        }
    }
}

#[test]
fn transcript_saved_before_typed_tools_loads_unchanged() {
    let saved = json!({"id":"old","name":"read","detail":"README.md","done":true,"failed":false,
        "output":"saved output","images":[],"structured":null,"parent_call":null,"exit_code":null,
        "subagents":[],"anchor":{"message":1,"offset":0},"block":0,"duration_ms":12});
    let tool: Tool = serde_json::from_value(saved).unwrap();
    assert_eq!(tool.kind, None);
    assert!(tool.changes.is_empty());
    assert_eq!(tool.resolved_kind(), crate::work::ToolKind::Read);
    assert_eq!(tool.output, "saved output");
}

#[test]
fn rpiv_drawer_collects_answers_then_replies_only_to_exact_live_tool_dialogs() {
    let (mut t, commands) = thread(Status::Running);
    let params = json!({"questions":[
        {"header":"Storage","question":"Where?","options":[{"label":"Disk","description":"Local"}]},
        {"question":"Which?","multiSelect":true,"options":[{"label":"A","description":"First"},{"label":"B","description":"Second"}]}
    ]});
    t.apply(json!({"type":"tool_execution_start","toolCallId":"ask-1","toolName":"ask_user_question","args":params}));
    t.apply(json!({"type":"extension_ui_request","id":"first","method":"select","title":"[Storage] Where?",
        "options":["1. Disk \u{2014} Local","2. Type something."]}));
    assert!(t.native.questionnaire.is_some());
    t.questionnaire_toggle(0);
    assert!(t.native_tick(Instant::now() + Duration::from_millis(250)));
    assert_eq!(t.native.questionnaire.as_ref().unwrap().index, 1);
    assert!(
        commands
            .try_iter()
            .all(|r| r["type"] != "extension_ui_response")
    );
    t.questionnaire_toggle(1);
    t.questionnaire_toggle(0);
    t.questionnaire_step(true);
    assert_eq!(t.native.questionnaire.as_ref().unwrap().index, 0);
    t.questionnaire_step(false);
    t.questionnaire_step(false);
    assert_eq!(
        command(&commands, "extension_ui_response")["value"],
        "1. Disk \u{2014} Local"
    );
    assert!(t.request.is_none());
    let second = json!({"type":"extension_ui_request","id":"second","method":"input",
        "title":"Which?\n\n1. A \u{2014} First\n2. B \u{2014} Second\n\nEnter the numbers of all that apply, comma-separated (e.g. \"1,3\"). Add \"; custom answer\" to include your own text, or type only custom text."});
    let mut unrelated = second.clone();
    unrelated["title"] = json!("Unrelated");
    unrelated["id"] = json!("unrelated");
    t.apply(unrelated);
    assert!(t.request.is_some());
    assert!(
        commands
            .try_iter()
            .all(|r| r["type"] != "extension_ui_response")
    );
    t.respond(json!({"value":"Manual answer"}));
    command(&commands, "extension_ui_response");
    t.apply(second.clone());
    let response = command(&commands, "extension_ui_response");
    assert_eq!(response["id"], "second");
    assert_eq!(response["value"], "1,2");
    assert!(t.request.is_none());
    t.apply(json!({"type":"tool_execution_end","toolCallId":"ask-1","result":{"content":[]}}));
    t.apply(second);
    assert!(t.native.questionnaire.is_none());
    assert!(t.request.is_some());
}

#[test]
fn structured_questionnaire_expiry_cancels_auto_advance_and_notifications_expire() {
    let (mut t, commands) = thread(Status::Running);
    t.apply(
        json!({"type":"extension_ui_request","id":"form","method":"input","timeout":1,
        "questions":[{"id":"one","question":"Choose","options":[{"label":"A"}]}]}),
    );
    t.questionnaire_toggle(0);
    assert!(t.native.questionnaire_advance.is_some());
    t.native_tick(Instant::now() + Duration::from_secs(1));
    assert!(t.native.questionnaire.is_none());
    assert!(t.native.questionnaire_advance.is_none());
    assert!(
        commands
            .try_iter()
            .all(|r| r["type"] != "extension_ui_response")
    );
    t.apply(json!({"type":"extension_ui_request","method":"notify","message":"Done"}));
    assert_eq!(t.native.notifications.len(), 1);
    assert!(t.native_deadline().is_some());
    assert!(t.native_tick(Instant::now() + Duration::from_secs(7)));
    assert!(t.native.notifications.is_empty());
    t.native
        .questionnaire_replies
        .push(("tool".into(), "signature".into(), "answer".into()));
    t.cancel_native_dialogs();
    assert!(t.native.questionnaire_replies.is_empty());
}

#[test]
fn single_selection_auto_submits_and_other_edit_cancels_auto_advance() {
    let (mut t, commands) = thread(Status::Running);
    let mut request = json!({"type":"extension_ui_request","id":"form","method":"input",
        "questions":[{"id":"one","question":"Choose","options":[{"label":"A"}]}]});
    t.apply(request.clone());
    t.questionnaire_toggle(0);
    assert!(t.native_tick(Instant::now() + Duration::from_millis(250)));
    let response = command(&commands, "extension_ui_response");
    assert_eq!(response["id"], "form");
    assert_eq!(response["value"], json!({"one":"A"}).to_string());
    request["id"] = json!("other");
    t.apply(request);
    t.questionnaire_toggle(0);
    t.questionnaire_edit().replace_text("Custom");
    assert!(!t.native_tick(Instant::now() + Duration::from_millis(250)));
    assert!(t.request.is_some());
    t.questionnaire_step(false);
    let response = command(&commands, "extension_ui_response");
    assert_eq!(response["value"], json!({"one":"Custom"}).to_string());
    assert!(t.native.questionnaire.is_none());
}

#[test]
fn files_send_and_steer_without_vision_and_recover_from_persisted_queue_references() {
    let path = crate::store::directory().unwrap().join("ordinary.txt");
    std::fs::write(&path, "A file, not an image.").unwrap();
    let file = attachments::attach_file(&path).unwrap();
    let (mut t, commands) = thread(Status::Ready);
    t.supports_images = Some(false);
    t.add_attachment(file.clone()).unwrap();
    t.send();
    let prompt = command(&commands, "prompt");
    assert!(
        prompt["message"]
            .as_str()
            .unwrap()
            .contains("[Attached file \"ordinary.txt\"")
    );
    assert!(prompt.get("images").is_none());
    assert_eq!(t.title, "File: ordinary.txt");
    t.add_attachment(file.clone()).unwrap();
    t.send_during_run();
    let steer = command(&commands, "prompt");
    assert_eq!(steer["streamingBehavior"], "steer");
    assert!(steer["message"].as_str().unwrap().contains("ordinary.txt"));
    t.native.follow_up = true;
    t.add_attachment(file.clone()).unwrap();
    t.send_during_run();
    let saved = t.pending_inputs_value();
    assert!(!saved.to_string().contains("A file, not an image."));
    std::fs::remove_file(path).unwrap();
    let (mut restored, _) = thread(Status::Ready);
    restored.restore_pending_inputs(&saved).unwrap();
    restored.recover_native_inputs();
    assert_eq!(restored.draft_images, vec![file]);
    assert!(
        restored
            .attachment_error
            .as_deref()
            .unwrap()
            .contains("Missing file")
    );
    assert!(restored.native.queue_held);
    assert_eq!(restored.native.queue.len(), 1);
}

fn thread(status: Status) -> (Thread, mpsc::Receiver<Value>) {
    let mut thread = Thread::new(1);
    let (pi, commands, _) = Pi::test_connection();
    thread.pi = Some(pi);
    thread.status = status;
    thread.model_provider = "cpa".into();
    thread.model_id = "fixture".into();
    thread.session = Some("source.jsonl".into());
    thread.context_window = Some(200_000);
    thread.native.session_name = Some(thread.title.clone());
    (thread, commands)
}
fn image_fixture() -> Attachment {
    use image::ImageEncoder;
    let mut bytes = vec![];
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[0, 255, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    attachments::attach_bytes("fixture.png", bytes).unwrap()
}

fn answer(thread: &mut Thread, request: &Value, data: Value) {
    thread.apply(json!({"type":"response","id":request["id"],"command":request["type"],"success":true,"data":data}));
}
fn command(commands: &mpsc::Receiver<Value>, kind: &str) -> Value {
    for value in commands.try_iter() {
        if value["type"] == kind {
            return value;
        }
    }
    panic!("missing {kind}");
}

#[test]
fn rewind_recovers_images_by_entry_id_instead_of_repeated_prompt_text() {
    let (mut t, commands) = thread(Status::Ready);
    let image = image_fixture();
    t.native.fork_points = vec![
        json!({"entryId":"first","text":"Same prompt","content":"Same prompt"}),
        json!({"entryId":"second","text":"Same prompt","content":[{"type":"text","text":"Same prompt"},{"type":"image","shinaAttachment":image.to_value()}]}),
    ];
    t.rewind("second".into());
    let fork = command(&commands, "fork");
    answer(
        &mut t,
        &fork,
        json!({"text":"Same prompt","cancelled":false}),
    );
    assert_eq!(t.draft.text, "Same prompt");
    assert_eq!(t.draft_images[0].id, image.id);
}

#[test]
fn native_subagent_results_keep_individual_status_and_bounded_output() {
    let (mut t, _) = thread(Status::Running);
    t.apply(json!({"type":"tool_execution_start","toolCallId":"agents","toolName":"subagent","args":{}}));
    t.apply(json!({"type":"tool_execution_update","toolCallId":"agents","partialResult":{"content":[],"details":{"results":[
        {"agent":"Research","task":"Read source","model":"fixture","messages":[{"role":"assistant","content":"Progress"}]},
        {"agent":"Review","task":"Review code","finished":true,"exitCode":1,"errorMessage":"Failed"}
    ]}}}));
    assert_eq!(t.tools[0].subagents[0]["status"], "Working");
    assert_eq!(t.tools[0].subagents[0]["output"], "Progress");
    assert_eq!(t.tools[0].subagents[1]["status"], "Failed");
}

#[test]
fn user_events_do_not_duplicate_initial_prompts_or_internal_deliveries() {
    let (mut t, commands) = thread(Status::Ready);
    t.draft.replace_text("/template");
    t.send();
    command(&commands, "prompt");
    t.apply(json!({"type":"message_start","message":{"role":"user","content":"Expanded template content"}}));
    assert_eq!(t.messages.len(), 1);
    assert_eq!(t.messages[0].text, "/template");
    t.enqueue_injection("Result".into());
    t.flush_injection();
    let injection = command(&commands, "prompt");
    t.apply(
        json!({"type":"message_start","message":{"role":"user","content":injection["message"]}}),
    );
    assert_eq!(t.messages.iter().filter(|m| m.role == "Shina").count(), 1);
}

#[test]
fn expanded_steers_are_owned_even_when_user_events_precede_the_ack() {
    let (mut t, commands) = thread(Status::Running);
    t.draft.replace_text("/skill:review");
    t.send_during_run();
    let steer = command(&commands, "prompt");
    t.apply(json!({"type":"message_start","message":{"role":"user","content":"Expanded skill"}}));
    answer(&mut t, &steer, json!({"disposition":"queued"}));
    assert!(t.native.queued_inputs.is_empty());
    assert_eq!(t.messages[0].text, "/skill:review");
    t.disconnect();
    assert!(t.draft.text.is_empty());
}

#[test]
fn fork_metadata_preserves_the_source_and_the_permission_policy() {
    let mut s = State::new(".".into(), false);
    let source = s.current().id;
    s.current_mut().session = Some("original.jsonl".into());
    s.current_mut().native.access = AccessMode::Supervised;
    // A missing workspace fails before process startup. This test never starts an installed Pi.
    s.projects.entries[0].path = s.data_dir.join("missing-fork-workspace");
    s.fork_conversation(None);
    assert_ne!(s.current().id, source);
    assert_eq!(s.threads[0].session.as_deref(), Some("original.jsonl"));
    assert!(s.current().session.is_none());
    assert_eq!(
        s.current().native.fork_source.as_deref(),
        Some("original.jsonl")
    );
    assert_eq!(s.current().native.access, AccessMode::Supervised);
    let saved = s.snapshot();
    let mut restored = State::new(PathBuf::new(), false);
    restored.restore_global(&saved).unwrap();
    assert_eq!(restored.current().native.access, AccessMode::Supervised);
    assert_eq!(
        restored.current().native.fork_source.as_deref(),
        Some("original.jsonl")
    );
}

#[test]
fn a_stale_state_response_cannot_rebind_a_rewound_thread_to_its_source() {
    let (mut t, commands) = thread(Status::Ready);
    t.command("get_state", json!({}));
    let stale = command(&commands, "get_state");
    t.native.fork_points = vec![json!({"entryId":"entry"})];
    t.rewind("entry".into());
    let fork = command(&commands, "fork");
    answer(&mut t, &fork, json!({"cancelled":false}));
    let state = command(&commands, "get_state");
    answer(
        &mut t,
        &state,
        json!({"sessionFile":"rewound.jsonl","isStreaming":false}),
    );
    answer(&mut t, &stale, json!({"sessionFile":"source.jsonl"}));
    assert_eq!(t.session.as_deref(), Some("rewound.jsonl"));
    let history = command(&commands, "get_entries");
    let mut record = json!({"type":"response","id":history["id"],"command":"get_entries","success":true,"data":{"entries":[],"leafId":null}});
    session_history(&mut record);
    t.apply(record);
    answer(&mut t, &stale, json!({"sessionFile":"source.jsonl"}));
    assert_eq!(t.session.as_deref(), Some("rewound.jsonl"));
}

#[test]
fn native_compaction_sends_the_rpc_and_settles_without_an_agent_turn() {
    let (mut t, commands) = thread(Status::Completed);
    t.compact(Some("Keep implementation decisions".into()));
    let compact = command(&commands, "compact");
    assert_eq!(
        compact["customInstructions"],
        "Keep implementation decisions"
    );
    assert!(t.status.busy());
    t.apply(json!({"type":"compaction_start","reason":"manual"}));
    t.apply(json!({"type":"compaction_end","reason":"manual","result":{"summary":"Kept decisions","tokensBefore":100000,"estimatedTokensAfter":12000}}));
    assert_eq!(t.native.compactions.len(), 1);
    answer(&mut t, &compact, json!({"summary":"Kept decisions"}));
    let probe = command(&commands, "get_state");
    answer(
        &mut t,
        &probe,
        json!({"isStreaming":false,"isCompacting":false,"sessionFile":"source.jsonl"}),
    );
    assert_eq!(t.status, Status::Ready);
    assert!(!t.native.operating());
    assert!(t.context.usage.percent.is_none());
    assert_eq!(
        t.native.compactions[0]["result"]["estimatedTokensAfter"],
        12000
    );
}

#[test]
fn stop_during_native_compaction_retires_the_process_and_keeps_the_draft() {
    let (mut t, commands) = thread(Status::Ready);
    t.draft.replace_text("Unsent text");
    t.compact(None);
    command(&commands, "compact");
    t.apply(json!({"type":"compaction_start","reason":"manual"}));
    t.stop();
    assert!(t.pi.is_none());
    assert_eq!(t.draft.text, "Unsent text");
    assert_eq!(t.status, Status::Ready);
    assert_eq!(t.native.compactions.last().unwrap()["aborted"], true);
}

#[test]
fn compaction_failure_and_extension_cancellation_leave_the_thread_usable() {
    let (mut t, commands) = thread(Status::Completed);
    t.compact(None);
    let compact = command(&commands, "compact");
    t.apply(json!({"type":"response","id":compact["id"],"command":"compact","success":false,"error":"Cancelled by extension"}));
    let probe = command(&commands, "get_state");
    answer(
        &mut t,
        &probe,
        json!({"isStreaming":false,"isCompacting":false,"sessionFile":"source.jsonl"}),
    );
    assert!(!t.status.busy());
    assert!(!t.native.operating());
    assert_eq!(t.error.as_deref(), Some("Cancelled by extension"));
    t.draft.replace_text("Still usable");
    t.send();
    assert_eq!(command(&commands, "prompt")["message"], "Still usable");
}

#[test]
fn a_steer_does_not_reset_the_current_assistant_or_fail_the_run_when_rejected() {
    let (mut t, commands) = thread(Status::Running);
    t.assistant().text = "Already writing".into();
    let assistant = t.assistant;
    t.draft.replace_text("Focus on the tests");
    t.send_during_run();
    let steer = command(&commands, "prompt");
    assert_eq!(steer["streamingBehavior"], "steer");
    assert_eq!(t.assistant, assistant);
    assert!(t.draft.text.is_empty());
    t.apply(json!({"type":"response","id":steer["id"],"command":"prompt","success":false,"error":"Rejected steer"}));
    assert_eq!(t.status, Status::Running);
    assert_eq!(t.draft.text, "Focus on the tests");
    assert_eq!(t.messages[0].text, "Already writing");
}

#[test]
fn a_compaction_race_defers_a_steer_and_submits_it_once_after_compaction() {
    let (mut t, commands) = thread(Status::Running);
    t.draft.replace_text("After compaction");
    t.send_during_run();
    let first = command(&commands, "prompt");
    t.apply(json!({"type":"response","id":first["id"],"command":"prompt","success":false,"error":"Cannot submit a prompt while compaction is in progress."}));
    assert!(commands.try_recv().is_err());
    t.apply(json!({"type":"compaction_start"}));
    t.apply(json!({"type":"compaction_end","result":{"summary":"Summary"}}));
    let retry = command(&commands, "prompt");
    assert_eq!(retry["message"], "After compaction");
    assert_ne!(retry["id"], first["id"]);
    answer(&mut t, &retry, json!({"disposition":"queued"}));
    assert!(t.native.sends.is_empty());
    assert_eq!(t.native.queued_inputs.len(), 1);
    assert!(commands.try_recv().is_err());
}

#[test]
fn follow_up_stop_and_clear_queue_keep_original_text_and_images() {
    let (mut t, commands) = thread(Status::Running);
    let image = crate::attachments::attach_bytes("queued.png", {
        use image::ImageEncoder;
        let mut bytes = vec![];
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    })
    .unwrap();
    t.supports_images = Some(true);
    t.native.follow_up = true;
    t.draft.replace_text("Do this later");
    t.draft_images.push(image.clone());
    t.send_during_run();
    assert!(
        commands.try_recv().is_err(),
        "application follow-ups are not sent until settlement"
    );
    assert_eq!(t.native.queue[0].prompt.images[0].id, image.id);
    t.stop();
    let clear = command(&commands, "clear_queue");
    answer(&mut t, &clear, json!({"steering":[],"followUp":[]}));
    assert!(t.draft.text.is_empty());
    assert!(t.draft_images.is_empty());
    assert!(t.native.queue_held);
    assert_eq!(t.native.queue[0].prompt.text, "Do this later");
    assert_eq!(t.native.queue[0].prompt.images[0].id, image.id);
    t.clear_user_queue();
    assert_eq!(t.draft.text, "Do this later");
    assert_eq!(t.draft_images[0].id, image.id);
    assert!(t.native.queued_inputs.is_empty());
}

#[test]
fn queued_messages_can_be_edited_cancelled_promoted_and_run_after_settlement() {
    let (mut t, commands) = thread(Status::Running);
    t.native.follow_up = true;
    t.draft.replace_text("Queued A");
    t.send_during_run();
    let a = t.native.queue[0].id;
    t.draft.replace_text("Queued B");
    t.send_during_run();
    let b = t.native.queue[1].id;
    t.cancel_queued(a);
    assert_eq!(t.native.queue.len(), 1);
    t.edit_queued(b);
    assert_eq!(t.draft.text, "Queued B");
    assert_eq!(t.native.queue.len(), 1);
    t.save_queued_edit();
    let b = t.native.queue[0].id;
    t.steer_queued(b);
    let steer = command(&commands, "prompt");
    assert_eq!(steer["streamingBehavior"], "steer");
    answer(&mut t, &steer, json!({"disposition":"queued"}));
    t.draft.replace_text("Queued C");
    t.send_during_run();
    t.draft.replace_text("Keep typing");
    t.apply(json!({"type":"agent_settled","aborted":false}));
    let next = command(&commands, "prompt");
    assert_eq!(next["message"], "Queued C");
    assert_eq!(t.draft.text, "Keep typing");
    assert_eq!(t.status, Status::Running);
}

#[test]
fn delivered_queued_messages_are_not_restored_after_a_disconnect() {
    let (mut t, commands) = thread(Status::Running);
    t.draft.replace_text("Steer");
    t.send_during_run();
    let send = command(&commands, "prompt");
    answer(&mut t, &send, json!({"disposition":"queued"}));
    t.apply(json!({"type":"message_start","message":{"role":"user","content":"Steer"}}));
    assert_eq!(t.messages.len(), 1);
    t.disconnect();
    assert!(t.draft.text.is_empty());
}

#[test]
fn pending_steers_survive_profile_save_and_return_to_draft_without_auto_resending() {
    let (mut t, commands) = thread(Status::Running);
    t.draft.replace_text("Unacknowledged");
    t.send_during_run();
    command(&commands, "prompt");
    let pending = t.pending_inputs_value();
    let (mut restored, commands) = thread(Status::Ready);
    restored.restore_pending_inputs(&pending).unwrap();
    restored.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}));
    assert_eq!(restored.draft.text, "Unacknowledged");
    assert!(!commands.try_iter().any(|r| r["type"] == "prompt"));
}

#[test]
fn authoritative_history_prevents_recovering_a_prompt_pi_already_saved() {
    let (mut t, _) = thread(Status::Ready);
    t.restore_pending_inputs(&json!([{"text":"Saved","images":[],"after":0}]))
        .unwrap();
    t.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[{"role":"user","content":"Saved"}]}}));
    assert!(t.draft.text.is_empty());
}

#[test]
fn rewind_uses_entry_ids_and_waits_for_authoritative_state_and_history() {
    let (mut t, commands) = thread(Status::Ready);
    t.native.fork_points = vec![json!({"entryId":"target","text":"Repeated prompt"})];
    t.rewind("target".into());
    let fork = command(&commands, "fork");
    assert_eq!(fork["entryId"], "target");
    assert!(t.configuring);
    answer(
        &mut t,
        &fork,
        json!({"text":"Repeated prompt","cancelled":false}),
    );
    let state = command(&commands, "get_state");
    answer(
        &mut t,
        &state,
        json!({"sessionFile":"rewound.jsonl","isStreaming":false}),
    );
    let history = command(&commands, "get_entries");
    let mut record = json!({"type":"response","id":history["id"],"command":"get_entries","success":true,"data":{"entries":[],"leafId":null}});
    session_history(&mut record);
    t.apply(record);
    assert_eq!(t.session.as_deref(), Some("rewound.jsonl"));
    assert_eq!(t.native.previous_session.as_deref(), Some("source.jsonl"));
    assert_eq!(t.draft.text, "Repeated prompt");
    assert!(!t.configuring);
    assert!(t.messages.is_empty());
}

#[test]
fn a_rewind_reapplies_the_pinned_selection_before_another_prompt_can_run() {
    let (mut t, commands) = thread(Status::Ready);
    t.selected_model = Some(("cpa".into(), "pinned".into()));
    t.native.fork_points = vec![json!({"entryId":"target","text":"Prompt"})];
    t.rewind("target".into());
    let fork = command(&commands, "fork");
    answer(&mut t, &fork, json!({"cancelled":false}));
    let state = command(&commands, "get_state");
    answer(
        &mut t,
        &state,
        json!({"sessionFile":"rewound.jsonl","isStreaming":false,"model":{"provider":"cpa","id":"old-model"}}),
    );
    let history = command(&commands, "get_entries");
    let mut response = json!({"type":"response","id":history["id"],"command":"get_entries","success":true,"data":{"entries":[],"leafId":null}});
    session_history(&mut response);
    t.apply(response);
    let model = command(&commands, "set_model");
    assert_eq!(model["modelId"], "pinned");
    assert!(t.configuring);
    assert!(!commands.try_iter().any(|c| c["type"] == "prompt"));
}

#[test]
fn cancelled_rewind_does_not_rebind_or_clear_the_original_conversation() {
    let (mut t, commands) = thread(Status::Ready);
    t.messages.push(Message {
        id: uuid::Uuid::new_v4().to_string(),
        role: "You",
        text: "Keep".into(),
        images: vec![],
        context: None,
    });
    t.native.fork_points = vec![json!({"entryId":"target"})];
    t.rewind("target".into());
    let fork = command(&commands, "fork");
    answer(&mut t, &fork, json!({"cancelled":true}));
    assert_eq!(t.session.as_deref(), Some("source.jsonl"));
    assert_eq!(t.messages[0].text, "Keep");
    assert!(!t.configuring);
}

#[test]
fn history_uses_leaf_ancestry_and_keeps_pre_compaction_messages() {
    let mut record = json!({"type":"response","command":"get_entries","success":true,"data":{"leafId":"compact","entries":[
        {"id":"u","type":"message","parentId":null,"message":{"role":"user","content":"Before"}},
        {"id":"abandoned","type":"message","parentId":"u","message":{"role":"assistant","content":"Wrong branch"}},
        {"id":"a","type":"message","parentId":"u","message":{"role":"assistant","content":"Right branch"}},
        {"id":"compact","type":"compaction","parentId":"a","summary":"Summary","tokensBefore":10000}
    ]}});
    session_history(&mut record);
    assert_eq!(record["data"]["messages"].as_array().unwrap().len(), 2);
    assert_eq!(record["data"]["messages"][0]["content"], "Before");
    assert_eq!(record["data"]["messages"][1]["content"], "Right branch");
    assert_eq!(record["data"]["compactions"][0]["summary"], "Summary");
    assert_eq!(record["data"]["compactions"][0]["historyIndex"], 2);
    let mut broken = json!({"type":"response","command":"get_entries","success":true,"data":{"leafId":"missing","entries":[]}});
    session_history(&mut broken);
    assert_eq!(broken["success"], false);
}

#[test]
fn pi_compaction_replay_records_the_middle_of_the_transcript() {
    let mut record = json!({"type":"response","id":"history","command":"get_entries","success":true,"data":{"leafId":"b","entries":[
        {"id":"u","type":"message","parentId":null,"message":{"role":"user","content":"Before"}},
        {"id":"a","type":"message","parentId":"u","message":{"role":"assistant","content":"First answer"}},
        {"id":"compact","type":"compaction","parentId":"a","summary":"Kept facts"},
        {"id":"v","type":"message","parentId":"compact","message":{"role":"user","content":"After"}},
        {"id":"b","type":"message","parentId":"v","message":{"role":"assistant","content":"Second answer"}}
    ]}});
    session_history(&mut record);
    let mut thread = Thread::new(1);
    thread.apply(record);
    assert_eq!(
        crate::work::compaction_anchor(&thread, &thread.native.compactions[0]),
        ToolAnchor {
            message: 2,
            offset: 0
        }
    );
    assert_eq!(thread.messages.len(), 4);
}

#[test]
fn stop_records_a_durable_marker_before_the_provider_acknowledges_it() {
    let (mut t, commands) = thread(Status::Running);
    t.messages.push(Message {
        id: uuid::Uuid::new_v4().to_string(),
        role: "You",
        text: "Work".into(),
        images: vec![],
        context: None,
    });
    t.started = Some(Instant::now() - Duration::from_secs(134));
    t.stop();
    assert!(t.work[0].stopped);
    assert_eq!(t.work[0].elapsed, Some(134));
    assert_eq!(command(&commands, "abort")["type"], "abort");
    let saved = serde_json::to_value(&t.work[0]).unwrap();
    let restored: AssistantWork = serde_json::from_value(saved).unwrap();
    assert!(restored.stopped);
}

#[test]
fn command_discovery_is_workspace_local_and_retains_a_previous_inventory_on_failure() {
    let (mut t, _) = thread(Status::Ready);
    t.apply(
        json!({"type":"response","command":"get_commands","success":true,"data":{"commands":[
            {"name":"skill:review","source":"skill","description":"Review changes"},
            {"name":"fix","source":"prompt"},{"name":"fix","source":"extension"}
        ]}}),
    );
    assert_eq!(t.native.commands.len(), 2);
    t.draft.replace_text("/skill:");
    t.draft.focused = true;
    assert_eq!(t.slash_matches()[0].name, "skill:review");
    t.apply(json!({"type":"response","command":"get_commands","success":false,"error":"Failed discovery"}));
    assert_eq!(t.native.commands.len(), 2);
    assert!(t.error.is_none());
    let other = Thread::new(2);
    assert!(other.native.commands.is_empty());
}

#[test]
fn slash_commands_use_the_caret_token_and_keep_provider_commands_at_prompt_start() {
    let (mut t, _) = thread(Status::Ready);
    t.apply(json!({"type":"response","command":"get_commands","success":true,
        "data":{"commands":[{"name":"skill:review","source":"skill"},{"name":"provider-only","source":"ACP"}]}}));
    t.draft.focused = true;
    t.draft.replace_text("/");
    assert!(t.slash_matches().iter().any(|c| c.name == "provider-only"));
    t.draft.replace_text("please /");
    let matches = t.slash_matches();
    assert!(matches.iter().any(|c| c.name == "model"));
    assert!(matches.iter().any(|c| c.name == "skill:review"));
    assert!(
        !matches
            .iter()
            .any(|c| c.name == "provider-only" || c.name == "compact")
    );
    t.draft.replace_text("please /review ");
    assert!(t.slash_trigger().is_none());
    t.draft.replace_text("please /nothing");
    assert!(t.slash_trigger().is_some());
    assert!(t.slash_matches().is_empty());
}

#[test]
fn overlapping_dialogs_expire_without_losing_the_next_request_or_timing_out_startup() {
    let (mut t, commands) = thread(Status::Ready);
    t.initializing = true;
    t.session_started = Some(Instant::now() - Duration::from_secs(100));
    t.apply(json!({"type":"extension_ui_request","id":"first","method":"confirm","title":"Trust?","timeout":1}));
    t.apply(
        json!({"type":"extension_ui_request","id":"second","method":"input","prefill":"Prefill"}),
    );
    assert!(!t.check_deadlines(Instant::now()));
    t.native_tick(Instant::now() + Duration::from_secs(1));
    assert_eq!(t.request.as_ref().unwrap()["id"], "second");
    t.respond(json!({"value":"Answer"}));
    let response = command(&commands, "extension_ui_response");
    assert_eq!(response["id"], "second");
    assert_eq!(t.status, Status::Ready);
    assert!(t.session_started.unwrap().elapsed() < Duration::from_secs(1));
}

#[test]
fn session_approvals_are_scoped_to_the_process_and_other_dialogs_still_need_the_user() {
    let (mut t, commands) = thread(Status::Running);
    let mut request = json!({"type":"extension_ui_request","id":"first","method":"confirm","title":"Allow write?","message":"file"});
    t.apply(request.clone());
    t.approve_for_session();
    command(&commands, "extension_ui_response");
    request["id"] = json!("second");
    t.apply(request.clone());
    assert_eq!(command(&commands, "extension_ui_response")["id"], "second");
    request["id"] = json!("third");
    request["message"] = json!("another file");
    t.apply(request);
    assert!(t.request.is_some());
    t.disconnect();
    assert!(t.native.session_approvals.is_empty());
}

#[test]
fn tool_results_keep_structured_data_exit_codes_and_nested_call_metadata() {
    let (mut t, _) = thread(Status::Running);
    t.apply(json!({"type":"tool_execution_start","toolCallId":"parent/1","parentToolCallId":"parent","toolName":"powershell","args":{"command":"exit 7"}}));
    t.apply(json!({"type":"tool_execution_end","toolCallId":"parent/1","result":{"content":[{"type":"text","text":"Output"}],"structuredContent":{"exit_code":7},"details":{"custom":"value"}},"isError":true}));
    assert_eq!(t.tools[0].parent_call.as_deref(), Some("parent"));
    assert_eq!(t.tools[0].exit_code, Some(7));
    assert_eq!(t.tools[0].structured["details"]["custom"], "value");
    assert_eq!(t.tools[0].output, "Output");
}

#[test]
fn access_mode_id_parse_round_trip_and_unknown_fallback() {
    for mode in AccessMode::ALL {
        assert_eq!(AccessMode::parse(mode.id()), Some(mode));
    }
    assert_eq!(AccessMode::Auto.id(), "auto");
    assert_eq!(AccessMode::Auto.label(), "Auto");
    assert_eq!(AccessMode::parse("future-mode"), None);

    let mut state = State::new(PathBuf::new(), false);
    let mut snapshot = state.snapshot();
    snapshot["threads"][0]["accessMode"] = json!("future-mode");
    state.restore_global(&snapshot).unwrap();
    assert_eq!(state.current().native.access, AccessMode::Supervised);

    state
        .execute_command(&crate::protocol::Command::new(
            "thread.configure",
            json!({"threadId": state.current().id, "access": "future-mode"}),
        ))
        .unwrap();
    assert_eq!(state.current().native.access, AccessMode::Supervised);
}

#[test]
fn supported_access_modes_table_and_compatible_resolution() {
    assert_eq!(
        supported_access_modes("pi"),
        &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Edits,
            AccessMode::Full,
        ]
    );
    assert_eq!(
        supported_access_modes("antigravity"),
        &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Edits,
            AccessMode::Full,
        ]
    );
    assert_eq!(supported_access_modes("codex"), &AccessMode::ALL);
    assert_eq!(supported_access_modes("claude"), &AccessMode::ALL);
    assert_eq!(supported_access_modes("opencode"), &AccessMode::ALL);
    assert_eq!(supported_access_modes("deepseek"), &AccessMode::ALL);
    assert_eq!(
        supported_access_modes("kiro"),
        &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Full
        ]
    );
    assert_eq!(
        supported_access_modes("muse"),
        &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Full
        ]
    );
    assert_eq!(
        supported_access_modes("fx"),
        &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Auto
        ]
    );
    assert_eq!(
        supported_access_modes("grok"),
        &[
            AccessMode::ReadOnly,
            AccessMode::Supervised,
            AccessMode::Auto,
            AccessMode::Full,
        ]
    );
    assert_eq!(supported_access_modes("gemini"), &AccessMode::ALL);

    // Supported current mode is kept.
    let (modes, resolved) =
        resolve_compatible_access_mode(AccessMode::Edits, supported_access_modes("pi"));
    assert!(!modes.contains(&AccessMode::Auto));
    assert_eq!(resolved, AccessMode::Edits);

    // Read only is preserved when supported.
    let (_, resolved) =
        resolve_compatible_access_mode(AccessMode::ReadOnly, supported_access_modes("pi"));
    assert_eq!(resolved, AccessMode::ReadOnly);

    // Unsupported current mode falls back to first supported mode skipping Read only.
    let (_, resolved) =
        resolve_compatible_access_mode(AccessMode::Auto, supported_access_modes("pi"));
    assert_eq!(resolved, AccessMode::Supervised);

    // Empty supported list means all modes are supported.
    let (modes, resolved) = resolve_compatible_access_mode(AccessMode::Auto, &[]);
    assert_eq!(modes, AccessMode::ALL);
    assert_eq!(resolved, AccessMode::Auto);
}

#[test]
fn managed_policy_denial_and_clamp_behaviour() {
    // Missing file means no policy/ceiling.
    let dir = crate::store::directory().unwrap();
    let policy_path = dir.join("policy.json");
    assert_eq!(read_managed_policy_from_path(&policy_path), None);
    assert_eq!(managed_access_mode_denial(None, AccessMode::Full), None);
    assert_eq!(
        clamp_managed_access_mode(None, AccessMode::Full, &AccessMode::ALL),
        Some(AccessMode::Full)
    );

    // Policy with extra fields ignores unknown keys and reads only organization + limits.maxRuntimeMode.
    std::fs::write(
        &policy_path,
        r#"{
            "schemaVersion": 1,
            "organization": "Example Corp",
            "supportUrl": "https://example.com/support",
            "limits": {
                "maxRuntimeMode": "auto-accept-edits",
                "webhookTasks": false
            }
        }"#,
    )
    .unwrap();
    let policy = read_managed_policy_from_path(&policy_path).unwrap();
    assert_eq!(policy.organization, "Example Corp");
    assert_eq!(policy.max_access, Some(AccessMode::Edits));

    // Ceiling above mode: allowed, unchanged.
    assert_eq!(
        managed_access_mode_denial(Some(&policy), AccessMode::Supervised),
        None
    );
    assert_eq!(
        clamp_managed_access_mode(Some(&policy), AccessMode::Supervised, &AccessMode::ALL),
        Some(AccessMode::Supervised)
    );

    // Ceiling equal to mode: allowed, unchanged.
    assert_eq!(
        managed_access_mode_denial(Some(&policy), AccessMode::Edits),
        None
    );
    assert_eq!(
        clamp_managed_access_mode(Some(&policy), AccessMode::Edits, &AccessMode::ALL),
        Some(AccessMode::Edits)
    );

    // Ceiling below mode: denied with exact message and clamped to ceiling.
    assert_eq!(
        managed_access_mode_denial(Some(&policy), AccessMode::Auto).as_deref(),
        Some("Example Corp limits threads to Auto-accept edits.")
    );
    assert_eq!(
        managed_access_mode_denial(Some(&policy), AccessMode::Full).as_deref(),
        Some("Example Corp limits threads to Auto-accept edits.")
    );
    assert_eq!(
        clamp_managed_access_mode(Some(&policy), AccessMode::Full, &AccessMode::ALL),
        Some(AccessMode::Edits)
    );
}

#[test]
fn pi_session_stats_produce_estimated_and_inferred_breakdown_while_acp_leaves_none() {
    let (mut t, commands) = thread(Status::Ready);
    t.apply(json!({
        "type": "message_start",
        "message": { "role": "user", "content": [{ "type": "text", "text": &"x".repeat(3500) }] }
    }));
    t.refresh_context_usage();
    let req = command(&commands, "get_session_stats");
    answer(
        &mut t,
        &req,
        json!({"contextUsage": {"tokens": 5000, "contextWindow": 100000, "percent": 5.0}}),
    );
    let breakdown = t
        .context
        .usage
        .breakdown
        .as_ref()
        .expect("expected Pi breakdown");
    assert_eq!(
        breakdown,
        &vec![
            crate::context_breakdown::ContextBreakdownRow {
                label: "System".into(),
                tokens: 4000,
                kind: crate::context_breakdown::BreakdownRowKind::Inferred,
            },
            crate::context_breakdown::ContextBreakdownRow {
                label: "Messages".into(),
                tokens: 1000,
                kind: crate::context_breakdown::BreakdownRowKind::Estimated,
            },
        ]
    );

    let (mut acp_thread, acp_commands) = thread(Status::Ready);
    acp_thread.harness = "acp-fixture".into();
    acp_thread.refresh_context_usage();
    let acp_req = command(&acp_commands, "get_session_stats");
    answer(&mut acp_thread, &acp_req, json!({"contextUsage": null}));
    assert_eq!(acp_thread.context.usage.breakdown, None);
}
