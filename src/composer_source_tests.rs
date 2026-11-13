use super::*;
use crate::composer_rich::{self, Rules};
use rune::{BlockKind, Cx, El, Env, Ime, Key, KeyEvent, Mods, Runtime, text_input};

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

struct TestEditor {
    edit: TextEdit,
    rules: Rules,
    source: MarkdownSourceState,
}

impl TestEditor {
    fn new(initial_md: &str) -> Runtime<Self> {
        let mut edit = if initial_md.is_empty() {
            composer_rich::draft("")
        } else {
            state_from(initial_md)
        };
        edit.focused = true;
        let state = Self {
            edit,
            rules: Rules::default(),
            source: MarkdownSourceState::default(),
        };
        let view = |s: &Self, _: &mut Cx<Self>| -> El {
            let style = TextStyle::new(15.0, 400.0);
            let (spans, strikes) = preview_decorations(
                &s.edit,
                style,
                Color::from_oklab([0.5, 0.0, 0.0, 1.0]),
                Color::WHITE,
                Color::BLACK,
            );
            text_input(&s.edit, style)
                .key("editor")
                .w(400.0)
                .min_h(80.0)
                .on_edit(|s: &mut Self| &mut s.edit)
                .on_input_key(|s: &mut Self, ev, _| {
                    let before = s.edit.text.clone();
                    s.source.on_key_before(&s.edit, ev);
                    let handled = s.rules.key(&mut s.edit, ev);
                    if handled && s.edit.text != before {
                        s.source.on_doc_change(&s.edit);
                    }
                    handled
                })
                .on_input_change(|s: &mut Self, _| {
                    if s.rules.take_history_suppress() {
                        s.source.on_history_change(&s.edit);
                    } else {
                        s.source.on_doc_change(&s.edit);
                    }
                    s.rules.changed(&mut s.edit);
                })
                .on_input_selection_change(|s: &mut Self, _| {
                    on_selection_change(&mut s.source, &mut s.edit);
                })
                .decorations(spans)
                .strikes(strikes)
        };
        let mut rt = Runtime::new(state, view, (420.0, 120.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt
    }
}

fn send_key(rt: &mut Runtime<TestEditor>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(0.0);
}

fn type_str(rt: &mut Runtime<TestEditor>, s: &str) {
    for ch in s.chars() {
        send_key(rt, Key::Text(ch.to_string()), Mods::default());
    }
}

fn cmd_z(rt: &mut Runtime<TestEditor>, redo: bool) {
    send_key(
        rt,
        Key::Command('z'),
        Mods {
            cmd: true,
            shift: redo,
            ..Mods::default()
        },
    );
}

fn doc_key(edit: &TextEdit) -> (String, Vec<MarkRange>, Vec<rune::Block>) {
    (
        edit.text.clone(),
        edit.marks().to_vec(),
        edit.blocks().to_vec(),
    )
}

fn probe_redo_depth(edit: &TextEdit, has_undo: bool) -> usize {
    let mut probe = edit.clone();
    let mut depth = 0usize;
    loop {
        let before = (doc_key(&probe), probe.anchor, probe.focus);
        probe.redo();
        let after = (doc_key(&probe), probe.anchor, probe.focus);
        if after == before {
            break;
        }
        if depth == 0 && has_undo {
            let mut check = edit.clone();
            check.undo();
            check.redo();
            if doc_key(&probe) == doc_key(&check) {
                break;
            }
        }
        depth += 1;
    }
    depth
}

#[test]
fn part_b_type_reveal_edit_collapse_undo_and_redo_step_by_step() {
    let mut rt = TestEditor::new("");

    // Step 1: Type "**bold** after".
    type_str(&mut rt, "**bold** after");
    assert_eq!(rt.state.edit.text, "bold after");
    assert!(!has_shown_source(&rt.state.edit));
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after");
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 0);

    // Step 2: Reveal by moving caret inside "bold" (no undo step added, redo stays 0).
    rt.update(|s, _| s.edit.set_caret(2));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "**bold** after");
    assert!(has_shown_source(&rt.state.edit));
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 0);

    // Step 3: Edit while revealed ("**boXld** after").
    send_key(&mut rt, Key::Text("X".into()), Mods::default());
    assert_eq!(rt.state.edit.text, "**boXld** after");
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 0);

    // Step 4: Collapse by moving caret to end.
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(rt.state.edit.text, "boXld after");
    assert!(!has_shown_source(&rt.state.edit));
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**boXld** after");
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 0);

    // Step 5: Undo once -> restores collapsed "**bold** after" (revealed because caret
    // returned to pos 2 inside "bold", and redo depth becomes 1!).
    cmd_z(&mut rt, false);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after");
    assert_eq!(rt.state.edit.text, "**bold** after");
    assert!(has_shown_source(&rt.state.edit));
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 1);

    // Collapse by moving caret out -> redo depth STILL 1!
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(rt.state.edit.text, "bold after");
    assert!(!has_shown_source(&rt.state.edit));
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 1);

    // Reveal again after undo -> keeps redo depth 1!
    rt.update(|s, _| s.edit.set_caret(2));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "**bold** after");
    assert!(has_shown_source(&rt.state.edit));
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 1);

    // Step 6: Undo again -> undoes " after", back to "**bold**" (caret at end of "bold", revealed).
    cmd_z(&mut rt, false);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold**");
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 2);

    // Step 7: Undo again -> undoes "**bold**" input rule back to "**bold*".
    cmd_z(&mut rt, false);
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 3);

    // Redo step by step back to "**boXld** after".
    cmd_z(&mut rt, true);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold**");
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 2);

    cmd_z(&mut rt, true);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after");
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 1);

    cmd_z(&mut rt, true);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**boXld** after");
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 0);

    // Move caret out to confirm collapsed state.
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(rt.state.edit.text, "boXld after");
    assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Bold));
}

#[test]
fn part_b_ksg_undoes_a_marker_edit_in_one_step_without_extra_step_for_showing_markers() {
    // Ported from KSG ComposerPromptEditorTiptap.input.browser.test.tsx:
    // "undoes a marker edit in one step without an extra step for showing markers"
    let mut rt = TestEditor::new("**bold** after");
    rt.update(|s, _| s.edit.set_caret(s.edit.text.len()));
    rt.frame(0.0);
    type_str(&mut rt, "!");
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after!");

    rt.update(|s, _| s.edit.set_caret(0));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "**bold** after!");
    assert!(has_shown_source(&rt.state.edit));

    send_key(&mut rt, Key::Delete, Mods::default());
    send_key(&mut rt, Key::Delete, Mods::default());
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(composer_rich::markdown(&rt.state.edit), "bold** after!");
    assert!(rt.state.edit.marks().is_empty());

    // Undo restores the markers in one step; caret is at 0 so they reveal.
    cmd_z(&mut rt, false);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after!");
    assert!(has_shown_source(&rt.state.edit));
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 1);

    // Leaving hides them without touching the text undo needs.
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(rt.state.edit.text, "bold after!");
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 1);

    // Undo again removes "!".
    cmd_z(&mut rt, false);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after");
    assert_eq!(probe_redo_depth(&rt.state.edit, false), 2);

    // Moving the caret into styled text keeps redo, and adds no undo step after it.
    rt.update(|s, _| s.edit.set_caret(2));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "**bold** after");
    assert!(has_shown_source(&rt.state.edit));
    assert_eq!(probe_redo_depth(&rt.state.edit, false), 2);

    // Redo restores "!", then redo again restores "bold** after!".
    cmd_z(&mut rt, true);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after!");
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 1);

    rt.update(|s, _| s.edit.set_caret(0));
    rt.frame(0.0);
    cmd_z(&mut rt, true);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "bold** after!");
    assert!(!rt.state.edit.marks().iter().any(|m| m.mark == Mark::Bold));
    assert_eq!(probe_redo_depth(&rt.state.edit, true), 0);

    // Undo twice, redo once, leave, then type "?" and undo/redo it.
    cmd_z(&mut rt, false);
    cmd_z(&mut rt, false);
    cmd_z(&mut rt, true);
    send_key(&mut rt, Key::End, Mods::default());
    type_str(&mut rt, "?");
    cmd_z(&mut rt, false);
    cmd_z(&mut rt, true);
    assert_eq!(rt.state.edit.text, "bold after!?");
    rt.update(|s, _| s.edit.set_caret(0));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "**bold** after!?");
    assert!(has_shown_source(&rt.state.edit));

    send_key(&mut rt, Key::Delete, Mods::default());
    assert_eq!(composer_rich::markdown(&rt.state.edit), "*bold** after!?");
    cmd_z(&mut rt, false);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after!?");
    assert!(has_shown_source(&rt.state.edit));
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(rt.state.edit.text, "bold after!?");
    assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Bold));
}

#[test]
fn part_b_line_break_inside_revealed_bold_keeps_both_halves_bold() {
    // Ported from KSG ComposerPromptEditorTiptap.input.browser.test.tsx:
    // "a line break inside bold keeps both halves bold"
    let mut rt = TestEditor::new("**bold** after");
    rt.update(|s, _| s.edit.set_caret(2));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "**bold** after");
    assert_eq!(rt.state.edit.focus, 4);

    send_key(
        &mut rt,
        Key::Enter,
        Mods {
            shift: true,
            ..Mods::default()
        },
    );
    assert_eq!(
        composer_rich::markdown(&rt.state.edit),
        "**bo**\n**ld** after"
    );

    cmd_z(&mut rt, false);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after");

    cmd_z(&mut rt, true);
    assert_eq!(
        composer_rich::markdown(&rt.state.edit),
        "**bo**\n**ld** after"
    );
}

#[test]
fn part_b_splitting_list_item_inside_revealed_bold_keeps_both_halves_bold() {
    // Ported from KSG ComposerPromptEditorTiptap.input.browser.test.tsx:
    // "splitting a list item inside bold keeps both halves bold"
    let mut rt = TestEditor::new("- **bold** after");
    assert_eq!(rt.state.edit.block_kind(0), BlockKind::Bullet(0));
    rt.update(|s, _| s.edit.set_caret(2));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "**bold** after");
    assert_eq!(rt.state.edit.focus, 4);

    send_key(&mut rt, Key::Enter, Mods::default());
    assert_eq!(
        composer_rich::markdown(&rt.state.edit),
        "- **bo**\n- **ld** after"
    );

    cmd_z(&mut rt, false);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "- **bold** after");
}

#[test]
fn part_b_ksg_undoes_composition_in_revealed_markdown_through_earlier_paste() {
    // Ported from KSG ComposerPromptEditorTiptap.input.browser.test.tsx:
    // "undoes composition $at in revealed Markdown through the earlier paste"
    for (sel, composed, expect_bold) in [
        ((4, 4), "**bo漢ld** after", true),
        ((0, 2), "漢bold** after", false),
        ((8, 8), "**bold**漢 after", true),
    ] {
        let mut rt = TestEditor::new("");
        rt.update(|s, _| {
            composer_rich::paste(&mut s.edit, "**bold** after", &|_| None);
        });
        rt.frame(0.0);
        type_str(&mut rt, "!");
        cmd_z(&mut rt, false);
        assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after");

        rt.update(|s, _| s.edit.set_caret(2));
        rt.frame(0.0);
        assert_eq!(rt.state.edit.text, "**bold** after");
        assert!(has_shown_source(&rt.state.edit));

        // Canceled composition keeps redo and adds no undo.
        rt.update(move |s, _| s.edit.set_selection(sel.0, sel.1));
        rt.frame(0.0);
        rt.ime(Ime::Preedit(" ".into(), None));
        rt.frame(0.0);
        rt.ime(Ime::Preedit("".into(), None));
        rt.frame(0.0);
        assert_eq!(probe_redo_depth(&rt.state.edit, true), 1);

        // Actual composition commit records one step against the revealed/collapsed history.
        rt.ime(Ime::Preedit("漢".into(), None));
        rt.frame(0.0);
        rt.ime(Ime::Commit("漢".into()));
        rt.frame(0.0);
        send_key(&mut rt, Key::End, Mods::default());
        assert_eq!(composer_rich::markdown(&rt.state.edit), composed);

        cmd_z(&mut rt, false);
        assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after");
        cmd_z(&mut rt, false);
        assert_eq!(composer_rich::markdown(&rt.state.edit), "");

        cmd_z(&mut rt, true);
        cmd_z(&mut rt, true);
        send_key(&mut rt, Key::End, Mods::default());
        assert_eq!(composer_rich::markdown(&rt.state.edit), composed);
        assert_eq!(
            rt.state.edit.marks().iter().any(|m| m.mark == Mark::Bold),
            expect_bold
        );
    }
}

fn cmd_mod(rt: &mut Runtime<TestEditor>, ch: char, shift: bool) {
    send_key(
        rt,
        Key::Command(ch),
        Mods {
            cmd: true,
            shift,
            ..Mods::default()
        },
    );
}

#[test]
fn part_c_ksg_lets_the_caret_step_out_in_front_of_bold_that_starts_the_line() {
    // Unrevealable bold (`2*3`) tests the styled-edge two-stop navigation directly.
    let mut rt = TestEditor::new("");
    rt.update(|s, _| {
        s.edit = composer_rich::draft("2*3 tail");
        s.edit.set_marks(vec![MarkRange::new(0..3, Mark::Bold)]);
        s.edit.focused = true;
        s.edit.set_caret(0);
    });
    rt.frame(0.0);
    assert!(!has_shown_source(&rt.state.edit));
    // At pos 0 in Rune, default inherited marks are outside (`[]`).
    // ArrowRight steps into Bold without moving pos 0.
    send_key(&mut rt, Key::Right, Mods::default());
    assert_eq!(rt.state.edit.focus, 0);
    assert!(rt.state.edit.has_mark(&Mark::Bold));
    // ArrowLeft steps back outside without moving pos 0.
    send_key(&mut rt, Key::Left, Mods::default());
    assert_eq!(rt.state.edit.focus, 0);
    assert!(!rt.state.edit.has_mark(&Mark::Bold));
    // Typing here inserts plain text before bold.
    type_str(&mut rt, "x");
    assert_eq!(composer_rich::markdown(&rt.state.edit), "x**2*3** tail");
}

#[test]
fn part_c_ksg_lets_the_caret_step_out_after_bold_that_ends_the_line() {
    let mut rt = TestEditor::new("");
    rt.update(|s, _| {
        s.edit = composer_rich::draft("head 2*3");
        s.edit.set_marks(vec![MarkRange::new(5..8, Mark::Bold)]);
        s.edit.focused = true;
        s.edit.set_caret(8);
    });
    rt.frame(0.0);
    assert!(rt.state.edit.has_mark(&Mark::Bold));
    // ArrowRight steps out of Bold at end of line without moving pos 8.
    send_key(&mut rt, Key::Right, Mods::default());
    assert_eq!(rt.state.edit.focus, 8);
    assert!(!rt.state.edit.has_mark(&Mark::Bold));
    // Pressing ArrowRight again at doc end keeps the outside state.
    send_key(&mut rt, Key::Right, Mods::default());
    assert_eq!(rt.state.edit.focus, 8);
    assert!(!rt.state.edit.has_mark(&Mark::Bold));
    // ArrowLeft steps back inside Bold.
    send_key(&mut rt, Key::Left, Mods::default());
    assert_eq!(rt.state.edit.focus, 8);
    assert!(rt.state.edit.has_mark(&Mark::Bold));
    // Step out again and type plain text.
    send_key(&mut rt, Key::Right, Mods::default());
    type_str(&mut rt, "x");
    assert_eq!(composer_rich::markdown(&rt.state.edit), "head **2*3**x");
}

#[test]
fn part_c_ksg_offers_both_stops_where_styled_text_meets_plain_text_mid_line() {
    let mut rt = TestEditor::new("");
    rt.update(|s, _| {
        s.edit = composer_rich::draft("a 2*3 c");
        s.edit.set_marks(vec![MarkRange::new(2..5, Mark::Bold)]);
        s.edit.focused = true;
        s.edit.set_caret(2);
    });
    rt.frame(0.0);
    assert!(!rt.state.edit.has_mark(&Mark::Bold));
    send_key(&mut rt, Key::Right, Mods::default());
    assert_eq!(rt.state.edit.focus, 2);
    assert!(rt.state.edit.has_mark(&Mark::Bold));
    type_str(&mut rt, "x");
    assert_eq!(composer_rich::markdown(&rt.state.edit), "a **x2*3** c");
}

#[test]
fn part_c_ksg_offers_the_plain_stop_between_bold_text_and_a_chip_right_after_it() {
    let token = crate::composer_tokens::Token::File {
        path: "README.md".into(),
        directory: false,
        source: Some("@README.md".into()),
    };
    let mut rt = TestEditor::new("");
    rt.update(|s, _| {
        s.edit = composer_rich::draft("2*3@README.md");
        s.edit.replace_with_pill(3..13, token.id(), "@README.md");
        s.edit.set_selection(0, 3);
        s.edit.set_mark(Mark::Bold);
        s.edit.focused = true;
        s.edit.set_caret(3);
    });
    rt.frame(0.0);
    assert!(rt.state.edit.has_mark(&Mark::Bold));
    send_key(&mut rt, Key::Right, Mods::default());
    assert_eq!(rt.state.edit.focus, 3);
    assert!(!rt.state.edit.has_mark(&Mark::Bold));
    type_str(&mut rt, "x");
    assert_eq!(
        composer_rich::markdown(&rt.state.edit),
        "**2*3**x@README.md"
    );
}

#[test]
fn part_c_ksg_steps_out_of_inline_code_at_the_end_of_a_line_without_inserting_a_space() {
    let mut edit = composer_rich::draft("code");
    edit.set_marks(vec![MarkRange::new(0..4, Mark::Code)]);
    edit.set_caret(4);
    assert!(edit.has_mark(&Mark::Code));
    assert!(step_caret_across_styled_edge(&mut edit, 1));
    assert!(!edit.has_mark(&Mark::Code));
    assert!(!step_caret_across_styled_edge(&mut edit, 1));
    edit.insert("x");
    assert_eq!(composer_rich::markdown(&edit), "`code`x");
}

#[test]
fn part_c_ksg_keeps_marks_the_user_toggled_at_an_edge_and_lets_the_arrow_move() {
    let mut edit = composer_rich::draft("a b c");
    edit.set_marks(vec![MarkRange::new(2..3, Mark::Bold)]);
    edit.set_caret(2);
    edit.set_mark(Mark::Bold);
    edit.set_mark(Mark::Italic);
    assert!(!step_caret_across_styled_edge(&mut edit, 1));
    assert!(!step_caret_across_styled_edge(&mut edit, -1));
}

#[test]
fn part_c_ksg_formats_a_filled_pair_leaves_following_text_plain_and_keeps_normal_input_rule() {
    for prefix in ["", "prefix ", "@README.md", "a ``` "] {
        let mut rt = TestEditor::new("");
        if prefix == "@README.md" {
            let token = crate::composer_tokens::Token::File {
                path: "README.md".into(),
                directory: false,
                source: Some("@README.md".into()),
            };
            rt.update(|s, _| {
                s.edit.replace_with_pill(0..0, token.id(), "@README.md");
                s.source.prev_text = s.edit.text.clone();
            });
            rt.frame(0.0);
        } else if !prefix.is_empty() {
            type_str(&mut rt, prefix);
        }
        let offset = rt.state.edit.focus;
        type_str(&mut rt, "``");
        assert_eq!(
            rt.state.source.typed_backtick_pairs.last(),
            Some(&(offset..offset + 2)),
            "{prefix}"
        );
        rt.update(move |s, _| s.edit.set_caret(offset + 1));
        rt.frame(0.0);
        type_str(&mut rt, "hello world");
        assert_eq!(
            composer_rich::markdown(&rt.state.edit),
            format!("{prefix}`hello world`")
        );
        // Caret exits at the closing tick: converts to inline code (and reveals at the edge).
        rt.update(move |s, _| s.edit.set_caret(offset + 13));
        rt.frame(0.0);
        assert_eq!(
            composer_rich::markdown(&rt.state.edit),
            format!("{prefix}`hello world`")
        );
        type_str(&mut rt, " next `typed`");
        assert_eq!(
            composer_rich::markdown(&rt.state.edit),
            format!("{prefix}`hello world` next `typed`")
        );
        // Move past `typed` so everything collapses cleanly.
        type_str(&mut rt, " ");
        assert_eq!(
            composer_rich::markdown(&rt.state.edit),
            format!("{prefix}`hello world` next `typed` ")
        );
        assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Code));
    }
}

#[test]
fn part_c_ksg_formats_or_keeps_literal_a_pair_whose_backticks_came_from_a_dead_key() {
    // 1. Dead-key backtick pair converts to inline code on exit.
    let mut rt = TestEditor::new("");
    type_str(&mut rt, "do ");
    rt.ime(Ime::Preedit("`".into(), None));
    rt.frame(0.0);
    rt.ime(Ime::Commit("`".into()));
    rt.frame(0.0);
    rt.ime(Ime::Preedit("`".into(), None));
    rt.frame(0.0);
    rt.ime(Ime::Commit("`".into()));
    rt.frame(0.0);
    rt.update(|s, _| s.edit.set_caret(4));
    rt.frame(0.0);
    type_str(&mut rt, "xyz");
    rt.update(|s, _| s.edit.set_caret(8));
    rt.frame(0.0);
    type_str(&mut rt, " ");
    assert_eq!(composer_rich::markdown(&rt.state.edit), "do `xyz` ");
    assert_eq!(rt.state.edit.text, "do xyz ");
    assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Code));

    // 2. Pair after an unmatched backtick stays literal when composed.
    let mut rt = TestEditor::new("");
    rt.update(|s, _| {
        s.edit.paste("a ` ");
        s.source.on_paste(&s.edit);
    });
    rt.frame(0.0);
    rt.update(|s, _| {
        s.source.was_composing = true;
        s.edit.insert("`");
        s.source.on_doc_change(&s.edit);
    });
    rt.frame(0.0);
    rt.update(|s, _| {
        s.source.was_composing = true;
        s.edit.insert("`");
        s.source.on_doc_change(&s.edit);
    });
    rt.frame(0.0);
    assert!(rt.state.source.typed_backtick_pairs.is_empty());
    rt.update(|s, _| s.edit.set_caret(5));
    rt.frame(0.0);
    type_str(&mut rt, "xyz");
    rt.update(|s, _| s.edit.set_caret(9));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "a ` `xyz`");
    assert!(rt.state.edit.marks().is_empty());
}

#[test]
fn part_c_ksg_keeps_composition_edits_literal_until_the_caret_leaves_the_typed_pair() {
    let mut rt = TestEditor::new("");
    type_str(&mut rt, "``");
    rt.update(|s, _| s.edit.set_caret(1));
    rt.frame(0.0);
    rt.ime(Ime::Preedit("世".into(), None));
    rt.frame(0.0);
    rt.ime(Ime::Commit("世界".into()));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "`世界`");
    assert!(rt.state.edit.marks().is_empty());
    let end = rt.state.edit.text.len();
    rt.update(move |s, _| s.edit.set_caret(end));
    rt.frame(0.0);
    assert_eq!(composer_rich::markdown(&rt.state.edit), "`世界`");
    type_str(&mut rt, " ");
    assert_eq!(rt.state.edit.text, "世界 ");
    assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Code));
}

#[test]
fn part_c_ksg_keeps_a_paste_or_set_content_pair_literal_after_moving_through_it() {
    // Plain paste case.
    let mut rt = TestEditor::new("");
    rt.update(|s, _| {
        s.edit.paste("`literal`");
        s.source.on_paste(&s.edit);
    });
    rt.frame(0.0);
    rt.update(|s, _| s.edit.set_caret(1));
    rt.frame(0.0);
    rt.update(|s, _| s.edit.set_caret(9));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "`literal`");
    assert!(rt.state.edit.marks().is_empty());
}

#[test]
fn part_c_ksg_leaves_doubled_delimiters_and_code_contexts_unchanged_on_caret_movement() {
    for (delimiters, caret, mark, code_block) in [
        ("````", 2usize, None, false),
        ("```", 1, None, false),
        ("``", 1, Some(Mark::Bold), false),
        ("``", 1, Some(Mark::Code), false),
        ("``", 1, None, true),
    ] {
        let mut rt = TestEditor::new("");
        if code_block {
            rt.update(|s, _| {
                let _ = s.edit.set_logical_block(
                    0..0,
                    BlockKind::Code,
                    rune::BlockAttributes::default()
                        .with_code(rune::CodeAttributes::new("```", "")),
                );
            });
            rt.frame(0.0);
        }
        if let Some(m) = mark {
            rt.update(move |s, _| s.edit.set_mark(m));
            rt.frame(0.0);
        }
        type_str(&mut rt, delimiters);
        rt.update(move |s, _| s.edit.set_caret(caret));
        rt.frame(0.0);
        type_str(&mut rt, "literal");
        let before_text = rt.state.edit.text.clone();
        rt.update(move |s, _| s.edit.set_caret(caret + "literal".len() + 1));
        rt.frame(0.0);
        assert_eq!(rt.state.edit.text, before_text, "{delimiters}");
        rt.update(|s, _| s.edit.set_caret(s.edit.text.len()));
        rt.frame(0.0);
        assert_eq!(rt.state.edit.text, before_text, "{delimiters}");
    }
}

#[test]
fn part_c_ksg_toggles_bold_and_italic_with_mod_chord_at_a_collapsed_cursor() {
    for (ch, marker) in [('b', "**"), ('i', "*")] {
        let mut rt = TestEditor::new("ship it");
        rt.update(|s, _| s.edit.set_caret(s.edit.text.len()));
        rt.frame(0.0);
        type_str(&mut rt, " ");
        cmd_mod(&mut rt, ch, false);
        type_str(&mut rt, "now");
        cmd_mod(&mut rt, ch, false);
        type_str(&mut rt, " done");
        assert_eq!(
            composer_rich::markdown(&rt.state.edit),
            format!("ship it {marker}now{marker} done")
        );
    }
}

#[test]
fn part_c_ksg_mod_chord_at_caret_in_a_bold_word_styles_the_next_typed_text() {
    // Mod+i inside `**n|ow**` -> `**n*X*ow**`
    let mut rt = TestEditor::new("ship it **now** done");
    rt.update(|s, _| s.edit.set_caret(9));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "ship it **now** done");
    cmd_mod(&mut rt, 'i', false);
    type_str(&mut rt, "X");
    assert_eq!(
        composer_rich::markdown(&rt.state.edit),
        "ship it **n*X*ow** done"
    );

    // Mod+i just before `|**now**` -> `*X***now**`
    let mut rt = TestEditor::new("ship it **now** done");
    rt.update(|s, _| s.edit.set_caret(8));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "ship it **now** done");
    assert_eq!(rt.state.edit.focus, 8);
    cmd_mod(&mut rt, 'i', false);
    type_str(&mut rt, "X");
    assert_eq!(
        composer_rich::markdown(&rt.state.edit),
        "ship it *X***now** done"
    );

    // Mod+e just after `**now**|` -> `**now**`X``
    let mut rt = TestEditor::new("ship it **now** done");
    rt.update(|s, _| s.edit.set_caret(s.edit.text.len()));
    rt.frame(0.0);
    for _ in 0..5 {
        send_key(&mut rt, Key::Left, Mods::default());
    }
    assert_eq!(rt.state.edit.text, "ship it **now** done");
    assert_eq!(rt.state.edit.focus, 15);
    cmd_mod(&mut rt, 'e', false);
    type_str(&mut rt, "X");
    assert_eq!(
        composer_rich::markdown(&rt.state.edit),
        "ship it **now**`X` done"
    );
}

#[test]
fn part_c_ksg_mod_b_over_selection_after_prefix_and_removes_bold() {
    for (word, prefix, should_reveal) in [
        ("ship", "", true),
        ("path\\", "", false),
        ("2*3", "", false),
        ("b", "*", false),
        ("word", "\\\\*", false),
        ("word", "\\\\**", false),
        ("word", "\\", false),
        ("word", "\\\\", true),
        ("word", "\\\\\\", false),
        ("word", "`path\\`", true),
    ] {
        let initial = format!("{prefix}{word} it");
        let mut rt = TestEditor::new(&initial);
        let prefix_len = rt.state.edit.text.find(word).unwrap();
        rt.update(move |s, _| s.edit.set_selection(prefix_len, prefix_len + word.len()));
        rt.frame(0.0);
        cmd_mod(&mut rt, 'b', false);
        assert_eq!(
            composer_rich::markdown(&rt.state.edit),
            format!("{prefix}**{word}** it"),
            "prefix={prefix:?}"
        );
        // Decision 9: non-collapsed selection does not reveal, so `should_reveal` is checked
        // when collapsing caret inside the bolded word, keeping redo/marks intact.
        let end = rt.state.edit.text.len();
        rt.update(move |s, _| s.edit.set_caret(prefix_len + 1));
        rt.frame(0.0);
        assert_eq!(
            has_shown_source(&rt.state.edit),
            should_reveal,
            "prefix={prefix:?}"
        );
        rt.update(move |s, _| s.edit.set_caret(end));
        rt.frame(0.0);

        // Selecting the word (or shown markers) and pressing Mod+B removes bold.
        let span_end = if should_reveal {
            rt.update(move |s, _| s.edit.set_caret(prefix_len + 1));
            rt.frame(0.0);
            prefix_len + word.len() + 4
        } else {
            prefix_len + word.len()
        };
        rt.update(move |s, _| s.edit.set_selection(prefix_len, span_end));
        rt.frame(0.0);
        cmd_mod(&mut rt, 'b', false);
        assert_eq!(composer_rich::markdown(&rt.state.edit), initial);
    }
}

#[test]
fn part_c_ksg_edits_the_markers_as_text() {
    // 1. deleting the opening markers removes bold
    let mut rt = TestEditor::new("**bold** after");
    rt.update(|s, _| s.edit.set_caret(0));
    rt.frame(0.0);
    send_key(&mut rt, Key::Delete, Mods::default());
    send_key(&mut rt, Key::Delete, Mods::default());
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(composer_rich::markdown(&rt.state.edit), "bold** after");
    assert!(rt.state.edit.marks().is_empty());

    // 2. turning ** into * makes it italic
    let mut rt = TestEditor::new("**bold** after");
    rt.update(|s, _| s.edit.set_caret(0));
    rt.frame(0.0);
    send_key(&mut rt, Key::Delete, Mods::default());
    for _ in 0..5 {
        send_key(&mut rt, Key::Right, Mods::default());
    }
    send_key(&mut rt, Key::Delete, Mods::default());
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(composer_rich::markdown(&rt.state.edit), "*bold* after");
    assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Italic));
    assert!(!rt.state.edit.marks().iter().any(|m| m.mark == Mark::Bold));

    // 3. backspacing from the closing edge keeps italic
    let mut rt = TestEditor::new("**bold** after");
    rt.update(|s, _| s.edit.set_caret(0));
    rt.frame(0.0);
    send_key(&mut rt, Key::Delete, Mods::default());
    for _ in 0..6 {
        send_key(&mut rt, Key::Right, Mods::default());
    }
    send_key(&mut rt, Key::Backspace, Mods::default());
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(composer_rich::markdown(&rt.state.edit), "*bold* after");
    assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Italic));

    // 4. retyping deleted closing markers restores bold
    let mut rt = TestEditor::new("**bold** after");
    rt.update(|s, _| s.edit.set_caret(2));
    rt.frame(0.0);
    send_key(&mut rt, Key::Right, Mods::default());
    send_key(&mut rt, Key::Right, Mods::default());
    send_key(&mut rt, Key::Delete, Mods::default());
    send_key(&mut rt, Key::Delete, Mods::default());
    type_str(&mut rt, "**");
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(composer_rich::markdown(&rt.state.edit), "**bold** after");
    assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Bold));

    // 5. deleting a backtick removes inline code
    let mut rt = TestEditor::new("`code` after");
    rt.update(|s, _| s.edit.set_caret(0));
    rt.frame(0.0);
    send_key(&mut rt, Key::Delete, Mods::default());
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(composer_rich::markdown(&rt.state.edit), "code` after");
    assert!(rt.state.edit.marks().is_empty());
}

#[test]
fn part_c_ksg_shows_nested_formatting_as_markdown_and_restores_when_caret_leaves() {
    let value = "**a *b* c** d";
    let mut rt = TestEditor::new(value);
    rt.update(|s, _| s.edit.set_caret(3));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, value);
    assert!(!rt.state.edit.marks().iter().any(|m| m.mark == Mark::Bold));
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(rt.state.edit.text, "a b c d");
    assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Bold));
    assert!(rt.state.edit.marks().iter().any(|m| m.mark == Mark::Italic));
    assert_eq!(composer_rich::markdown(&rt.state.edit), value);
}

#[test]
fn part_c_ksg_keeps_a_styled_chip_selected_for_backspace_or_text_when_showing_source() {
    let token = crate::composer_tokens::Token::Skill {
        name: "babysit".into(),
        harness: "test".into(),
    };
    for (is_backspace, expected) in [(true, "**** after"), (false, "**X** after")] {
        let mut rt = TestEditor::new("");
        rt.update(|s, _| {
            s.edit = composer_rich::draft("$babysit after");
            s.edit.replace_with_pill(0..8, token.id(), "$babysit");
            s.edit.set_selection(0, 8);
            s.edit.set_mark(Mark::Bold);
            s.edit.focused = true;
            s.edit.set_caret(s.edit.text.len());
        });
        rt.frame(0.0);
        // Move caret to 0 to trigger reveal via `on_input_selection_change`, then select the pill.
        rt.update(|s, _| s.edit.set_caret(0));
        rt.frame(0.0);
        assert_eq!(rt.state.edit.text, "**$babysit** after");
        rt.update(|s, _| s.edit.set_selection(2, 10));
        rt.frame(0.0);
        assert!(has_shown_source(&rt.state.edit));
        if is_backspace {
            send_key(&mut rt, Key::Backspace, Mods::default());
        } else {
            type_str(&mut rt, "X");
        }
        send_key(&mut rt, Key::End, Mods::default());
        assert_eq!(composer_rich::markdown(&rt.state.edit), expected);
    }
}

#[test]
fn part_c_ksg_keeps_bold_around_a_skill_pasted_inside_it() {
    let token = crate::composer_tokens::Token::Skill {
        name: "babysit".into(),
        harness: "test".into(),
    };
    let resolver = |src: &str| (src == "$babysit").then(|| (token.id(), "$babysit".into()));
    let mut rt = TestEditor::new("**helloworld** after");
    rt.update(|s, _| s.edit.set_caret(5));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, "**helloworld** after");
    assert_eq!(rt.state.edit.focus, 7);

    rt.update(|s, _| {
        collapse_shown_markdown_source(&mut s.edit);
        s.source.on_paste(&s.edit);
        composer_rich::paste(&mut s.edit, "$babysit", &resolver);
    });
    rt.frame(0.0);
    send_key(&mut rt, Key::End, Mods::default());
    assert_eq!(
        composer_rich::markdown(&rt.state.edit),
        "**hello** $babysit **world** after"
    );
}

#[test]
fn part_c_ksg_arabic_line_walks_back_through_inline_code_and_shows_backticks() {
    let value = "اكتب `sumBy` الان";
    let mut rt = TestEditor::new(value);
    rt.update(|s, _| s.edit.set_caret(s.edit.text.len()));
    rt.frame(0.0);
    let mut saw_source = false;
    let mut steps = 0usize;
    while rt.state.edit.focus > "اكتب".len() && steps < 32 {
        send_key(&mut rt, Key::Right, Mods::default());
        saw_source |= rt.state.edit.text == value;
        steps += 1;
    }
    assert!(saw_source);
    assert_eq!(composer_rich::markdown(&rt.state.edit), value);
    // Re-enter code and check `sumBy` decorations include faint backticks + mono code.
    let mid = value.find("sum").unwrap() + 1;
    rt.update(move |s, _| s.edit.set_caret(mid));
    rt.frame(0.0);
    assert_eq!(rt.state.edit.text, value);
    let (spans, _) = preview_decorations(
        &rt.state.edit,
        TextStyle::new(15.0, 400.0),
        Color::from_oklab([0.5, 0.0, 0.0, 1.0]),
        Color::WHITE,
        Color::BLACK,
    );
    let tick1 = value.find('`').unwrap();
    let tick2 = value.rfind('`').unwrap();
    assert!(
        spans
            .iter()
            .any(|s| s.start as usize == tick1 && s.end as usize == tick1 + 1)
    );
    assert!(
        spans
            .iter()
            .any(|s| s.start as usize == tick1 + 1 && s.end as usize == tick2 && s.mono)
    );
    assert!(
        spans
            .iter()
            .any(|s| s.start as usize == tick2 && s.end as usize == tick2 + 1)
    );
}

#[test]
fn part_c_arabic_bidi_key_sequence_settles_with_bounded_frames() {
    let value = "اكتب `sumBy` الان";
    let mut rt = TestEditor::new(value);
    rt.update(|s, _| s.edit.set_caret(s.edit.text.len()));
    rt.frame(0.0);

    // Walk across the RTL+LTR line with both Key::Left and Key::Right for bounded steps,
    // verifying each frame settles and on_selection_change is idempotent and re-entry safe.
    for key in [Key::Left, Key::Right, Key::Right, Key::Left] {
        for _ in 0..20 {
            rt.key(KeyEvent {
                key: key.clone(),
                mods: Mods::default(),
            });
            let mut frames = 0usize;
            while frames < 4 {
                rt.frame(0.0);
                frames += 1;
            }
            assert!(frames <= 4);

            // Calling on_selection_change again at the settled caret must be a complete no-op.
            let before_edit = rt.state.edit.clone();
            let before_state = rt.state.source.clone();
            on_selection_change(&mut rt.state.source, &mut rt.state.edit);
            assert_eq!(rt.state.edit, before_edit);
            assert_eq!(rt.state.source, before_state);
        }
    }

    // Re-entrancy guard: invoking on_selection_change while `normalizing` is true is a no-op.
    rt.update(|s, _| s.edit.set_caret(value.find("sum").unwrap()));
    rt.frame(0.0);
    let mut edit = state_from("**bold**");
    edit.set_caret(2);
    let mut st = MarkdownSourceState {
        normalizing: true,
        ..Default::default()
    };
    on_selection_change(&mut st, &mut edit);
    assert!(!has_shown_source(&edit));
    assert_eq!(edit.text, "bold");
}
