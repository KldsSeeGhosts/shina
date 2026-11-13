use super::*;

fn question(capability: &str) -> (Thread, mpsc::Receiver<Value>) {
    let mut thread = Thread::new(1);
    let (pi, commands, _) = Pi::test_connection();
    thread.pi = Some(pi);
    thread.harness = "codex".into();
    thread.model_provider = "fixture".into();
    thread.model_id = "fixture".into();
    thread.session = Some("fixture-session".into());
    thread.status = Status::Running;
    thread.draft.replace_text("Ordinary draft");
    thread.apply(
        json!({"type":"extension_ui_request","id":"question","method":"input",
        "responseCapability":{"type":capability},"questions":[
            {"id":"one","header":"Question","question":"Where?","options":[]}
        ]}),
    );
    (thread, commands)
}

#[test]
fn message_question_survives_settlement_and_answers_as_a_new_message() {
    let (mut thread, commands) = question("message");
    thread.reply.replace_text("  On disk.  ");
    thread.apply(json!({"type":"agent_settled","aborted":false}));
    assert!(thread.request.is_some());
    assert!(thread.questionnaire_can_respond());
    thread.questionnaire_step(false);
    assert_eq!(thread.draft.text, "Ordinary draft");
    let sent: Vec<_> = commands.try_iter().collect();
    assert!(!sent.iter().any(|v| v["type"] == "extension_ui_response"));
    assert!(
        sent.iter()
            .any(|v| v["type"] == "prompt" && v["message"] == "Where?\nOn disk."),
        "{sent:?}"
    );
    assert!(thread.request.is_none());
}

#[test]
fn live_question_becomes_not_resumable_and_cannot_send_after_settlement() {
    let (mut thread, commands) = question("live");
    thread.reply.replace_text("Answer");
    thread.apply(json!({"type":"agent_settled","aborted":false}));
    assert!(thread.request.is_some());
    assert!(!thread.questionnaire_can_respond());
    thread.questionnaire_step(false);
    assert!(
        !commands
            .try_iter()
            .any(|v| v["type"] == "extension_ui_response" || v["type"] == "prompt")
    );
}

#[test]
fn question_responding_and_older_peer_guards_preserve_the_answer() {
    let (mut thread, commands) = question("live");
    thread.reply.replace_text("Draft answer");
    thread.responding_request = Some(uuid::Uuid::new_v4());
    thread.questionnaire_toggle(0);
    thread.questionnaire_step(false);
    assert!(thread.request.is_some());
    assert_eq!(thread.reply.text, "Draft answer");
    thread.responding_request = None;
    thread.question_contract_supported = false;
    thread.respond(json!({"value":"{}"}));
    assert_eq!(
        thread.error.as_deref(),
        Some(crate::protocol::QUESTIONS_UPDATE_REQUIRED)
    );
    assert!(
        !commands
            .try_iter()
            .any(|v| v["type"] == "extension_ui_response")
    );
}

#[test]
fn question_editor_prefill_and_independent_question_drafts_are_not_reseeded() {
    let (mut thread, _) = question("live");
    thread.request = None;
    thread.apply(
        json!({"type":"extension_ui_request","id":"prefills","method":"input","questions":[
            {"id":"one","question":"First?","initialAnswer":"  initial  ","options":[]},
            {"id":"two","question":"Second?","initialAnswer":"second initial","options":[]}
        ]}),
    );
    assert_eq!(thread.reply.text, "  initial  ");
    thread.reply.replace_text("edited first");
    thread.questionnaire_step(false);
    assert_eq!(thread.reply.text, "second initial");
    thread.reply.replace_text("edited second");
    thread.questionnaire_step(true);
    assert_eq!(thread.reply.text, "edited first");
    thread.questionnaire_step(false);
    assert_eq!(thread.reply.text, "edited second");
}
