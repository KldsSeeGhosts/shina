use super::*;

fn pending(custom: bool) -> State {
    let mut state = State::new(".".into(), false);
    let (pi, _, _) = Pi::test_connection();
    let t = state.current_mut();
    t.pi = Some(pi);
    t.status = Status::Running;
    t.draft.replace_text("Ordinary draft");
    t.apply(
        json!({"type":"extension_ui_request","id":"question-files","method":"input",
        "questions":[
            {"id":"first","question":"First?","allowCustomAnswer":custom,"options":[]},
            {"id":"second","question":"Second?","options":[]}
        ]}),
    );
    state
}

#[test]
fn question_attachments_are_scoped_and_do_not_replace_the_ordinary_draft() {
    let mut state = pending(true);
    let path = state.data_dir.join("answer.txt");
    std::fs::write(&path, "Answer attachment").unwrap();
    state.attach_paths(&[path]);
    assert_eq!(
        state
            .current()
            .native
            .questionnaire
            .as_ref()
            .unwrap()
            .answers[0]
            .attachment_count,
        1
    );
    assert_eq!(state.current().draft.text, "Ordinary draft");
    assert!(state.current().draft_images.is_empty());
    state.current_mut().questionnaire_step(false);
    assert_eq!(
        state.current().native.questionnaire.as_ref().unwrap().index,
        1
    );
    assert_eq!(
        state
            .current()
            .native
            .questionnaire
            .as_ref()
            .unwrap()
            .answers[1]
            .attachment_count,
        0
    );
    state.current_mut().questionnaire_step(true);
    assert_eq!(
        state
            .current()
            .native
            .questionnaire
            .as_ref()
            .unwrap()
            .answers[0]
            .attachment_count,
        1
    );
}

#[test]
fn choice_only_questions_refuse_attachments_with_ksg_wording() {
    let mut state = pending(false);
    let path = state.data_dir.join("not-an-answer.txt");
    std::fs::write(&path, "No").unwrap();
    state.attach_paths(&[path]);
    assert_eq!(
        state.current().attachment_error.as_deref(),
        Some("This question cannot accept attachments.")
    );
    assert!(state.current().draft_images.is_empty());
}

#[test]
fn question_files_prepare_before_the_native_reply_and_keep_the_ordinary_draft() {
    let mut state = pending(true);
    let (pi, commands, _) = Pi::test_connection();
    state.current_mut().pi = Some(pi);
    let path = state.data_dir.join("answer.txt");
    std::fs::write(&path, "File answer").unwrap();
    state.attach_paths(&[path]);
    state.current_mut().questionnaire_step(false);
    state.current_mut().reply.replace_text("Second answer");
    state.current_mut().questionnaire_step(false);
    assert!(
        !commands
            .try_iter()
            .any(|v| v["type"] == "extension_ui_response")
    );
    for _ in 0..200 {
        state.poll_question_answers();
        if state.current().request.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let reply = commands
        .try_iter()
        .find(|v| v["type"] == "extension_ui_response")
        .expect("native reply");
    let answers: Value = serde_json::from_str(reply["value"].as_str().unwrap()).unwrap();
    assert!(
        answers["first"]
            .as_str()
            .unwrap()
            .contains("Attached file \"answer.txt\": "),
        "{answers}"
    );
    assert_eq!(answers["second"], "Second answer");
    assert_eq!(state.current().draft.text, "Ordinary draft");
    assert!(state.current().draft_images.is_empty());
}

#[test]
fn expiring_questions_return_only_edited_custom_answers_to_the_ordinary_draft() {
    let mut state = pending(true);
    state.current_mut().reply.replace_text("  Unsent first  ");
    state.current_mut().questionnaire_step(false);
    state.current_mut().reply.replace_text("Unsent second");
    state
        .current_mut()
        .apply(json!({"type":"extension_ui_cancel","id":"question-files"}));
    assert_eq!(
        crate::composer_rich::markdown(&state.current().draft),
        "Ordinary draft\n\nUnsent first\n\nUnsent second"
    );
    assert!(state.current().native.questionnaire.is_none());
    assert_eq!(
        state.current().question_resolutions.last().unwrap().reason,
        "cancelled"
    );

    let mut state = State::new(".".into(), false);
    state.current_mut().draft.replace_text("Ordinary draft");
    state.current_mut().apply(
        json!({"type":"extension_ui_request","id":"unmodified-prefill","method":"editor",
        "title":"Edit","prefill":"Do not recover an unedited suggestion"}),
    );
    state
        .current_mut()
        .apply(json!({"type":"extension_ui_cancel","id":"unmodified-prefill"}));
    assert_eq!(state.current().draft.text, "Ordinary draft");
}
