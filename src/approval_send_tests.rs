use super::*;
use crate::backend::Pi;
use serde_json::json;
use std::sync::mpsc;

fn pending(feedback: bool) -> (State, mpsc::Receiver<Value>) {
    let mut state = State::new(".".into(), false);
    let (pi, commands, _) = Pi::test_connection();
    let thread = state.current_mut();
    if feedback {
        thread.harness = "claude".into();
    }
    thread.pi = Some(pi);
    thread.status = Status::Running;
    thread.model_provider = "cpa".into();
    thread.model_id = "fixture".into();
    thread.context_window = Some(200_000);
    thread.session = Some("approval-fixture".into());
    thread.apply(
        json!({"type":"extension_ui_request","method":"confirm","id":"first",
        "title":"Allow this command?","message":"Requested command.",
        "responseCapability":{"type":"live","supportsFeedback":feedback}}),
    );
    thread.draft.replace_text("Use the safer command.");
    (state, commands)
}

#[test]
fn feedback_declines_without_a_new_prompt_and_clears_only_the_unchanged_draft() {
    let (mut state, commands) = pending(true);
    state.send();
    state.poll_approval_sends();
    state.poll_approval_sends();
    let sent: Vec<_> = commands.try_iter().collect();
    let response = sent
        .iter()
        .find(|r| r["type"] == "extension_ui_response")
        .unwrap();
    assert_eq!(response["feedback"], "Use the safer command.");
    assert_eq!(response["confirmed"], false);
    assert!(!sent.iter().any(|r| r["type"] == "prompt"));
    assert!(state.current().draft.text.is_empty());
    assert!(
        state
            .current()
            .messages
            .iter()
            .any(|m| m.text == "Use the safer command.")
    );
}

#[test]
fn changed_draft_prevents_decline_and_keeps_the_new_text() {
    let (mut state, commands) = pending(false);
    state.send();
    state
        .current_mut()
        .draft
        .replace_text("Changed after Send.");
    state.poll_approval_sends();
    assert!(state.current().request.is_some());
    assert_eq!(state.current().draft.text, "Changed after Send.");
    assert!(
        !commands
            .try_iter()
            .any(|r| r["type"] == "extension_ui_response" || r["type"] == "prompt")
    );
}

#[test]
fn redirect_declines_all_shown_approvals_before_sending() {
    for arriving in [false, true] {
        let (mut state, commands) = pending(false);
        if arriving {
            state
                .current_mut()
                .apply(json!({"type":"extension_ui_request","method":"confirm",
                "id":"second","title":"Another command?","message":"New request."}));
        }
        state.send();
        state.poll_approval_sends();
        state.poll_approval_sends();
        state.poll_approval_sends();
        let sent: Vec<_> = commands.try_iter().collect();
        let decline = sent
            .iter()
            .position(|r| r["type"] == "extension_ui_response")
            .unwrap();
        assert_eq!(sent[decline]["confirmed"], false);
        if arriving {
            assert_eq!(
                sent.iter()
                    .filter(|r| r["type"] == "extension_ui_response")
                    .count(),
                2
            );
        }
        let prompt = sent.iter().position(|r| r["type"] == "prompt").unwrap();
        assert!(decline < prompt);
        assert!(state.current().draft.text.is_empty());
    }
}

#[test]
fn approval_arriving_during_decline_preserves_the_redirect_draft() {
    let (mut state, commands) = pending(false);
    state.send();
    state.poll_approval_sends();
    state
        .current_mut()
        .apply(json!({"type":"extension_ui_request","method":"confirm",
        "id":"new-arrival","title":"Another command?","message":"New request."}));
    state.poll_approval_sends();
    assert_eq!(state.current().draft.text, "Use the safer command.");
    let sent: Vec<_> = commands.try_iter().collect();
    assert_eq!(
        sent.iter()
            .filter(|r| r["type"] == "extension_ui_response")
            .count(),
        1
    );
    assert!(!sent.iter().any(|r| r["type"] == "prompt"));
}

#[test]
fn redirect_steers_the_waiting_turn_even_when_followups_normally_queue() {
    let (mut state, commands) = pending(false);
    state.current_mut().native.follow_up = true;
    state.send();
    state.poll_approval_sends();
    state.poll_approval_sends();
    assert!(state.current().native.queue.is_empty());
    let prompt = commands.try_iter().find(|r| r["type"] == "prompt").unwrap();
    assert_eq!(prompt["streamingBehavior"], "steer");
    assert!(state.current().native.follow_up);
}

#[test]
fn redirect_does_not_decline_an_existing_typing_guard_held_approval() {
    let (mut state, commands) = pending(false);
    let thread = state.current_mut();
    thread.draft.focused = true;
    thread.typing_guard.on_type(std::time::Instant::now());
    thread.apply(
        json!({"type":"extension_ui_request","method":"confirm","id":"held-second",
        "title":"Hidden request","message":"Do not decline this one."}),
    );
    assert_eq!(thread.shown_approvals().len(), 1);
    state.send();
    state.poll_approval_sends();
    state.poll_approval_sends();
    let sent: Vec<_> = commands.try_iter().collect();
    let decisions: Vec<_> = sent
        .iter()
        .filter(|r| r["type"] == "extension_ui_response")
        .collect();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["id"], "first");
    assert!(sent.iter().any(|r| r["type"] == "prompt"));
    assert_eq!(
        state.current().request.as_ref().unwrap()["id"],
        "held-second"
    );
}

#[test]
fn typing_guard_held_and_backend_sends_do_not_decline_unshown_requests() {
    for headless in [false, true] {
        let (mut state, commands) = pending(false);
        state.headless = headless;
        if !headless {
            let thread = state.current_mut();
            thread.request = None;
            thread
                .typing_guard
                .on_requests(&[], true, std::time::Instant::now());
            thread.typing_guard.on_type(std::time::Instant::now());
            thread.draft.focused = true;
            thread.apply(
                json!({"type":"extension_ui_request","method":"confirm","id":"held",
                "title":"Held approval","message":"Not shown yet."}),
            );
            assert!(thread.request_held_by_typing_guard());
        }
        state.send();
        state.poll_approval_sends();
        let sent: Vec<_> = commands.try_iter().collect();
        assert!(!sent.iter().any(|r| r["type"] == "extension_ui_response"));
        assert!(sent.iter().any(|r| r["type"] == "prompt"));
        assert!(state.current().request.is_some());
    }
}

#[test]
fn feedback_rejects_attachments_and_inflight_responses_leave_approvals_alone() {
    let (mut state, commands) = pending(true);
    state.current_mut().draft_images.push(Attachment {
        id: "image.png".into(),
        name: "image.png".into(),
        mime: "image/png".into(),
        width: 1,
        height: 1,
        bytes: 1,
        path: None,
        asset_id: None,
        uploaded: None,
        source: None,
    });
    state.send();
    assert_eq!(
        state.current().error.as_deref(),
        Some("Send approval feedback without attachments or added context.")
    );
    assert!(state.current().request.is_some());
    assert!(
        !commands
            .try_iter()
            .any(|r| r["type"] == "extension_ui_response")
    );

    let (mut state, commands) = pending(false);
    state.current_mut().responding_request = Some(uuid::Uuid::new_v4());
    state.send();
    assert!(state.current().request.is_some());
    assert!(
        !commands
            .try_iter()
            .any(|r| r["type"] == "extension_ui_response")
    );
}

#[test]
fn background_redirect_waits_for_the_same_guards_and_feedback_keeps_its_thread() {
    for feedback in [false, true] {
        let (mut state, commands) = pending(feedback);
        let source = state.current().id;
        state.send_background();
        assert_eq!(state.current().id, source);
        state.poll_approval_sends();
        state.poll_approval_sends();
        assert_eq!(state.current().id == source, feedback);
        assert_eq!(
            commands
                .try_iter()
                .filter(|r| r["type"] == "prompt")
                .count(),
            usize::from(!feedback)
        );
    }
}

#[test]
fn older_peer_cannot_receive_feedback_or_a_decline_and_redirect() {
    let (mut state, commands) = pending(true);
    let (client, mut remote_commands) =
        crate::client::Client::test_connected(state.data_dir.clone());
    state.client = Some(client);
    state.send();
    state.poll_approval_sends();
    assert!(
        state
            .current()
            .error
            .as_ref()
            .is_some_and(|e| e.contains("Update required"))
    );
    assert_eq!(state.current().draft.text, "Use the safer command.");
    assert!(state.current().request.is_some());
    assert!(remote_commands.try_recv().is_err());
    assert!(
        !commands
            .try_iter()
            .any(|r| r["type"] == "extension_ui_response")
    );
}

#[test]
fn non_resumable_approvals_are_skipped_without_a_native_decision() {
    let (mut state, commands) = pending(false);
    state.current_mut().request.as_mut().unwrap()["responseCapability"] =
        json!({"type":"not_resumable","reason":"The provider turn ended."});
    state.send();
    state.poll_approval_sends();
    state.poll_approval_sends();
    let records: Vec<_> = commands.try_iter().collect();
    assert!(!records.iter().any(|r| r["type"] == "extension_ui_response"));
    assert!(records.iter().any(|r| r["type"] == "prompt"));
}

#[test]
fn rejected_drafts_and_local_commands_never_decline_an_approval() {
    for text in ["/model", "/compact", "/unknown-command"] {
        let (mut state, commands) = pending(false);
        state.current_mut().draft.replace_text(text);
        state.send();
        state.poll_approval_sends();
        assert!(state.current().request.is_some(), "{text}");
        assert!(
            !commands
                .try_iter()
                .any(|r| r["type"] == "extension_ui_response"),
            "{text}"
        );
    }
}

#[test]
fn feedback_remains_in_history_when_the_provider_does_not_echo_it() {
    let (mut state, commands) = pending(true);
    state.send();
    state.poll_approval_sends();
    state.poll_approval_sends();
    let feedback_id = state
        .current()
        .messages
        .iter()
        .find(|m| m.text == "Use the safer command.")
        .unwrap()
        .id
        .clone();
    for _ in 0..2 {
        state.current_mut().apply(
            json!({"type":"response","id":"history","command":"get_messages",
            "success":true,"data":{"replaceHistory":true,"messages":[
                {"role":"user","content":[{"type":"text","text":"Original prompt."}]},
                {"role":"assistant","content":[{"type":"text","text":"Tool completed."}]}
            ]}}),
        );
        let feedback: Vec<_> = state
            .current()
            .messages
            .iter()
            .filter(|m| m.id == feedback_id)
            .collect();
        assert_eq!(feedback.len(), 1);
        assert_eq!(feedback[0].text, "Use the safer command.");
    }
    assert!(!commands.try_iter().any(|r| r["type"] == "prompt"));
}

#[test]
fn feedback_draft_changes_wait_for_the_receipt_and_preserve_later_text() {
    let (mut state, _) = pending(true);
    let (client, mut commands) = crate::client::Client::test_connected(state.data_dir.clone());
    *client.info.lock().unwrap() = json!({"canApprove":true,"canOperate":true,"capabilities":[
        crate::protocol::COMPOSER_REQUESTS,crate::protocol::APPROVAL_FEEDBACK
    ]});
    state.current_mut().remote = Some(client.commands.clone());
    state.client = Some(client);
    state.send();
    let mut response = None;
    for _ in 0..100 {
        state.poll_approval_sends();
        if let Ok(command) = commands.try_recv() {
            response = Some(command);
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let response = response.expect("feedback command");
    assert_eq!(response.method, "request.respond");
    state.current_mut().draft.replace_text("Later text.");
    state.poll_approval_sends();
    assert!(state.current().approval_send_pending());
    state.current_mut().request = None;
    state.machines.receipts.insert(
        "local".into(),
        vec![crate::client::Event::Receipt(
            response,
            json!({"result":{"status":"accepted","result":{}}}),
        )],
    );
    state.poll_client(&mut Tx::default());
    assert_eq!(state.current().draft.text, "Later text.");
    assert!(!state.current().approval_send_pending());
    assert!(commands.try_recv().is_err());
}

#[test]
fn redirect_requires_operate_permission_before_declining() {
    let (mut state, native) = pending(false);
    let (client, mut commands) = crate::client::Client::test_connected(state.data_dir.clone());
    *client.info.lock().unwrap() = json!({"canApprove":true,"canOperate":false,"capabilities":[
        crate::protocol::COMPOSER_REQUESTS,crate::protocol::APPROVAL_FEEDBACK
    ]});
    state.current_mut().remote = Some(client.commands.clone());
    state.client = Some(client);
    state.send();
    state.poll_approval_sends();
    assert_eq!(
        state.current().error.as_deref(),
        Some("This connection cannot change threads.")
    );
    assert!(state.current().request.is_some());
    assert!(commands.try_recv().is_err());
    assert!(
        !native
            .try_iter()
            .any(|r| r["type"] == "extension_ui_response")
    );
}
