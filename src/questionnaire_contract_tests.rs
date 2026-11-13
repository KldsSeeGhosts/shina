use super::*;

#[test]
fn choice_only_ignores_custom_text_and_preserves_literal_option_values() {
    let mut form = Questionnaire::parse(&json!({"questions":[{
        "id":"native-choice","question":"**Choose** one.","allowCustomAnswer":false,
        "options":[{"label":"Empty","value":""},{"label":"Spaced","value":" value "}]
    }]}))
    .unwrap();
    form.answers[0].other = "This is not an offered choice.".into();
    assert!(form.answer(0).is_none());
    form.toggle(0);
    assert_eq!(form.answer(0), Some(json!("")));
    form.toggle(1);
    assert_eq!(form.answer(0), Some(json!(" value ")));
}

#[test]
fn literal_prefill_keeps_whitespace_and_accepts_an_empty_edited_answer() {
    let mut form = Questionnaire::parse(&json!({"questions":[{
        "id":"editor","question":"Edit the message.","initialAnswer":"  line one\n\n",
        "options":[]
    }]}))
    .unwrap();
    assert_eq!(form.answers[0].other, "  line one\n\n");
    assert_eq!(form.answer(0), Some(json!("  line one\n\n")));
    form.answers[0].other.clear();
    assert_eq!(form.answer(0), Some(json!("")));
    assert!(form.complete());

    let choice_only = Questionnaire::parse(&json!({"questions":[{
        "id":"no-custom","question":"Choose.","allowCustomAnswer":false,
        "initialAnswer":"Not a valid choice.","options":[{"label":"A"}]
    }]}))
    .unwrap();
    assert!(choice_only.answer(0).is_none());
}

#[test]
fn multi_select_keeps_selection_order_and_custom_text_overrides_it() {
    let mut form = Questionnaire::parse(&json!({"questions":[{
        "id":"multi","question":"Choose.","multiSelect":true,
        "options":[{"label":"A"},{"label":"B"},{"label":"C"}]
    }]}))
    .unwrap();
    form.toggle(2);
    form.toggle(0);
    assert_eq!(form.answer(0), Some(json!(["C", "A"])));
    form.answers[0].other = "  custom answer  ".into();
    assert_eq!(form.answer(0), Some(json!("custom answer")));
    form.toggle(1);
    assert_eq!(form.answer(0), Some(json!(["C", "A", "B"])));
}

#[test]
fn request_and_question_identity_scope_answer_drafts_and_prefill() {
    let request = |id: &str| {
        json!({"id":id,"questions":[
            {"id":"same-question","question":"First.","options":[],"initialAnswer":"first prefill"},
            {"id":"another-question","question":"Second.","options":[],"initialAnswer":"second prefill"}
        ]})
    };
    let mut first = Questionnaire::parse(&request("first-request")).unwrap();
    first.answers[0].other = "edited first answer".into();
    first.index = 1;
    assert_eq!(first.answers[1].other, "second prefill");
    first.index = 0;
    assert_eq!(first.answers[0].other, "edited first answer");
    let second = Questionnaire::parse(&request("second-request")).unwrap();
    assert_eq!(second.answers[0].other, "first prefill");
}

#[test]
fn message_requests_stay_answerable_and_non_resumable_requests_do_not() {
    for (capability, expected) in [
        ("live", true),
        ("message", true),
        ("not_resumable", false),
        ("future-capability", false),
    ] {
        let form = Questionnaire::parse(&json!({"responseCapability":{"type":capability},
            "questions":[{"id":"one","question":"Answer.","options":[]}]}))
        .unwrap();
        assert_eq!(form.can_respond(), expected);
    }
}

#[test]
fn native_pi_select_input_and_editor_preserve_their_distinct_wire_contracts() {
    let mut select = Questionnaire::pi(&json!({"id":"select","method":"select","title":"Pick",
        "options":["","  exact value  "]}))
    .unwrap();
    assert_eq!(select.questions[0].options[0].label, "Empty value");
    select.toggle(0);
    assert_eq!(select.response(), Some(json!({"value":""})));
    select.toggle(1);
    assert_eq!(select.response(), Some(json!({"value":"  exact value  "})));
    let mut input = Questionnaire::pi(&json!({"id":"input","method":"input","title":"Input",
        "placeholder":"Enter **text**."}))
    .unwrap();
    assert_eq!(input.questions[0].header, "Input");
    assert_eq!(input.questions[0].text, "Enter **text**.");
    assert_eq!(input.questions[0].options[0].label, "Submit empty value");
    input.toggle(0);
    assert_eq!(input.response(), Some(json!({"value":""})));
    let editor = Questionnaire::pi(&json!({"id":"editor","method":"editor","title":"Edit",
        "prefill":"  literal\n"}))
    .unwrap();
    assert_eq!(editor.response(), Some(json!({"value":"  literal\n"})));
}

#[test]
fn custom_edits_clear_selections_without_losing_whitespace_only_nonliteral_selections() {
    let mut form = Questionnaire::parse(&json!({"questions":[{
        "id":"one","question":"Choose.","options":[{"label":"A"}]
    }]}))
    .unwrap();
    form.toggle(0);
    form.edit("   ".into());
    assert_eq!(form.answer(0), Some(json!("A")));
    form.edit(" new answer ".into());
    assert!(form.answers[0].selected.is_empty());
    assert_eq!(form.answer(0), Some(json!("new answer")));
    form.edit("".into());
    assert!(form.answer(0).is_none());
}
