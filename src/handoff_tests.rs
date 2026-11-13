use super::*;

fn codex_thread(state: &mut State) {
    let t = state.current_mut();
    t.harness = "codex".into();
    t.model_provider = "codex".into();
    t.model_id = "gpt-5.2".into();
    t.model = "GPT-5.2".into();
    t.selected_model = Some(("codex".into(), "gpt-5.2".into()));
    t.session = Some("codex-session-1".into());
    t.messages.push(Message {
        id: "u1".into(),
        role: "You",
        text: "First request.".into(),
        images: vec![],
        context: None,
    });
    t.messages.push(Message {
        id: "a1".into(),
        role: "Codex",
        text: "First answer.".into(),
        images: vec![],
        context: None,
    });
    t.tools.push(Tool {
        id: "t1".into(),
        name: "bash".into(),
        kind: None,
        changes: vec![],
        input: json!({}),
        detail: "ls".into(),
        done: true,
        failed: false,
        output: "file.rs".into(),
        images: vec![],
        structured: json!({}),
        parent_call: None,
        exit_code: Some(0),
        subagents: vec![],
        anchor: ToolAnchor {
            message: 1,
            offset: 0,
        },
        block: 0,
        order: 0,
        expanded: false,
        raw_expanded: false,
        group_expanded: false,
        started: None,
        duration_ms: None,
    });
    t.status = Status::Ready;
}

fn fake_state() -> (
    State,
    std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Receiver<Value>>>>,
) {
    let dir = std::env::temp_dir().join(format!("shina-handoff-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = State::new(dir.join("workspace"), false);
    state.data_dir = dir.join("data");
    std::fs::create_dir_all(&state.data_dir).unwrap();
    state.runtime_enabled = true;
    let channels: std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Receiver<Value>>>> =
        std::sync::Arc::default();
    let hook_channels = channels.clone();
    state.test_connect_pi = Some(Box::new(move |_tid, _path| {
        let (pi, commands, _events) = Pi::test_connection();
        hook_channels.lock().unwrap().push(commands);
        Some(pi)
    }));
    (state, channels)
}

fn take_commands(
    channels: &std::sync::Arc<std::sync::Mutex<Vec<std::sync::mpsc::Receiver<Value>>>>,
    index: usize,
) -> Vec<Value> {
    let mut guard = channels.lock().unwrap();
    let rx = guard.remove(index);
    drop(guard);
    rx.try_iter().collect()
}

fn prompt_accept(state: &mut State, command: &Value) {
    state.current_mut().apply(json!({
        "type": "response",
        "id": command["id"],
        "command": "prompt",
        "success": true,
        "data": {}
    }));
}

/// Codex to Claude on a thread with history: a new Claude session starts (not
/// resumed), the package precedes the user's message exactly once, the record
/// becomes delivered, and the Codex session id is still stored.
#[test]
fn switch_codex_to_claude_delivers_package_and_keeps_source_session() {
    let (mut state, channels) = fake_state();
    codex_thread(&mut state);
    let thread_id = state.current().id;

    state
        .current_mut()
        .set_model("claude".into(), "claude-opus-4-5".into());
    assert_eq!(state.current().handoffs.len(), 1);
    let staged = state.current().staged_handoff().expect("handoff staged");
    assert_eq!(staged.from_instances, vec!["codex"]);
    assert_eq!(staged.to_instance, "claude");
    assert_eq!(staged.status, crate::handoff::HandoffStatus::Pending);
    assert!(state.current().error.is_none());

    state
        .current_mut()
        .draft
        .replace_text("Keep going on the fix.");
    state.send();
    let t = state.current();
    assert_eq!(t.harness, "claude");
    assert_eq!(t.model_id, "claude-opus-4-5");
    assert_eq!(
        t.handoff_sessions.get("codex").map(String::as_str),
        Some("codex-session-1")
    );
    assert!(t.session.is_none(), "claude has no retained session yet");

    // The recorded connect used the destination harness with no resume id.
    assert_eq!(
        state.test_connect_sessions,
        vec![(thread_id, "claude".into(), None)]
    );

    // The package rides inline in the same prompt record, once, ahead of the text.
    let commands = take_commands(&channels, 0);
    let prompt = commands
        .iter()
        .find(|c| c["type"] == "prompt")
        .expect("prompt dispatched");
    let message = prompt["message"].as_str().unwrap();
    assert!(
        message.starts_with("Provider context handoff."),
        "{message}"
    );
    assert!(message.contains("item=u1"), "{message}");
    assert!(message.contains("item=t1"), "{message}");
    assert!(message.ends_with("\n\nUser message:\nKeep going on the fix."));
    assert_eq!(message.matches("User message:").count(), 1);
    assert_eq!(
        state.current().handoffs[0].status,
        crate::handoff::HandoffStatus::Pending
    );

    // Acceptance marks the record delivered.
    prompt_accept(&mut state, prompt);
    assert_eq!(
        state.current().handoffs[0].status,
        crate::handoff::HandoffStatus::Ready
    );
    assert_eq!(
        state.current().handoffs[0].delivery.status,
        crate::handoff::DeliveryStatus::Inline
    );
}

/// Switching back resumes the retained Codex session and delivers only the
/// incremental package for what Codex has not seen.
#[test]
fn switch_back_resumes_retained_session_with_incremental_package() {
    let (mut state, channels) = fake_state();
    codex_thread(&mut state);
    let thread_id = state.current().id;

    state
        .current_mut()
        .set_model("claude".into(), "claude-opus-4-5".into());
    state
        .current_mut()
        .draft
        .replace_text("Keep going on the fix.");
    state.send();
    let commands = take_commands(&channels, 0);
    let prompt = commands
        .iter()
        .find(|c| c["type"] == "prompt")
        .expect("first prompt")
        .clone();
    prompt_accept(&mut state, &prompt);
    state
        .current_mut()
        .apply(json!({"type":"agent_settled","aborted":false}));
    // Claude's answer lands in the transcript after the switch.
    state.current_mut().messages.push(Message {
        id: "a2".into(),
        role: "Pi",
        text: "Claude answer.".into(),
        images: vec![],
        context: None,
    });
    state.current_mut().session = Some("claude-session-9".into());
    state.current_mut().status = Status::Ready;

    state
        .current_mut()
        .set_model("codex".into(), "gpt-5.2".into());
    assert_eq!(state.current().handoffs.len(), 2);
    state
        .current_mut()
        .draft
        .replace_text("Back to the original plan.");
    state.send();

    let t = state.current();
    assert_eq!(t.harness, "codex");
    assert_eq!(
        t.handoff_sessions.get("claude").map(String::as_str),
        Some("claude-session-9")
    );
    assert_eq!(t.session.as_deref(), Some("codex-session-1"));

    // The connect resumes the retained codex session.
    assert_eq!(
        state.test_connect_sessions.last().unwrap(),
        &(thread_id, "codex".into(), Some("codex-session-1".into()))
    );

    // The delta covers only what codex never saw: the user turn on claude and
    // claude's answer, not u1/a1/t1.
    let commands = take_commands(&channels, 0);
    let prompt = commands
        .iter()
        .find(|c| c["type"] == "prompt")
        .expect("second prompt");
    let message = prompt["message"].as_str().unwrap();
    assert!(
        message.starts_with("Provider context handoff."),
        "{message}"
    );
    assert!(!message.contains("item=u1"), "{message}");
    assert!(!message.contains("item=a1"), "{message}");
    assert!(!message.contains("item=t1"), "{message}");
    assert!(message.contains("item=a2"), "{message}");
    assert!(message.contains("Keep going on the fix."), "{message}");
    assert!(message.ends_with("\n\nUser message:\nBack to the original plan."));
    assert_eq!(
        state.current().handoffs[1].strategy,
        crate::handoff::HandoffStrategy::Delta
    );
}

/// A destination that fails to start restores the source harness, model, and
/// message; the record stays pending; a later send on the source works.
#[test]
fn destination_start_failure_reverts_to_source_and_restores_message() {
    let (mut state, channels) = fake_state();
    codex_thread(&mut state);
    state
        .current_mut()
        .set_model("claude".into(), "claude-opus-4-5".into());
    state
        .current_mut()
        .draft
        .replace_text("Try the other provider.");
    state.test_connect_error = Some("claude could not start".into());
    state.send();

    let t = state.current();
    assert_eq!(t.harness, "codex");
    assert_eq!(t.model_id, "gpt-5.2");
    assert_eq!(t.session.as_deref(), Some("codex-session-1"));
    assert_eq!(t.draft.text, "Try the other provider.");
    assert_eq!(t.handoffs.len(), 1);
    assert_eq!(t.handoffs[0].status, crate::handoff::HandoffStatus::Pending);
    assert_eq!(
        t.submission_error.as_deref(),
        Some("claude could not start")
    );

    // A later send on the source works and carries no package.
    state.test_connect_error = None;
    state.send();
    let commands = take_commands(&channels, 0);
    let prompt = commands
        .iter()
        .find(|c| c["type"] == "prompt")
        .expect("source prompt after failure");
    assert_eq!(prompt["message"], "Try the other provider.");
    assert_eq!(state.test_connect_sessions.last().unwrap().1, "codex");
    assert_eq!(
        state.test_connect_sessions.last().unwrap().2.as_deref(),
        Some("codex-session-1")
    );
}

/// Switching is refused while a run is active or an approval is pending, with
/// KSG's reason text.
#[test]
fn handoff_blocked_while_running_and_pending_request() {
    let (mut state, _channels) = fake_state();
    codex_thread(&mut state);

    state.current_mut().status = Status::Running;
    state
        .current_mut()
        .set_model("claude".into(), "claude-opus-4-5".into());
    assert_eq!(
        state.current().error.as_deref(),
        Some("Wait for the current run to finish before switching providers.")
    );
    assert!(state.current().handoffs.is_empty());
    assert!(state.current().staged_handoff().is_none());

    state.current_mut().status = Status::Ready;
    state.current_mut().error = None;
    state.current_mut().request = Some(json!({"method":"confirm","params":{}}));
    state
        .current_mut()
        .set_model("claude".into(), "claude-opus-4-5".into());
    assert_eq!(
        state.current().error.as_deref(),
        Some("Answer the pending request before switching providers.")
    );
    assert!(state.current().handoffs.is_empty());
    assert!(state.current().staged_handoff().is_none());

    // The picker reports the same reason for cross-provider rows.
    assert_eq!(
        state
            .model_disabled_reason_for("claude", "claude-opus-4-5")
            .as_deref(),
        Some("Answer the pending request before switching providers.")
    );
}

/// Handoff records survive stored_metadata round-trips and a metadata record
/// without them decodes as none.
#[test]
fn handoffs_round_trip_storage_and_decode_old_metadata() {
    let (mut state, _channels) = fake_state();
    codex_thread(&mut state);
    state
        .current_mut()
        .handoffs
        .push(crate::handoff::Handoff::new(
            "codex".into(),
            Some("gpt-5.2".into()),
            "claude".into(),
            Some("claude-opus-4-5".into()),
            2,
            42,
        ));
    state
        .current_mut()
        .handoff_sessions
        .insert("codex".into(), "codex-session-1".into());

    let snapshot = state.snapshot();
    let mut restored = State::new(".".into(), false);
    restored.restore_global(&snapshot).unwrap();
    assert_eq!(restored.current().handoffs.len(), 1);
    assert_eq!(restored.current().handoffs[0].to_instance, "claude");
    assert_eq!(
        restored.current().handoffs[0].status,
        crate::handoff::HandoffStatus::Pending
    );
    assert_eq!(
        restored
            .current()
            .handoff_sessions
            .get("codex")
            .map(String::as_str),
        Some("codex-session-1")
    );

    // Old metadata without the fields decodes with an empty record set.
    let mut old = snapshot.clone();
    for record in old["threads"].as_array_mut().unwrap() {
        record.as_object_mut().unwrap().remove("handoffs");
        record.as_object_mut().unwrap().remove("handoffSessions");
    }
    let mut restored_old = State::new(".".into(), false);
    restored_old.restore_global(&old).unwrap();
    assert!(restored_old.current().handoffs.is_empty());
    assert!(restored_old.current().handoff_sessions.is_empty());
}

/// The handoff choice rides the existing thread.configure request with one new
/// optional field, only when the destination differs.
#[test]
fn set_model_forwards_handoff_field_on_configure() {
    let mut state = State::new(".".into(), false);
    let (client, mut commands) = crate::client::Client::test_connected(state.data_dir.clone());
    *client.info.lock().unwrap() = json!({"capabilities":[crate::protocol::PROVIDER_HANDOFF]});
    let remote_commands = client.commands.clone();
    state.client = Some(client);
    {
        codex_thread(&mut state);
        let t = state.current_mut();
        t.remote = Some(remote_commands);
        t.messages.push(Message {
            id: "u9".into(),
            role: "You",
            text: "Remote thread.".into(),
            images: vec![],
            context: None,
        });
    }
    state
        .current_mut()
        .set_model("claude".into(), "claude-opus-4-5".into());
    let command = commands.try_recv().expect("configure forwarded");
    assert_eq!(command.method, "thread.configure");
    assert_eq!(command.params["handoff"], true);
    assert_eq!(command.params["provider"], "claude");

    // Same-harness picks send no handoff flag.
    state
        .current_mut()
        .set_model("codex".into(), "gpt-5.3".into());
    let command = commands.try_recv().expect("configure forwarded");
    assert_eq!(command.method, "thread.configure");
    assert!(command.params.get("handoff").is_none());
}

/// Without the negotiated capability the picker keeps the provider locked; a
/// capable peer unlocks it.
#[test]
fn picker_stays_locked_for_peer_without_provider_handoff_capability() {
    let mut state = State::new(".".into(), false);
    let (client, _commands) = crate::client::Client::test_connected(state.data_dir.clone());
    *client.info.lock().unwrap() = json!({"capabilities":[]});
    let remote_commands = client.commands.clone();
    state.client = Some(client);
    {
        codex_thread(&mut state);
        let t = state.current_mut();
        t.remote = Some(remote_commands);
    }
    assert_eq!(
        state.locked_provider_for_current().as_deref(),
        Some("codex")
    );
    assert_eq!(
        state
            .model_disabled_reason_for("claude", "claude-opus-4-5")
            .as_deref(),
        Some(
            "This thread does not support switching providers after it has started. Start a new thread to use this model."
        )
    );

    *state.client.as_ref().unwrap().info.lock().unwrap() =
        json!({"capabilities":[crate::protocol::PROVIDER_HANDOFF]});
    assert_eq!(state.locked_provider_for_current(), None);
    assert!(
        state
            .model_disabled_reason_for("claude", "claude-opus-4-5")
            .is_none()
    );
}
