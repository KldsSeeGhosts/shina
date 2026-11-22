use super::*;
use crate::composer_rich::{self, Rules};
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, text_input};

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
