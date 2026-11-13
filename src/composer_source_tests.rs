use super::*;
use crate::composer_rich;
use rune::Ime;

fn state_from(md: &str) -> TextEdit {
    let mut edit = composer_rich::draft("");
    composer_rich::paste(&mut edit, md, &|_| None);
    edit.clear_history();
    edit
}

#[test]
fn part_a_reveal_and_collapse_all_four_marks_and_nested_marks_keep_markdown() {
    for (source, expected_text, expected_marks) in [
        ("**bold** after", "bold after", vec![Mark::Bold]),
        ("*italic* after", "italic after", vec![Mark::Italic]),
        ("~~strike~~ after", "strike after", vec![Mark::Strike]),
        ("`code` after", "code after", vec![Mark::Code]),
        ("**a *b* c** d", "a b c d", vec![Mark::Bold, Mark::Italic]),
        ("***both*** d", "both d", vec![Mark::Bold, Mark::Italic]),
        (
            "**a `code` c** d",
            "a code c d",
            vec![Mark::Bold, Mark::Code],
        ),
        ("*a **b** c* d", "a b c d", vec![Mark::Italic, Mark::Bold]),
        (
            "~~**`x`**~~ d",
            "x d",
            vec![Mark::Strike, Mark::Bold, Mark::Code],
        ),
    ] {
        let mut edit = state_from(source);
        assert_eq!(edit.text, expected_text, "{source}");
        let mut st = MarkdownSourceState::default();

        // Caret inside the marked run (pos 1) reveals the source.
        edit.set_caret(1);
        on_selection_change(&mut st, &mut edit);
        assert!(has_shown_source(&edit), "expected reveal for {source}");
        assert_eq!(edit.text, source, "revealed text mismatch for {source}");
        assert_eq!(
            composer_rich::markdown(&edit),
            source,
            "serialized markdown while revealed for {source}"
        );

        // Moving caret to the end (outside the revealed region) collapses back.
        let end = edit.text.len();
        edit.set_caret(end);
        on_selection_change(&mut st, &mut edit);
        assert!(!has_shown_source(&edit), "expected collapse for {source}");
        assert_eq!(
            edit.text, expected_text,
            "collapsed text mismatch for {source}"
        );
        assert_eq!(composer_rich::markdown(&edit), source);
        for mark in &expected_marks {
            assert!(
                edit.marks().iter().any(|m| &m.mark == mark),
                "missing mark {mark:?} after collapse for {source}"
            );
        }
    }
}

#[test]
fn part_a_one_region_at_a_time_and_single_notification_jump() {
    let mut edit = state_from("**first** and **second**");
    let mut st = MarkdownSourceState::default();
    assert_eq!(edit.text, "first and second");

    // Enter first region.
    edit.set_caret(2);
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "**first** and second");
    assert_eq!(source_regions(&edit).len(), 1);

    // Jump directly into second region in one notification: collapses first, reveals second.
    let second_pos = edit.text.find("sec").unwrap() + 1;
    edit.set_caret(second_pos);
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "first and **second**");
    assert_eq!(source_regions(&edit).len(), 1);
    assert_eq!(composer_rich::markdown(&edit), "**first** and **second**");
}

#[test]
fn part_a_editing_delimiters_breaks_or_changes_mark() {
    // Deleting opening markers leaves text literal.
    let mut edit = state_from("**bold** after");
    let mut st = MarkdownSourceState::default();
    edit.set_caret(0);
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "**bold** after");
    edit.replace_range(0..2, "");
    on_selection_change(&mut st, &mut edit);
    edit.set_caret(edit.text.len());
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "bold** after");
    assert!(edit.marks().is_empty());

    // Turning ** into * converts Bold to Italic.
    let mut edit = state_from("**bold** after");
    let mut st = MarkdownSourceState::default();
    edit.set_caret(0);
    on_selection_change(&mut st, &mut edit);
    edit.replace_range(7..8, "");
    edit.replace_range(0..1, "");
    edit.set_caret(edit.text.len());
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "bold after");
    assert_eq!(composer_rich::markdown(&edit), "*bold* after");
    assert!(edit.marks().iter().any(|m| m.mark == Mark::Italic));
    assert!(!edit.marks().iter().any(|m| m.mark == Mark::Bold));
}

#[test]
fn part_a_unfaithful_or_escaped_ranges_do_not_reveal() {
    // Bold `2*3` or `path\` or `\**word**` would not round-trip, so stay hidden.
    for (text, bold_range) in [
        ("2*3 it", 0..3),
        ("path\\ it", 0..5),
        ("\\word it", 1..5),
        ("*word it", 1..5),
    ] {
        let mut edit = composer_rich::draft(text);
        edit.set_marks(vec![MarkRange::new(bold_range.clone(), Mark::Bold)]);
        let mut st = MarkdownSourceState::default();
        edit.set_caret(bold_range.start + 1);
        on_selection_change(&mut st, &mut edit);
        assert!(!has_shown_source(&edit), "should not reveal {text}");
        assert_eq!(edit.text, text);
    }

    // Preceded by `\\` (even backslashes) or code ending in `\`: DOES reveal.
    let mut edit = composer_rich::draft("\\\\word it");
    edit.set_marks(vec![MarkRange::new(2..6, Mark::Bold)]);
    let mut st = MarkdownSourceState::default();
    edit.set_caret(3);
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "\\\\**word** it");

    let mut edit = composer_rich::draft("path\\word it");
    edit.set_marks(vec![
        MarkRange::new(0..5, Mark::Code),
        MarkRange::new(5..9, Mark::Bold),
    ]);
    let mut st = MarkdownSourceState::default();
    edit.set_caret(6);
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "path\\**word** it");
}

#[test]
fn part_a_non_collapsed_selection_does_not_reveal_and_keeps_active_region() {
    let mut edit = state_from("**bold** after");
    let mut st = MarkdownSourceState::default();
    edit.set_selection(0, 4);
    on_selection_change(&mut st, &mut edit);
    assert!(!has_shown_source(&edit));
    assert_eq!(edit.text, "bold after");

    // Once revealed by a collapsed caret, a non-collapsed selection inside keeps it revealed.
    edit.set_caret(2);
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "**bold** after");
    edit.set_selection(0, 2);
    on_selection_change(&mut st, &mut edit);
    assert!(has_shown_source(&edit));

    // Selecting outside collapses it.
    edit.set_selection(9, 14);
    on_selection_change(&mut st, &mut edit);
    assert!(!has_shown_source(&edit));
    assert_eq!(edit.text, "bold after");
}

#[test]
fn part_a_pill_inside_bold_survives_reveal_and_collapse() {
    let token = crate::composer_tokens::Token::File {
        path: "foo_bar.rs".into(),
        directory: false,
        source: Some("@foo_bar.rs".into()),
    };
    let mut edit = composer_rich::draft("hello @foo_bar.rs world");
    edit.replace_with_pill(6..17, token.id(), "@foo_bar.rs");
    edit.set_selection(0, edit.text.len());
    edit.set_mark(Mark::Bold);

    let mut st = MarkdownSourceState::default();
    edit.set_caret(2);
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "**hello @foo_bar.rs world**");
    assert_eq!(edit.pills().count(), 1);
    assert_eq!(
        composer_rich::markdown(&edit),
        "**hello @foo_bar.rs world**"
    );

    edit.set_caret(0);
    // Still at leading edge, move outside by adding trailing text.
    edit.replace_range(edit.text.len()..edit.text.len(), " tail");
    edit.set_caret(edit.text.len());
    on_selection_change(&mut st, &mut edit);
    assert!(!has_shown_source(&edit));
    assert_eq!(edit.text, "hello @foo_bar.rs world tail");
    assert_eq!(edit.pills().count(), 1);
    assert_eq!(
        composer_rich::markdown(&edit),
        "**hello @foo_bar.rs world** tail"
    );
}

#[test]
fn part_a_ime_composition_defers_reveal_and_collapse_until_commit() {
    let mut edit = state_from("**bold** after");
    let mut st = MarkdownSourceState::default();
    edit.set_caret(2);
    edit.ime(&Ime::Preedit("に".into(), None));
    on_selection_change(&mut st, &mut edit);
    assert!(!has_shown_source(&edit));
    assert_eq!(edit.text, "bold after");

    edit.ime(&Ime::Commit("に".into()));
    on_selection_change(&mut st, &mut edit);
    assert!(has_shown_source(&edit));
    assert_eq!(edit.text, "**boにld** after");

    // Composition while revealed does not collapse even if caret moves outside during preedit.
    edit.set_caret(edit.text.len());
    edit.ime(&Ime::Preedit("か".into(), None));
    on_selection_change(&mut st, &mut edit);
    assert!(has_shown_source(&edit));

    edit.ime(&Ime::Commit("か".into()));
    on_selection_change(&mut st, &mut edit);
    assert!(!has_shown_source(&edit));
    assert_eq!(edit.text, "boにld afterか");
    assert!(edit.marks().iter().any(|m| m.mark == Mark::Bold));
}

#[test]
fn part_a_decorations_style_delimiters_faint_and_keep_content_marks() {
    let mut edit = state_from("~~***`x`***~~");
    let mut st = MarkdownSourceState::default();
    edit.set_caret(0);
    on_selection_change(&mut st, &mut edit);
    assert_eq!(edit.text, "~~***`x`***~~");

    let base = TextStyle::new(15.0, 400.0);
    let faint = Color::from_oklab([0.5, 0.0, 0.0, 1.0]);
    let ink = Color::WHITE;
    let code_bg = Color::BLACK;
    let (spans, strikes) = preview_decorations(&edit, base, faint, ink, code_bg);
    // Opening `~~***\`` is 0..6, `x` is 6..7, closing `\`***~~` is 7..13.
    assert!(spans.iter().any(|s| s.start == 0
        && s.end == 6
        && s.color == Some(faint)
        && s.weight.is_none()
        && !s.italic
        && !s.mono));
    assert!(spans.iter().any(|s| s.start == 7
        && s.end == 13
        && s.color == Some(faint)
        && s.weight.is_none()
        && !s.italic
        && !s.mono));
    assert!(
        spans
            .iter()
            .any(|s| s.start == 6 && s.end == 7 && s.weight == Some(650.0))
    );
    assert!(spans.iter().any(|s| s.start == 6 && s.end == 7 && s.italic));
    assert!(spans.iter().any(|s| s.start == 6
        && s.end == 7
        && s.mono
        && s.color == Some(ink)
        && s.bg == Some(code_bg)));
    assert_eq!(strikes, vec![(6, 7, None)]);
}

#[test]
fn part_a_draft_save_and_send_while_revealed_use_collapsed_text() {
    let temp_dir = std::env::temp_dir().join(format!("shina-source-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    struct CleanDir(std::path::PathBuf);
    impl Drop for CleanDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _guard = CleanDir(temp_dir.clone());

    crate::store::with_directory(temp_dir, || {
        let mut state = crate::model::State::new(std::path::PathBuf::new(), false);
        state.load_global_for_test();
        let mut t = crate::model::Thread::new(1);
        t.draft = state_from("**bold** after");
        t.draft.set_caret(2);
        on_selection_change(&mut t.composer_source, &mut t.draft);
        assert_eq!(t.draft.text, "**bold** after");
        assert!(has_shown_source(&t.draft));

        // Saving draft stores collapsed text/html.
        state.threads = vec![t];
        state.tabs = vec![1];
        state.active = 0;
        let ui = state.ui_snapshot();
        let saved = &ui["drafts"]["1"];
        assert_eq!(saved["text"].as_str(), Some("bold after"));
        assert!(
            !saved["richHtml"]
                .as_str()
                .unwrap()
                .contains("markdown-source")
        );

        // Restoring starts collapsed.
        state.checkpoint_and_flush().unwrap();
        drop(state);
        let mut restarted = crate::model::State::new(std::path::PathBuf::new(), false);
        restarted.load_global_for_test();
        let reloaded = &restarted.threads.iter().find(|t| t.id == 1).unwrap().draft;
        assert_eq!(reloaded.text, "bold after");
        assert!(!has_shown_source(reloaded));
        assert_eq!(composer_rich::markdown(reloaded), "**bold** after");

        // Sending while revealed produces collapsed prompt text and snapshot.
        let t = restarted.current_mut();
        t.draft.set_caret(2);
        on_selection_change(&mut t.composer_source, &mut t.draft);
        assert_eq!(t.draft.text, "**bold** after");
        let prompt = crate::composer_tokens::prompt_for_with_context(
            &t.draft,
            &t.draft_images,
            &t.draft_context,
            None,
            &str::to_owned,
        )
        .unwrap();
        assert_eq!(prompt.text, "**bold** after");
        let snap = prompt.draft.unwrap();
        assert_eq!(snap.text, "bold after");
        assert!(!snap.html.contains("markdown-source"));
    });
}
