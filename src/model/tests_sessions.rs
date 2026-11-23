use super::*;

fn next_stats_request(commands: &mpsc::Receiver<Value>) -> Value {
    let mut request = commands
        .try_recv()
        .expect("expected a context stats request");
    if request["type"] == "set_session_name" {
        request = commands.try_recv().unwrap();
    }
    assert_eq!(request["type"], "get_session_stats");
    request
}

fn answer_stats(thread: &mut Thread, request: &Value, context_usage: Value) {
    thread.apply(json!({
        "type": "response",
        "id": request["id"],
        "command": "get_session_stats",
        "success": true,
        "data": {"contextUsage": context_usage}
    }));
}

#[test]
fn context_stats_refresh_on_startup_turns_and_model_changes() {
    let mut thread = Thread::new(1);
    thread.harness = "pi".into();
    let (pi, commands, _events) = Pi::test_connection();
    thread.pi = Some(pi);
    thread.initializing = true;
    thread.context_startup_requested = false;
    thread.apply(json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
        "isStreaming":false,
        "model":{"provider":"cpa","id":"vision","name":"Vision","contextWindow":200000,"input":["text","image"]},
        "sessionFile":"saved.jsonl"
    }}));
    thread.apply(json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}));

    let startup = next_stats_request(&commands);
    assert!(thread.context.is_pending());
    assert_eq!(thread.context_window, Some(200000));
    assert_eq!(thread.supports_images, Some(true));
    answer_stats(
        &mut thread,
        &startup,
        json!({"tokens":60000,"contextWindow":200000,"percent":30}),
    );
    assert_eq!(thread.context.usage.tokens, Some(60000));
    assert_eq!(thread.context.usage.percent, Some(30.0));
    assert!(!thread.context.is_pending());

    thread.apply(json!({"type":"agent_settled","aborted":false}));
    let turn = next_stats_request(&commands);
    answer_stats(
        &mut thread,
        &turn,
        json!({"tokens":90000,"contextWindow":200000,"percent":45}),
    );
    assert_eq!(thread.context.usage.percent, Some(45.0));

    thread.models = vec![
        AgentModel::from_value(&json!({
            "provider":"cpa","id":"other","name":"Other","contextWindow":100000,"input":["text"]
        }))
        .unwrap(),
    ];
    thread.set_model("cpa".into(), "other".into());
    assert_eq!(commands.try_recv().unwrap()["type"], "set_model");
    thread.apply(
        json!({"type":"response","command":"set_model","success":true,"data":{
            "provider":"cpa","id":"other","name":"Other","contextWindow":100000,"input":["text"]
        }}),
    );
    assert!(
        thread.context.usage.percent.is_none(),
        "the old model's percentage must not survive a model change"
    );
    assert_eq!(thread.context_window, Some(100000));
    assert_eq!(thread.supports_images, Some(false));
    assert_eq!(commands.try_recv().unwrap()["type"], "get_state");
    assert_eq!(
        commands.try_recv().unwrap()["type"],
        "get_available_thinking_levels"
    );
    let changed = next_stats_request(&commands);
    answer_stats(
        &mut thread,
        &changed,
        json!({"tokens":20000,"contextWindow":100000,"percent":20}),
    );
    assert_eq!(thread.context.usage.percent, Some(20.0));
}

#[test]
fn compaction_marks_context_unknown_until_fresh_usage_arrives() {
    let mut thread = Thread::new(1);
    let (pi, commands, _events) = Pi::test_connection();
    thread.pi = Some(pi);
    thread.context_window = Some(200000);
    thread.context.absorb(&json!({
        "contextUsage":{"tokens":120000,"contextWindow":200000,"percent":60}
    }));
    thread.apply(json!({"type":"compaction_start"}));
    assert_eq!(
        thread.context.phase,
        crate::context::ContextPhase::Compacting
    );
    assert_eq!(thread.context.usage.tokens, None);
    assert_eq!(thread.context.usage.percent, None);
    assert_eq!(thread.context.usage.context_window, Some(200000));
    assert!(
        commands.try_recv().is_err(),
        "compaction start must not start a polling loop"
    );

    thread.apply(json!({"type":"compaction_end"}));
    assert_eq!(
        thread.context.phase,
        crate::context::ContextPhase::AwaitingUsage
    );
    let after_compaction = next_stats_request(&commands);
    answer_stats(
        &mut thread,
        &after_compaction,
        json!({"tokens":null,"contextWindow":200000,"percent":null}),
    );
    assert_eq!(thread.context.usage.tokens, None);
    assert_eq!(thread.context.usage.percent, None);
    assert!(
        thread
            .context
            .accessible_label(None)
            .contains("unknown after compaction")
    );

    thread.apply(json!({"type":"agent_settled","aborted":false}));
    let fresh = next_stats_request(&commands);
    answer_stats(
        &mut thread,
        &fresh,
        json!({"tokens":5000,"contextWindow":200000,"percent":2.5}),
    );
    assert_eq!(thread.context.phase, crate::context::ContextPhase::Idle);
    assert_eq!(thread.context.usage.percent, Some(2.5));
}

#[test]
fn idle_reclaim_preserves_the_session_and_reattaches_without_duplicate_history() {
    let mut thread = Thread::new(1);
    let (pi, _commands, _events) = Pi::test_connection();
    thread.pi = Some(pi);
    thread.session = Some("saved.jsonl".into());
    thread.model_provider = "cpa".into();
    thread.messages.push(Message {
        id: uuid::Uuid::new_v4().to_string(),
        role: "Pi",
        text: "Previous answer".into(),
        images: Vec::new(),
        context: None,
    });
    thread.status = Status::Completed;
    let idle = thread.last_activity + IDLE_TIMEOUT;
    thread.status = Status::Running;
    assert!(!thread.reclaim_idle(idle), "never reclaim work");
    thread.status = Status::Waiting;
    assert!(!thread.reclaim_idle(idle), "never reclaim a dialog");
    thread.status = Status::Completed;
    assert!(thread.reclaim_idle(idle));
    assert!(thread.pi.is_none());
    assert_eq!(thread.status, Status::Completed);
    assert_eq!(thread.session.as_deref(), Some("saved.jsonl"));
    assert_eq!(thread.messages.len(), 1);
    thread.draft.replace_text("Continue");
    thread.send();
    let (pi, commands, _events) = Pi::test_connection();
    thread.pi = Some(pi);
    thread.initializing = true;
    thread.state_loaded = false;
    thread.history_loaded = false;
    thread.apply(
        json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{
            "messages":[{"role":"assistant","content":"Previous answer"}]
        }}),
    );
    thread.apply(
        json!({"type":"response","id":"state","command":"get_state","success":true,"data":{
            "model":{"provider":"cpa","id":"restored"},"sessionFile":"saved.jsonl"
        }}),
    );
    assert_eq!(commands.try_recv().unwrap()["type"], "set_session_name");
    assert_eq!(commands.try_recv().unwrap()["type"], "get_session_stats");
    assert_eq!(commands.try_recv().unwrap()["message"], "Continue");
    assert_eq!(thread.messages.len(), 2);
    assert_eq!(thread.messages[0].text, "Previous answer");
    assert_eq!(thread.messages[1].text, "Continue");
}

#[test]
fn a_crash_restores_an_unaccepted_message_and_retry_uses_a_fresh_transport() {
    let mut state = State::new(".".into(), false);
    let (pi, _commands, events) = Pi::test_connection();
    state.current_mut().pi = Some(pi);
    state.current_mut().model_provider = "cpa".into();
    state.current_mut().session = Some("saved.jsonl".into());
    state.current_mut().draft.replace_text("Try this");
    state.send();
    events.send(Event::Exited).unwrap();
    poll(&mut state, 0.0, &mut Tx::default());
    assert_eq!(state.current().status, Status::Failed);
    assert!(state.current().pi.is_none());
    assert_eq!(state.current().draft.text, "Try this");
    assert!(state.current().messages.is_empty());
    let (pi, commands, events) = Pi::test_connection();
    state.current_mut().pi = Some(pi);
    state.current_mut().initializing = true;
    state.retry();
    assert_eq!(state.current().status, Status::Running);
    assert_eq!(state.current().messages.len(), 1);
    for record in [
        json!({"type":"response","id":"state","command":"get_state","success":true,"data":{"model":{"provider":"cpa","id":"restored"},"sessionFile":"saved.jsonl"}}),
        json!({"type":"response","id":"history","command":"get_messages","success":true,"data":{"messages":[]}}),
    ] {
        events.send(Event::Record(record)).unwrap();
    }
    poll(&mut state, 0.0, &mut Tx::default());
    let prompt = commands
        .try_iter()
        .find(|command| command["type"] == "prompt")
        .expect("retried prompt after the handshake");
    assert_eq!(prompt["message"], "Try this");
    assert!(state.current().error.is_none());
}

#[test]
fn an_accepted_run_is_not_replayed_automatically_after_a_crash() {
    let mut state = State::new(".".into(), false);
    let (pi, commands, events) = Pi::test_connection();
    state.current_mut().pi = Some(pi);
    state.current_mut().model_provider = "cpa".into();
    state.current_mut().draft.replace_text("Edit a file");
    state.send();
    let prompt = commands.try_recv().unwrap();
    state
        .current_mut()
        .apply(json!({"type":"response","command":"prompt","id":prompt["id"],"success":true}));
    events
        .send(Event::Error("Pi process crashed".into()))
        .unwrap();
    poll(&mut state, 0.0, &mut Tx::default());
    assert_eq!(state.current().status, Status::Failed);
    assert!(state.current().draft.text.is_empty());
    assert_eq!(
        state
            .current()
            .retry_prompt
            .as_ref()
            .map(|prompt| prompt.text.as_str()),
        Some("Edit a file")
    );
    assert!(state.current().pi.is_none());
    assert_eq!(state.current().messages.len(), 1);
}

#[test]
fn failed_startup_settings_restore_the_message_instead_of_sending_with_defaults() {
    let mut thread = Thread::new(1);
    thread.harness = "pi".into();
    thread.set_model("cpa".into(), "missing".into());
    thread.draft.replace_text("Keep my model choice");
    thread.send();
    let (pi, commands, _events) = Pi::test_connection();
    thread.pi = Some(pi);
    thread.initializing = true;
    thread.state_loaded = true;
    thread.history_loaded = true;
    thread.drive_session();
    assert_eq!(commands.try_recv().unwrap()["type"], "set_model");
    thread.apply(json!({"type":"response","command":"set_model","success":false,"error":"Model unavailable"}));
    assert_eq!(thread.status, Status::Failed);
    assert_eq!(thread.draft.text, "Keep my model choice");
    assert!(thread.pi.is_none());
    assert!(commands.try_recv().is_err());
}

#[test]
fn creating_and_editing_a_captain_preserves_conversations_and_drafts() {
    let mut state = State::new(".".into(), false);
    // Rosters start empty; these tests exercise the earlier named presets.
    state.captains = crate::captains::Captain::presets();
    state
        .current_mut()
        .draft
        .replace_text("Original thread draft");
    state.begin_new_captain();
    let editor = state.captain_editor.as_mut().unwrap();
    editor.name.replace_text("Atlas");
    editor.role.replace_text("Review");
    editor.instructions.replace_text("Cite failing tests.");
    editor.portrait = 2;
    state.save_captain();
    assert_eq!(state.captains.len(), 5);
    assert_eq!(state.current().captain, Some(4));
    assert_eq!(state.current().title, "Atlas");
    assert!(
        state.current().pi.is_none(),
        "creation must not start agent work"
    );
    assert_eq!(state.threads[0].draft.text, "Original thread draft");
    let home_id = state.current().id;
    state.new_thread();
    state.current_mut().draft.replace_text("Crew draft");
    let crew_id = state.current().id;
    assert_eq!(
        state.current().captain,
        None,
        "New thread creates a lone conversation"
    );
    state.edit_captain(4);
    state
        .captain_editor
        .as_mut()
        .unwrap()
        .name
        .replace_text("Atlas Review");
    state.save_captain();
    assert_eq!(state.current().id, home_id);
    assert_eq!(state.current().title, "Atlas Review");
    assert_eq!(
        state
            .threads
            .iter()
            .find(|t| t.id == crew_id)
            .unwrap()
            .draft
            .text,
        "Crew draft"
    );
    assert!(state.captains[4].prompt().contains("Cite failing tests."));
}

#[test]
fn concurrent_project_threads_keep_their_cwds_and_scratch_threads_are_isolated() {
    let mut state = State::new(".".into(), false);
    let a = state.thread_directory(0).unwrap();
    let b_id = state.add_project(std::env::temp_dir()).unwrap().unwrap();
    let b = state.active;
    let (a_commands, _a_events) = fake_pi(&mut state.threads[0], Status::Ready);
    let (b_commands, _b_events) = fake_pi(&mut state.threads[b], Status::Ready);
    state.active = 0;
    state.current_mut().draft.replace_text("Work in A");
    state.send();
    state.active = b;
    state.current_mut().draft.replace_text("Work in B");
    state.send();
    assert!(state.threads[0].status.busy() && state.threads[b].status.busy());
    assert_eq!(a_commands.try_recv().unwrap()["message"], "Work in A");
    assert_eq!(b_commands.try_recv().unwrap()["message"], "Work in B");
    assert_eq!(state.thread_directory(0).unwrap(), a);
    assert_eq!(
        state.thread_directory(b).unwrap(),
        state.project(Some(b_id)).unwrap().path
    );
    assert_ne!(
        state.thread_directory(0).unwrap(),
        state.thread_directory(b).unwrap()
    );
    state.current_mut().draft.replace_text("Scratch work");
    state.new_thread_in(None);
    let scratch = state.working_directory();
    let id = state.current().id;
    assert_eq!(scratch, state.data_dir.join("chats").join(id.to_string()));
    assert!(state.current().captain.is_none());
    state.current_mut().draft.replace_text("More scratch work");
    state.new_thread_in(None);
    assert_ne!(scratch, state.working_directory());
    state.current_mut().machine = "unavailable-host".into();
    assert!(state.thread_directory(state.active).is_err());
    assert!(crate::backend::BackendHost::resolve("local").is_ok());
    state.current_mut().machine = "local".into();
    let mut restored = State::new(PathBuf::new(), false);
    restored.restore_global(&state.snapshot()).unwrap();
    let index = restored.threads.iter().position(|t| t.id == id).unwrap();
    assert_eq!(restored.thread_directory(index).unwrap(), scratch);
}

#[test]
fn sidebar_is_pinned_then_recent_and_scopes_without_project_groups() {
    let mut state = State::new(".".into(), false);
    state.threads[0].updated = 10;
    state.threads[0].life.created_at = 10000;
    state.current_mut().draft.replace_text("First");
    state.new_thread();
    state.current_mut().updated = 30;
    state.current_mut().life.created_at = 30000;
    state.current_mut().draft.replace_text("Second");
    state.new_thread_in(None);
    state.current_mut().updated = 20;
    state.current_mut().life.created_at = 20000;
    state.current_mut().life.pinned_at = Some(1);
    for t in &mut state.threads {
        t.life.latest_user_message_at = Some(t.life.created_at);
    }
    let ids = |s: &State| {
        s.sidebar_threads()
            .iter()
            .map(|&i| s.threads[i].id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&state), vec![3, 2, 1]);
    state.project_scope = ProjectScope::Project(1);
    assert_eq!(ids(&state), vec![2, 1]);
    state.project_scope = ProjectScope::NoProject;
    assert_eq!(ids(&state), vec![3]);
    state.current_mut().life.archived_at = Some(1);
    assert!(ids(&state).is_empty());
    state.show_archive = true;
    assert_eq!(ids(&state), vec![3]);
}

#[test]
fn rename_and_confirmed_deletion_remove_children_not_project_files_or_other_threads() {
    let mut state = State::new(".".into(), false);
    state.begin_rename(1);
    state.rename_edit.replace_text("  A named thread  ");
    state.save_thread_name();
    assert_eq!(state.threads[0].title, "A named thread");
    state.new_thread_in(None);
    let survivor = state.current().id;
    state.active = 0;
    let (commands, _events) = fake_pi(&mut state.threads[0], Status::Running);
    let (index, child_commands, _events) = child(&mut state);
    let child_id = state.threads[index].id;
    state.tabs.push(child_id);
    let project_path = state.working_directory();
    let (reply, answer) = mpsc::channel();
    state.mcp_request(crate::mcp::Request {
        thread: 1,
        tool: "task_status".into(),
        args: json!({"thread_id":child_id,"wait_seconds":600}),
        reply,
    });
    state.request_delete(1);
    assert_eq!(
        state.threads.len(),
        3,
        "opening confirmation deletes nothing"
    );
    state.confirm_delete();
    assert_eq!(state.threads.len(), 1);
    assert_eq!(state.current().id, survivor);
    assert_eq!(state.tabs, vec![survivor]);
    assert_eq!(commands.try_recv().unwrap()["type"], "clear_queue");
    assert_eq!(commands.try_recv().unwrap()["type"], "abort");
    assert_eq!(child_commands.try_recv().unwrap()["type"], "clear_queue");
    assert!(answer.try_recv().unwrap().is_err());
    assert!(project_path.is_dir());
    let next = state.next_id;
    let mut restored = State::new(PathBuf::new(), false);
    restored.restore_global(&state.snapshot()).unwrap();
    assert_eq!(
        restored.next_id, next,
        "deleted ids must not be reused after restart"
    );
}

#[test]
fn image_only_prompts_send_typed_pi_images_with_ksg_path_context() {
    let mut thread = Thread::new(1);
    thread.harness = "pi".into();
    thread.model_provider = "cpa".into();
    thread.supports_images = Some(true);
    let (pi, commands, _events) = Pi::test_connection();
    thread.pi = Some(pi);
    let image = test_attachment("screen.png", 1);
    thread.add_attachment(image.clone()).unwrap();
    thread.send();
    let prompt = commands.try_recv().unwrap();
    assert_eq!(prompt["type"], "prompt");
    assert_eq!(
        prompt["message"],
        attachments::pi_message(&Prompt::new(String::new(), vec![image.clone()])).unwrap()
    );
    assert_eq!(prompt["images"][0]["type"], "image");
    assert_eq!(prompt["images"][0]["mimeType"], "image/png");
    assert_eq!(
        crate::attachments::decode_base64(prompt["images"][0]["data"].as_str().unwrap()).unwrap(),
        crate::attachments::read(&image).unwrap()
    );
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].text, "");
    assert_eq!(thread.messages[0].images, vec![image]);
    assert_eq!(thread.title, "Image: screen.png");
}

#[test]
fn non_vision_models_refuse_images_without_sending_or_clearing_the_draft() {
    let mut thread = Thread::new(1);
    thread.model = "Text Only".into();
    thread.model_provider = "cpa".into();
    thread.supports_images = Some(false);
    let (pi, commands, _events) = Pi::test_connection();
    thread.pi = Some(pi);
    let image = test_attachment("screen.png", 1);
    thread.add_attachment(image.clone()).unwrap();
    thread.draft.replace_text("Keep this text");
    thread.send();
    assert!(commands.try_recv().is_err(), "no prompt may be sent");
    assert!(thread.messages.is_empty());
    assert!(thread.queued_prompt.is_none());
    assert_eq!(thread.status, Status::Ready);
    assert_eq!(thread.draft.text, "Keep this text");
    assert_eq!(thread.draft_images, vec![image]);
    assert!(
        thread
            .attachment_error
            .as_ref()
            .unwrap()
            .contains("Text Only")
    );
}

#[test]
fn ordinary_file_drafts_round_trip_and_keep_missing_file_errors() {
    let path = crate::store::directory()
        .unwrap()
        .join("draft-reference.txt");
    std::fs::write(&path, "Draft text stays outside ui-state.").unwrap();
    let file = attachments::attach_file(&path).unwrap();
    let mut state = State::new(".".into(), false);
    state.current_mut().draft.replace_text("Review this");
    state.current_mut().add_attachment(file.clone()).unwrap();
    let snapshot = state.ui_snapshot();
    assert!(!snapshot.to_string().contains("Draft text stays outside"));
    let mut restored = State::new(".".into(), false);
    restored.restore_ui(&snapshot).unwrap();
    assert_eq!(restored.current().draft.text, "Review this");
    assert_eq!(restored.current().draft_images, vec![file.clone()]);
    std::fs::remove_file(path).unwrap();
    restored.restore_ui(&snapshot).unwrap();
    assert_eq!(restored.current().draft_images, vec![file]);
    assert!(
        restored
            .current()
            .attachment_error
            .as_deref()
            .unwrap()
            .contains("Missing file")
    );
    let again = restored.ui_snapshot();
    restored.restore_ui(&again).unwrap();
    assert!(
        restored
            .current()
            .attachment_error
            .as_deref()
            .unwrap()
            .contains("Missing file")
    );
}

#[test]
fn attachment_count_and_size_limits_keep_the_text_draft_untouched() {
    let mut thread = Thread::new(1);
    thread.draft.replace_text("Keep me");
    for width in 0..attachments::MAX_ATTACHMENTS {
        thread
            .add_attachment(test_attachment("image.png", width as u32 + 1))
            .unwrap();
    }
    let error = thread
        .add_attachment(test_attachment("extra.png", 120))
        .unwrap_err();
    assert!(error.contains("up to 100"));
    assert_eq!(thread.draft.text, "Keep me");
    thread.draft_images.clear();
    let mut oversized = test_attachment("ok.png", 20);
    oversized.bytes = crate::attachments::MAX_TOTAL_BYTES + 1;
    assert!(thread.add_attachment(oversized).is_err());
    assert_eq!(thread.draft.text, "Keep me");
}

#[test]
fn inline_file_and_contextual_image_chips_insert_at_caret_and_retain_records() {
    let dir = crate::store::directory().unwrap();
    let file_path = dir.join("spec.txt");
    std::fs::write(&file_path, "spec body").unwrap();
    let mut state = State::new(".".into(), false);
    state.current_mut().draft.replace_text("Read and reply");
    state.current_mut().draft.set_caret(4);
    state.attach_paths(&[file_path]);
    assert_eq!(state.current().draft_images.len(), 1);
    let file_id = state.current().draft_images[0].id.clone();
    let expected_file_ctx =
        crate::composer_context::to_kind_scoped_composer_context_id("file", &file_id);
    assert_eq!(state.current().draft.pills().count(), 1);
    assert_eq!(
        crate::composer_rich::markdown(&state.current().draft),
        format!("Read [spec.txt](t3-context://v1/file/{expected_file_ctx}) and reply")
    );
    assert!(
        state
            .current()
            .draft_context
            .iter()
            .any(|r| r.context_id == expected_file_ctx)
    );

    // Contextual image (when draft has prose text) inserts inline chip AND stays in tray.
    let end = state.current().draft.text.len();
    state.current_mut().draft.set_caret(end);
    state.attach_image("shot.png", &test_png(1));
    assert_eq!(state.current().draft_images.len(), 2);
    let img_id = state.current().draft_images[1].id.clone();
    let expected_img_ctx =
        crate::composer_context::to_kind_scoped_composer_context_id("image", &img_id);
    assert_eq!(state.current().draft.pills().count(), 2);
    assert!(
        crate::composer_rich::markdown(&state.current().draft).contains(&format!(
            "![shot.png](t3-context://v1/image/{expected_img_ctx})"
        ))
    );

    // Undo removes the inline image chip (image stays in tray); deleting the file chip
    // moves the file attachment into retained_context.
    state.current_mut().draft.undo();
    state.current_mut().reconcile_draft_context();
    assert_eq!(state.current().draft_images.len(), 2);
    assert_eq!(state.current().draft.pills().count(), 1);

    state.current_mut().draft.undo();
    state.current_mut().reconcile_draft_context();
    assert_eq!(state.current().draft.pills().count(), 0);
    assert_eq!(state.current().draft_images.len(), 1); // only image remains!
    assert!(
        state
            .current()
            .retained_context
            .files
            .contains_key(&expected_file_ctx)
    );

    // Redo brings both the file chip and backing file attachment back.
    state.current_mut().draft.redo();
    state.current_mut().reconcile_draft_context();
    assert_eq!(state.current().draft.pills().count(), 1);
    assert_eq!(state.current().draft_images.len(), 2);
}

#[test]
fn send_background_starts_draft_in_background_inherits_composer_and_notifies() {
    let mut state = State::new(".".into(), false);
    let dir_b = std::env::temp_dir().join(format!("shina-bg-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir_b);
    let p2 = state.add_project(dir_b.clone()).unwrap().unwrap();
    let model = AgentModel {
        provider: "codex".into(),
        id: "codex-fast".into(),
        name: "Codex Fast".into(),
        thinking_levels: vec!["low".into(), "high".into()],
        context_window: Some(100_000),
        supports_images: Some(true),
        service_tiers: vec![
            json!({"id":"default","name":"Standard"}),
            json!({"id":"fast","name":"Fast"}),
        ],
        ..Default::default()
    };
    state.codex_catalog.models = vec![model.clone()];
    let (pi, _cmds, _events) = Pi::test_connection();
    {
        let t = state.current_mut();
        t.harness = "codex".into();
        t.models = vec![model];
        t.set_model("codex".into(), "codex-fast".into());
        t.set_thinking("high".into());
        t.set_service_tier("fast".into());
        state.set_access_mode(AccessMode::ReadOnly);
        let t = state.current_mut();
        t.pi = Some(pi);
        t.draft.replace_text("Background task");
        t.add_attachment(test_attachment("shot.png", 1)).unwrap();
    }
    let bg_id = state.current().id;

    state.send_background();

    // Fresh draft composer is active, focused, empty, and inherits selections.
    assert_ne!(state.current().id, bg_id);
    assert!(state.current().is_draft());
    assert!(state.current().draft.focused);
    assert!(state.current().draft.text.is_empty());
    assert!(state.current().draft_images.is_empty());
    assert!(state.current().messages.is_empty());
    assert!(state.current().error.is_none());
    assert_eq!(state.current().project, Some(p2));
    assert_eq!(state.current().machine, "local");
    assert_eq!(state.current().harness, "codex");
    assert_eq!(
        state.current().selected_model,
        Some(("codex".into(), "codex-fast".into()))
    );
    assert_eq!(state.current().selected_thinking.as_deref(), Some("high"));
    assert_eq!(state.current().harness_state["serviceTier"], "fast");
    assert_eq!(state.current().native.access, AccessMode::ReadOnly);
    assert!(state.current().captain.is_none());

    // Background thread is running and visible in sidebar Active section.
    let bg = state.threads.iter().find(|t| t.id == bg_id).unwrap();
    assert_eq!(bg.status, Status::Running);
    assert_eq!(bg.messages.len(), 1);
    assert_eq!(bg.messages[0].text, "Background task");
    assert!(!bg.is_draft());
    assert!(
        state
            .sidebar_threads()
            .iter()
            .any(|&i| state.threads[i].id == bg_id)
    );

    // 5-second "Started in background" toast with "Open" action.
    assert_eq!(state.toasts.len(), 1);
    let toast = &state.toasts[0];
    assert_eq!(toast.kind, ToastKind::Success);
    assert_eq!(toast.title, "Started in background");
    assert_eq!(toast.action_label.as_deref(), Some("Open"));
    assert!(toast.expires_at.is_some());

    // Completing in background marks the thread unread; clicking Open navigates to it and clears unread.
    let bg_idx = state.threads.iter().position(|t| t.id == bg_id).unwrap();
    state.threads[bg_idx].apply(json!({"type":"agent_settled"}));
    let completed_at = state.threads[bg_idx].life.last_completed_at.unwrap();
    state.threads[bg_idx].life.last_visited_at = Some(completed_at.saturating_sub(100));
    assert!(state.threads[bg_idx].unread());

    let toast_id = toast.id;
    state.activate_toast(toast_id);
    assert_eq!(state.current().id, bg_id);
    assert!(state.toasts.is_empty());
    assert!(!state.current().unread());
    let _ = std::fs::remove_dir_all(&dir_b);
}

#[test]
fn failed_background_send_recovers_draft_sets_error_and_offers_open_draft_toast() {
    let mut state = State::new(".".into(), false);
    state.runtime_enabled = true;
    let bg_id = state.current().id;
    let img = test_attachment("fail.png", 1);
    state.current_mut().model_provider = "cpa".into();
    state.current_mut().supports_images = Some(true);
    state
        .current_mut()
        .draft
        .replace_text("Will fail on connect");
    state.current_mut().add_attachment(img.clone()).unwrap();
    // Force synchronous connect_thread failure via missing directory on unknown machine.
    state.current_mut().machine = "missing-host".into();

    state.send_background();

    // User landed on a fresh composer.
    assert_ne!(state.current().id, bg_id);
    assert!(state.current().is_draft());
    assert!(state.current().draft.focused);

    // Background task failed toast with "Open draft" action is shown.
    assert_eq!(state.toasts.len(), 1);
    let toast = &state.toasts[0];
    assert_eq!(toast.kind, ToastKind::Error);
    assert_eq!(toast.title, "Background task failed");
    assert!(toast.description.is_some());
    assert_eq!(toast.action_label.as_deref(), Some("Open draft"));

    // Clicking "Open draft" opens the failed draft with restored text, attachments, and error.
    let toast_id = toast.id;
    state.activate_toast(toast_id);
    assert_eq!(state.current().id, bg_id);
    assert_eq!(state.current().draft.text, "Will fail on connect");
    assert_eq!(state.current().draft_images, vec![img]);
    assert!(state.current().error.is_some());

    // Async failure during startup in poll() replaces "Started in background" with "Background task failed".
    let mut async_state = State::new(".".into(), false);
    let (pi, _cmds, events) = Pi::test_connection();
    let async_bg_id = async_state.current().id;
    async_state.current_mut().pi = Some(pi);
    async_state.current_mut().initializing = true;
    async_state.current_mut().model_provider = "cpa".into();
    async_state
        .current_mut()
        .draft
        .replace_text("Async failure");
    async_state.send_background();
    assert_eq!(async_state.toasts[0].title, "Started in background");

    events.send(Event::Error("Process crashed".into())).unwrap();
    poll(&mut async_state, 0.0, &mut Tx::default());

    assert_eq!(async_state.toasts.len(), 1);
    assert_eq!(async_state.toasts[0].kind, ToastKind::Error);
    assert_eq!(async_state.toasts[0].title, "Background task failed");
    assert_eq!(
        async_state.toasts[0].description.as_deref(),
        Some("Process crashed")
    );
    assert_eq!(
        async_state.toasts[0].action_label.as_deref(),
        Some("Open draft")
    );
    let bg_after = async_state
        .threads
        .iter()
        .find(|t| t.id == async_bg_id)
        .unwrap();
    assert_eq!(bg_after.draft.text, "Async failure");
    assert_eq!(bg_after.error.as_deref(), Some("Process crashed"));

    // Dismiss and 5s timeout expiration remove toasts cleanly.
    async_state.toasts[0].expires_at = Some(1);
    poll(&mut async_state, 0.0, &mut Tx::default());
    assert!(async_state.toasts.is_empty());
    async_state.notify_background_started("local", async_bg_id);
    assert_eq!(async_state.toasts.len(), 1);
    let dismiss_id = async_state.toasts[0].id;
    async_state.dismiss_toast(dismiss_id);
    assert!(async_state.toasts.is_empty());
}

#[test]
fn clear_command_arms_when_alone_sends_text_with_attachment_and_unavailable_with_approval() {
    let mut state = State::new(".".into(), false);
    // Empty thread: /clear is unavailable.
    state.current_mut().draft.replace_text("/");
    state.current_mut().draft.focused = true;
    assert!(!state.slash_matches().iter().any(|c| c.name == "clear"));
    state.current_mut().draft.replace_text("/clear");
    state.send();
    assert!(state.current().fresh_session_arm.is_none());
    assert_eq!(state.current().messages.len(), 1);
    assert_eq!(state.current().messages[0].text, "/clear");

    // Thread with history: /clear alone arms, clears draft, and sends nothing.
    state.current_mut().status = Status::Completed;
    state.current_mut().draft.replace_text("/");
    assert!(state.slash_matches().iter().any(|c| c.name == "clear"));
    state.current_mut().draft.replace_text("/clear");
    state.send();
    assert!(state.current().fresh_session_arm.is_some());
    assert!(state.current().draft.text.is_empty());
    assert_eq!(state.current().messages.len(), 1);
    assert!(
        state
            .composer_notices
            .ordered()
            .iter()
            .any(|n| n.id == "fresh-session")
    );

    // With an attachment present, /clear is sent as ordinary text and does not arm.
    state.disarm_fresh_session(state.current().id, None);
    state.current_mut().draft.replace_text("/clear");
    state
        .current_mut()
        .add_attachment(test_attachment("note.png", 1))
        .unwrap();
    state.send();
    assert!(state.current().fresh_session_arm.is_none());
    assert_eq!(state.current().messages.len(), 2);
    assert_eq!(state.current().messages[1].text, "/clear");

    // Unavailable with a pending approval.
    state.current_mut().status = Status::Waiting;
    state.current_mut().request = Some(json!({"id":"perm-1","method":"confirm"}));
    state.current_mut().draft.replace_text("/");
    assert!(!state.slash_matches().iter().any(|c| c.name == "clear"));
    state.current_mut().draft.replace_text("/clear");
    state.send();
    assert!(state.current().fresh_session_arm.is_none());
    assert_eq!(state.current().messages.len(), 2);

    // Unavailable on a linked machine lacking the `fresh-context` capability.
    state.current_mut().status = Status::Completed;
    state.current_mut().request = None;
    let (old_client, _) = crate::client::Client::test_connected(state.data_dir.clone());
    *old_client.info.lock().unwrap() = json!({"capabilities": []});
    state.client = Some(old_client);
    state.current_mut().draft.replace_text("/");
    assert!(!state.slash_matches().iter().any(|c| c.name == "clear"));
}

#[test]
fn fresh_session_arm_is_read_once_keeps_on_failed_send_and_persists_with_draft() {
    let dir = crate::store::directory().unwrap().join("fresh-arm-persist");
    std::fs::create_dir_all(&dir).unwrap();

    let mut state = State::new(dir.clone(), false);
    state.load_global_for_test();
    state.persist_transcripts = true;
    state.current_mut().messages.push(Message {
        id: "m1".into(),
        role: "You",
        text: "Earlier".into(),
        images: vec![],
        context: None,
    });
    state.current_mut().status = Status::Completed;
    state.current_mut().draft.replace_text("/clear");
    state.send();
    let first_arm = state.current().fresh_session_arm.expect("armed");

    // Persist draft and reload in a fresh State against the same temp directory.
    state.current_mut().draft.replace_text("Carry over");
    state.checkpoint_and_flush().unwrap();
    drop(state);

    let mut reloaded = State::new(dir, false);
    reloaded.load_global_for_test();
    assert_eq!(reloaded.current().draft.text, "Carry over");
    assert_eq!(reloaded.current().fresh_session_arm, Some(first_arm));
    assert!(
        reloaded
            .composer_notices
            .ordered()
            .iter()
            .any(|n| n.id == "fresh-session")
    );

    // Failed local send (e.g., Pi with non-CPA model) keeps the arm.
    reloaded.current_mut().harness = "pi".into();
    reloaded.current_mut().model_provider = "anthropic".into();
    reloaded.send();
    assert_eq!(reloaded.current().fresh_session_arm, Some(first_arm));
    assert_eq!(reloaded.current().draft.text, "Carry over");

    // Accepted local send clears the arm, keeps visible transcript, and marks the message.
    reloaded.current_mut().model_provider = "cpa".into();
    reloaded.send();
    assert!(reloaded.current().fresh_session_arm.is_none());
    assert_eq!(reloaded.current().messages.len(), 2);
    assert_eq!(reloaded.current().messages[0].text, "Earlier");
    assert_eq!(reloaded.current().messages[1].text, "Carry over");
    assert!(
        reloaded
            .current()
            .is_fresh_session_message(&reloaded.current().messages[1].id)
    );

    // Read-once semantics for in-flight remote sends: re-arming during flight stays armed.
    let (client, mut rx_cmd) = crate::client::Client::test_connected(reloaded.data_dir.clone());
    *client.info.lock().unwrap() = json!({"capabilities": [crate::protocol::FRESH_CONTEXT]});
    let commands = client.commands.clone();
    let mut remote_state = State::new(reloaded.data_dir.clone(), false);
    remote_state.client = Some(client);
    remote_state.current_mut().remote = Some(commands);
    remote_state.current_mut().messages.push(Message {
        id: "rm1".into(),
        role: "You",
        text: "First".into(),
        images: vec![],
        context: None,
    });
    remote_state.current_mut().draft.replace_text("/clear");
    remote_state.send();
    let inflight_arm = remote_state.current().fresh_session_arm.unwrap();
    remote_state.current_mut().draft.replace_text("Second");
    assert!(remote_state.dispatch_send());
    let sent = rx_cmd.try_recv().unwrap();
    assert_eq!(sent.params["freshContext"], true);
    let cmd_id = sent.command_id;

    // User re-arms while first send is in flight.
    remote_state.current_mut().draft.replace_text("/clear");
    remote_state.send();
    let newer_arm = remote_state.current().fresh_session_arm.unwrap();
    assert_ne!(newer_arm, inflight_arm);

    // First send acceptance must not clear the newer arm.
    assert!(remote_state.current_mut().finish_remote_send(cmd_id, true));
    assert_eq!(remote_state.current().fresh_session_arm, Some(newer_arm));

    // A failed remote send keeps its arm.
    remote_state.current_mut().draft.replace_text("Third");
    assert!(remote_state.dispatch_send());
    let failed_cmd = rx_cmd.try_recv().unwrap().command_id;
    assert!(
        remote_state
            .current_mut()
            .finish_remote_send(failed_cmd, false)
    );
    assert_eq!(remote_state.current().fresh_session_arm, Some(newer_arm));
}

#[test]
fn compact_before_send_queues_behind_compaction_respects_keep_and_skips_slash_compact() {
    let stale_at = lifecycle::now().saturating_sub(80 * 60_000);
    let setup_stale_claude = |state: &mut State| {
        let (pi, commands, _events) = Pi::test_connection();
        let t = state.current_mut();
        t.pi = Some(pi);
        t.harness = "claude".into();
        t.model_provider = "claude".into();
        t.session = Some("11111111-1111-4111-8111-111111111111".into());
        t.status = Status::Completed;
        t.messages.push(Message {
            id: "u1".into(),
            role: "You",
            text: "Earlier work".into(),
            images: vec![],
            context: None,
        });
        t.context.absorb(&json!({
            "contextUsage": {
                "tokens": 220_000,
                "contextWindow": 1_000_000,
                "updatedAt": stale_at
            }
        }));
        commands
    };

    // 1. Default "Compact" queues the message behind compaction and sends after compaction ends.
    let mut state = State::new(".".into(), false);
    let commands = setup_stale_claude(&mut state);
    assert_eq!(
        state.resume_compaction_tokens(state.current(), lifecycle::now()),
        Some(220_000)
    );
    state.current_mut().draft.replace_text("Follow up");
    state.send();
    assert_eq!(state.current().native.queue.len(), 1);
    assert_eq!(state.current().native.queue[0].prompt.text, "Follow up");
    let compact_cmd = commands.try_recv().unwrap();
    assert_eq!(compact_cmd["type"], "compact");
    let compact_id = compact_cmd["id"].as_str().unwrap().to_owned();
    assert!(
        commands.try_recv().is_err(),
        "prompt must wait for compaction"
    );

    // Compaction completes -> queued message begins its prompt.
    state.current_mut().apply(json!({"type":"compaction_end"}));
    state.current_mut().apply(json!({
        "type":"response","id":compact_id,"command":"compact","success":true,"data":{}
    }));
    let probe = commands
        .try_iter()
        .find(|c| c["type"] == "get_state")
        .unwrap();
    state.current_mut().apply(json!({
        "type":"response","id":probe["id"],"command":"get_state","success":true,"data":{"isStreaming":false,"isCompacting":false}
    }));
    assert!(state.current().native.queue.is_empty());
    let sent_prompt = commands
        .try_iter()
        .find(|c| c["type"] == "prompt")
        .expect("queued prompt sent after compaction");
    assert_eq!(sent_prompt["message"], "Follow up");

    // 2. If compaction fails, the message stays queued and the failure is shown.
    let mut fail_state = State::new(".".into(), false);
    let commands = setup_stale_claude(&mut fail_state);
    fail_state
        .current_mut()
        .draft
        .replace_text("Wait for compact");
    fail_state.send();
    let compact_cmd = commands.try_recv().unwrap();
    assert_eq!(compact_cmd["type"], "compact");
    let compact_id = compact_cmd["id"].as_str().unwrap().to_owned();
    fail_state.current_mut().apply(json!({
        "type":"compaction_end","errorMessage":"Compaction failed"
    }));
    fail_state.current_mut().apply(json!({
        "type":"response","id":compact_id,"command":"compact","success":false,"error":"Compaction failed"
    }));
    let probe = commands
        .try_iter()
        .find(|c| c["type"] == "get_state")
        .unwrap();
    fail_state.current_mut().apply(json!({
        "type":"response","id":probe["id"],"command":"get_state","success":true,"data":{"isStreaming":false,"isCompacting":false}
    }));
    assert_eq!(fail_state.current().native.queue.len(), 1);
    assert!(fail_state.current().native.queue_held);
    assert_eq!(
        fail_state.current().error.as_deref(),
        Some("Compaction failed")
    );
    assert!(
        !commands.try_iter().any(|c| c["type"] == "prompt"),
        "failed compaction must not send queued prompt"
    );

    // 3. "Keep" sends directly without compacting and resets after sending.
    let mut keep_state = State::new(".".into(), false);
    let commands = setup_stale_claude(&mut keep_state);
    keep_state.toggle_keep_full_history();
    assert!(keep_state.keep_full_history(keep_state.current()));
    keep_state.current_mut().draft.replace_text("Direct send");
    keep_state.send();
    assert!(keep_state.current().native.queue.is_empty());
    assert!(!keep_state.keep_full_history(keep_state.current()));
    let sent_cmds: Vec<_> = commands.try_iter().collect();
    assert!(!sent_cmds.iter().any(|c| c["type"] == "compact"));
    let sent = sent_cmds
        .iter()
        .find(|c| c["type"] == "prompt")
        .expect("direct prompt");
    assert_eq!(sent["message"], "Direct send");

    // 4. Typing `/compact` does not double-compact.
    let mut cmd_state = State::new(".".into(), false);
    let commands = setup_stale_claude(&mut cmd_state);
    cmd_state
        .current_mut()
        .draft
        .replace_text("/compact focus on tests");
    cmd_state.send();
    assert!(cmd_state.current().native.queue.is_empty());
    let compact = commands.try_recv().unwrap();
    assert_eq!(compact["type"], "compact");
    assert_eq!(compact["customInstructions"], "focus on tests");
    assert!(commands.try_recv().is_err());
}

#[test]
fn prompt_suggestion_show_rule_conditions() {
    let mut state = State::new(".".into(), false);
    state.thread_defaults.prompt_suggestions_enabled = true;
    let t = state.current_mut();
    t.harness = "claude".into();
    t.status = Status::Completed;
    t.run_id = Some("run-1".into());
    t.prompt_suggestion = Some(PromptSuggestion {
        run_id: "run-1".into(),
        text: "Run cargo test".into(),
    });

    // 1. Shows on empty draft after completion.
    assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

    // 2. Hidden with text, shows again after deleting back to empty.
    state.current_mut().draft.replace_text("hi");
    assert_eq!(state.offered_prompt_suggestion(), None);
    state.current_mut().draft.replace_text("");
    assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

    // 3. Hidden with an attachment.
    state.current_mut().draft_images.push(Attachment {
        id: "att-1".into(),
        name: "a.png".into(),
        mime: "image/png".into(),
        width: 1,
        height: 1,
        bytes: 3,
        path: None,
        asset_id: None,
        uploaded: None,
        source: None,
    });
    assert_eq!(state.offered_prompt_suggestion(), None);
    state.current_mut().draft_images.clear();
    assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

    // 4. Hidden with a pending approval.
    state.current_mut().request = Some(json!({"id": "req-1", "method": "confirm"}));
    assert_eq!(state.offered_prompt_suggestion(), None);
    state.current_mut().request = None;
    assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

    // 5. Hidden while running.
    state.current_mut().status = Status::Running;
    assert_eq!(state.offered_prompt_suggestion(), None);
    state.current_mut().status = Status::Completed;
    assert_eq!(state.offered_prompt_suggestion(), Some("Run cargo test"));

    // 6. Hidden after dismissal for this run.
    assert!(state.dismiss_prompt_suggestion());
    assert_eq!(state.offered_prompt_suggestion(), None);

    // 7. Shows again for the next run.
    let t = state.current_mut();
    t.run_id = Some("run-2".into());
    t.prompt_suggestion = Some(PromptSuggestion {
        run_id: "run-2".into(),
        text: "Inspect the diff".into(),
    });
    assert_eq!(state.offered_prompt_suggestion(), Some("Inspect the diff"));
}

#[test]
fn usage_limits_command_raises_notice_without_sending_and_requires_capability() {
    let mut state = State::new(".".into(), false);
    state.current_mut().harness = "pi".into();
    // Pi has no report, so `/usage-limits` is not offered and sends as ordinary text.
    state.current_mut().draft.focused = true;
    state.current_mut().draft.replace_text("/");
    assert!(
        !state
            .slash_matches()
            .iter()
            .any(|c| c.name == crate::usage_limits::USAGE_LIMITS_COMMAND_NAME)
    );

    let (pi, commands, _events) = Pi::test_connection();
    {
        let t = state.current_mut();
        t.harness = "codex".into();
        t.model_provider = "codex".into();
        t.pi = Some(pi);
        t.harness_state["usageLimitsRaw"] = json!({
            "checkedAtMs": 1_700_000_000_000i64,
            "snapshot": {
                "limitId": "codex",
                "planType": "plus",
                "primary": {"usedPercent": 35.0, "windowDurationMins": 300, "resetsAt": 1_700_003_600}
            }
        });
        t.draft.replace_text("/");
    }
    assert!(
        state
            .slash_matches()
            .iter()
            .any(|c| c.name == crate::usage_limits::USAGE_LIMITS_COMMAND_NAME)
    );

    // Sending `/usage-limits` alone raises the notice, clears the draft, and sends nothing.
    state.current_mut().draft.replace_text("/usage-limits");
    state.send();
    assert!(state.current().draft.text.is_empty());
    assert!(commands.try_recv().is_err());
    let notices = state.composer_notices.ordered();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].id, "usage-limits");
    assert_eq!(notices[0].title, "Usage limits");
    assert!(matches!(
        notices[0].body,
        Some(crate::composer_notices::NoticeBody::UsageLimits(_))
    ));

    // Dismissing removes it; choosing from the slash menu raises it again without sending.
    state.dismiss_composer_notice("usage-limits");
    assert!(state.composer_notices.ordered().is_empty());
    state.current_mut().draft.replace_text("/usage");
    state.choose_slash(crate::usage_limits::USAGE_LIMITS_COMMAND_NAME.into());
    assert!(state.current().draft.text.is_empty());
    assert!(commands.try_recv().is_err());
    assert_eq!(state.composer_notices.ordered().len(), 1);

    // Sending `/usage-limits` with extra text sends as ordinary text (KSG `isUsageLimitsCommand`).
    state
        .current_mut()
        .draft
        .replace_text("/usage-limits explain");
    state.send();
    let sent = commands.try_recv().expect("ordinary prompt sent");
    assert_eq!(sent["type"], "prompt");
    assert_eq!(sent["message"], "/usage-limits explain");

    // Machine lacking `usage-limits` capability hides the command.
    let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
    *client.info.lock().unwrap() = json!({"capabilities": []});
    state.client = Some(client);
    state.current_mut().draft.focused = true;
    state.current_mut().draft.replace_text("/");
    assert!(
        !state
            .slash_matches()
            .iter()
            .any(|c| c.name == crate::usage_limits::USAGE_LIMITS_COMMAND_NAME)
    );
}

#[test]
fn fallback_model_switches_when_out_of_usage_restores_on_reset_and_use_default_clears_notice() {
    let mut state = State::new(".".into(), false);
    state.default_harness = "codex".into();
    state.codex_catalog.models.push(
        AgentModel::from_harness_value(
            &json!({
                "provider": "codex",
                "id": "gpt-6-astra",
                "name": "GPT-6 Astra",
                "isDefault": true,
                "thinkingLevels": ["medium"]
            }),
            "codex",
        )
        .unwrap(),
    );
    state.claude_catalog.models.push(
        AgentModel::from_harness_value(
            &json!({
                "provider": "claude",
                "id": "claude-opus-5-5",
                "name": "Claude Opus 5.5",
                "isDefault": true,
                "thinkingLevels": ["high"]
            }),
            "claude",
        )
        .unwrap(),
    );
    state.claude_catalog.installed = true;
    state.thread_defaults.fallback_model_selection = Some(crate::model_selection::ModelSelection {
        instance_id: "claude".into(),
        model: "claude-opus-5-5".into(),
        options: vec![],
    });
    let now_ms = 1_700_000_000_000i64;
    // Codex primary window is exhausted (100% used, resets 1h from now_ms).
    state.codex_catalog.usage_limits_raw = json!({
        "checkedAtMs": now_ms,
        "snapshot": {
            "limitId": "codex",
            "planType": "plus",
            "primary": {"usedPercent": 100.0, "windowDurationMins": 300, "resetsAt": 1_700_003_600}
        }
    });

    state.sync_fallback_model(now_ms);
    assert_eq!(state.current().harness, "claude");
    assert_eq!(state.current().model_id, "claude-opus-5-5");
    let notices = state.composer_notices.ordered();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].id, "fallback-model");
    assert_eq!(notices[0].title, "Using Claude Opus 5.5");
    assert!(
        notices[0]
            .description
            .as_deref()
            .unwrap()
            .starts_with("Codex is out of usage · resets ")
    );
    assert_eq!(notices[0].actions.len(), 1);
    assert_eq!(notices[0].actions[0].label, "Use GPT-6 Astra");
    assert_eq!(
        notices[0].dismiss_label.as_deref(),
        Some("Dismiss fallback model notice")
    );

    // Dismissing keeps the notice hidden while the fallback state continues.
    state.dismiss_composer_notice("fallback-model");
    assert!(state.composer_notices.ordered().is_empty());
    state.sync_fallback_model(now_ms + 60_000);
    assert!(state.composer_notices.ordered().is_empty());

    // When usage resets (now passes resetsAt), the thread restores its default model and
    // clears dismissal so a later exhaustion can show the notice again.
    state.sync_fallback_model(1_700_003_601_000);
    assert_eq!(state.current().harness, "codex");
    assert_eq!(state.current().model_id, "gpt-6-astra");
    assert!(state.composer_notices.ordered().is_empty());

    // Out of usage again -> switches and re-raises notice; clicking "Use default" switches back and removes notice.
    state.codex_catalog.usage_limits_raw = json!({
        "checkedAtMs": 1_700_010_000_000i64,
        "snapshot": {
            "limitId": "codex",
            "planType": "plus",
            "primary": {"usedPercent": 100.0, "windowDurationMins": 300, "resetsAt": 1_700_020_000}
        }
    });
    state.sync_fallback_model(1_700_010_000_000);
    assert_eq!(state.current().harness, "claude");
    let action = state.composer_notices.ordered()[0].actions[0]
        .handler
        .clone();
    action.call(&mut state);
    assert_eq!(state.current().harness, "codex");
    assert_eq!(state.current().model_id, "gpt-6-astra");
    assert!(state.composer_notices.ordered().is_empty());
    // Explicit override persists while staying on that thread.
    state.sync_fallback_model(1_700_010_060_000);
    assert_eq!(state.current().harness, "codex");
    assert!(state.composer_notices.ordered().is_empty());
}
