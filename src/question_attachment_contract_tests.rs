use super::*;

#[test]
fn attachment_only_answers_require_custom_input_and_preparation_blocks_submission() {
    for allow_custom in [true, false] {
        let mut form = Questionnaire::parse(&json!({"questions":[{
            "id":"one","question":"Attach a file.","allowCustomAnswer":allow_custom,
            "options":[{"label":"A"}]
        }]}))
        .unwrap();
        form.answers[0].attachment_count = 1;
        assert_eq!(form.answer(0), allow_custom.then(|| json!("")));
        form.toggle(0);
        form.answers[0].attachments_blocked = true;
        assert!(form.answer(0).is_none());
        assert!(!form.complete());
        form.answers[0].attachments_blocked = false;
        assert_eq!(form.answer(0), Some(json!("A")));
    }
}
