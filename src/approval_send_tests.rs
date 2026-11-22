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
