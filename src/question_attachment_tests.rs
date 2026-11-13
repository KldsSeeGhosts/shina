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
fn removing_question_file_does_not_cancel_an_ordinary_upload_with_the_same_id() {
    let mut state = pending(true);
    let path = state.data_dir.join("shared.txt");
    std::fs::write(&path, "Same file in both drafts").unwrap();
    let file = attachments::attach_file(&path).unwrap();
    state.current_mut().draft_images.push(file.clone());
    state
        .attachment_uploads
        .entries
        .push(crate::attachment_uploads::Entry::fixture(
            "local",
            state.current().id,
            &file,
            crate::attachment_uploads::Phase::Preparing,
        ));
    state.attach_paths(&[path]);
    assert_eq!(state.current().question_files().len(), 1);
    state.remove_attachment(&file.id);
    assert!(state.current().question_files().is_empty());
    assert_eq!(state.current().draft_images, vec![file]);
    assert_eq!(state.attachment_uploads.entries.len(), 1);
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
fn question_capacity_is_reserved_across_questions_and_pending_imports_block_answers() {
    let mut state = pending(true);
    for i in 0..attachments::MAX_ATTACHMENTS {
        let path = state.data_dir.join(format!("file-{i}.txt"));
        std::fs::write(&path, format!("File {i}")).unwrap();
        state.attach_paths(&[path]);
    }
    assert_eq!(
        state.current().question_files().len(),
        attachments::MAX_ATTACHMENTS
    );
    state.current_mut().questionnaire_step(false);
    let path = state.data_dir.join("one-too-many.txt");
    std::fs::write(&path, "No capacity").unwrap();
    state.attach_paths(&[path]);
    assert!(state.current().question_files().is_empty());
    assert_eq!(
        state.current().question_reserved_count(),
        attachments::MAX_ATTACHMENTS
    );
    let scope = state.current().question_scope().unwrap();
    state
        .current_mut()
        .question_drafts
        .entry(scope)
        .or_default()
        .pending = 1;
    state
        .current_mut()
        .reply
        .replace_text("Text while importing");
    state.current_mut().save_question_editor();
    assert!(
        !state
            .current()
            .native
            .questionnaire
            .as_ref()
            .unwrap()
            .complete()
    );
}

#[test]
fn an_older_peer_refuses_question_attachments_before_mutating_either_draft() {
    let mut state = pending(true);
    let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
    *client.info.lock().unwrap() = json!({"canApprove":true,"capabilities":[
        crate::protocol::COMPOSER_REQUESTS,crate::protocol::QUESTION_CONTRACT]});
    state.client = Some(client);
    let path = state.data_dir.join("old-peer.txt");
    std::fs::write(&path, "Keep draft").unwrap();
    state.attach_paths(&[path]);
    assert_eq!(
        state.current().attachment_error.as_deref(),
        Some("This question cannot accept attachments.")
    );
    assert_eq!(state.current().draft.text, "Ordinary draft");
    assert!(state.current().question_files().is_empty());
}

#[test]
fn a_typing_guard_held_question_keeps_ordinary_attachment_intake_separate() {
    let mut state = State::new(".".into(), false);
    let t = state.current_mut();
    t.draft.replace_text("Still writing");
    t.draft.focused = true;
    t.typing_guard.on_type(Instant::now());
    t.apply(
        json!({"type":"extension_ui_request","method":"input","id":"held-question",
        "title":"Not shown while typing"}),
    );
    assert!(t.request_held_by_typing_guard());
    let path = state.data_dir.join("ordinary.txt");
    std::fs::write(&path, "Ordinary draft file").unwrap();
    state.attach_paths(&[path]);
    assert_eq!(state.current().draft_images.len(), 1);
    assert!(state.current().question_drafts.is_empty());
    assert!(!state.current().questionnaire_can_respond());
}

#[test]
fn preparation_failure_or_generation_change_never_answers_another_request() {
    for missing in [false, true] {
        let mut state = pending(true);
        let (pi, commands, _) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        let path = state.data_dir.join("transient.txt");
        std::fs::write(&path, "Question file").unwrap();
        state.attach_paths(&[path.clone()]);
        state.current_mut().questionnaire_step(false);
        state.current_mut().reply.replace_text("Second");
        state.current_mut().questionnaire_step(false);
        if missing {
            std::fs::remove_file(path).unwrap();
        }
        state.poll_question_answers();
        if !missing {
            state.current_mut().request_generation = "another-runtime".into();
        }
        for _ in 0..200 {
            state.poll_question_answers();
            if state.current().question_submission.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !commands
                .try_iter()
                .any(|v| v["type"] == "extension_ui_response")
        );
        assert!(state.current().request.is_some());
        assert_eq!(state.current().draft.text, "Ordinary draft");
    }
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

#[test]
fn literal_question_pastes_do_not_fold_or_parse_markdown() {
    let mut state = State::new(".".into(), false);
    state.current_mut().apply(
        json!({"type":"extension_ui_request","id":"literal","method":"editor",
        "title":"Edit","prefill":""}),
    );
    let text = format!(
        "  **literal**\n{}\n\n",
        "a".repeat(crate::composer_paste::THRESHOLD)
    );
    assert!(state.paste_composer_content(&rune::ClipboardContent::new(&text), false));
    assert_eq!(state.current().reply.text, text);
    assert!(state.current().question_files().is_empty());
}

#[test]
fn image_answers_use_file_references_not_the_native_prompt_image_count_limit() {
    let mut state = pending(true);
    let (pi, commands, _) = Pi::test_connection();
    state.current_mut().pi = Some(pi);
    for n in 0..9u8 {
        let image = image::DynamicImage::ImageRgba8(image::ImageBuffer::from_pixel(
            1,
            1,
            image::Rgba([n, 32, 64, 255]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        state.attach_image(&format!("image-{n}.png"), bytes.get_ref());
    }
    assert_eq!(state.current().question_files().len(), 9);
    let end = state.current().reply.text.len();
    state.current_mut().reply.set_selection(0, end);
    let scope = state.current().question_scope().unwrap();
    let copied = state
        .with_question_draft(state.active, scope, |s| s.copy_composer_selection(false))
        .flatten()
        .unwrap();
    assert!(copied.text().contains("t3-context://v1/image/"));
    state.current_mut().questionnaire_step(false);
    state.current_mut().reply.replace_text("Second");
    state.current_mut().questionnaire_step(false);
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
        .unwrap();
    let answers: Value = serde_json::from_str(reply["value"].as_str().unwrap()).unwrap();
    assert_eq!(
        answers["first"]
            .as_str()
            .unwrap()
            .matches("Attached image ")
            .count(),
        9
    );
    assert_eq!(state.current().draft.text, "Ordinary draft");
}
