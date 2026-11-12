//! Cases copied from KSG's composer-rich-text.test.ts, composer-rich-text-doc.test.ts
//! and composer-code-block.test.ts. Offsets here are UTF-8 bytes, not PM positions.
use super::*;

fn type_text(edit: &mut TextEdit, rules: &mut Rules, text: &str) {
    for ch in text.chars() {
        press(edit, rules, Key::Text(ch.to_string()), false);
    }
}

fn press(edit: &mut TextEdit, rules: &mut Rules, key: Key, shift: bool) {
    let event = KeyEvent {
        key,
        mods: rune::Mods {
            shift,
            ..Default::default()
        },
    };
    if !rules.key(edit, &event) {
        edit.key(&event, None);
        rules.changed(edit);
    }
}

#[test]
fn ksg_input_mark_rules_and_literal_exclusions() {
    for (source, text, expected, mark) in [
        ("**bold**", "bold", "**bold**", Some(Mark::Bold)),
        ("__bold__", "bold", "**bold**", Some(Mark::Bold)),
        ("*italic*", "italic", "*italic*", Some(Mark::Italic)),
        ("~~strike~~", "strike", "~~strike~~", Some(Mark::Strike)),
        (
            "`**not bold**`",
            "**not bold**",
            "`**not bold**`",
            Some(Mark::Code),
        ),
        (
            "_build node_modules",
            "_build node_modules",
            "_build node_modules",
            None,
        ),
        ("a*word*", "a*word*", "a*word*", None),
        ("** spaced **", " spaced ", " **spaced** ", Some(Mark::Bold)),
        ("** **", "** **", "** **", None),
        ("``literal``", "``literal``", "``literal``", None),
    ] {
        let mut edit = draft("");
        type_text(&mut edit, &mut Rules::default(), source);
        assert_eq!(edit.text, text, "{source}");
        assert_eq!(markdown(&edit), expected, "{source}");
        if let Some(mark) = mark {
            assert!(edit.marks().iter().any(|m| m.mark == mark), "{source}");
        } else {
            assert!(edit.marks().is_empty(), "{source}");
        }
    }
}

#[test]
fn ksg_formatting_chords() {
    for (ch, shift, mark) in [
        ('b', false, Mark::Bold),
        ('b', true, Mark::Bold),
        ('i', false, Mark::Italic),
        ('i', true, Mark::Italic),
        ('e', false, Mark::Code),
        ('s', true, Mark::Strike),
    ] {
        let mut edit = draft("one");
        edit.set_selection(0, 3);
        let mut rules = Rules::default();
        let event = KeyEvent {
            key: Key::Command(ch),
            mods: rune::Mods {
                cmd: true,
                shift,
                ..Default::default()
            },
        };
        assert!(rules.key(&mut edit, &event));
        assert!(edit.has_mark(&mark));
        assert!(rules.key(&mut edit, &event));
        assert!(edit.marks().is_empty());
    }
}

#[test]
fn ksg_surround_selection_and_round_trip_guard() {
    for (marker, selected, expected, marked) in [
        ("`", "hello", "say `hello` now", true),
        ("*", "hello", "say *hello* now", true),
        ("_", "hello", "say *hello* now", true),
        ("`", "   ", "say `   ` now", true),
        ("*", "   ", "say *   * now", false),
        ("_", "   ", "say _   _ now", false),
        ("*", "a*b", "say *a*b* now", false),
        ("`", "foo`bar", "say `foo`bar` now", false),
        ("(", "hello", "say (hello) now", false),
    ] {
        let mut edit = draft(format!("say {selected} now"));
        edit.set_selection(4, 4 + selected.len());
        let mut rules = Rules::default();
        press(&mut edit, &mut rules, Key::Text(marker.into()), false);
        assert_eq!(markdown(&edit), expected);
        assert_eq!(edit.selected(), selected);
        assert_eq!(!edit.marks().is_empty(), marked);
    }
}

#[test]
fn ksg_serializes_marks_in_stable_order_and_keeps_code_whitespace() {
    let mut edit = draft(" a ` b ");
    edit.set_selection(0, edit.text.len());
    for mark in [Mark::Italic, Mark::Bold, Mark::Strike, Mark::Code] {
        edit.set_mark(mark);
    }
    // KSG deliberately uses single backticks, even for code containing backticks.
    assert_eq!(markdown(&edit), "~~***` a ` b `***~~");

    let mut edit = draft("quote\n\nprose");
    edit.set_blocks(vec![Block::Quote, Block::Paragraph, Block::Paragraph]);
    assert_eq!(markdown(&edit), "> quote\n\nprose");
}

#[test]
fn ksg_code_tab_and_newline() {
    let mut edit = draft("  first\n\tsecond\n\n third");
    edit.set_blocks(vec![Block::Code; 4]);
    edit.set_selection(0, edit.text.len());
    let mut rules = Rules::default();
    press(&mut edit, &mut rules, Key::Tab, false);
    assert_eq!(edit.text, "    first\n  \tsecond\n\n   third");
    press(&mut edit, &mut rules, Key::Tab, true);
    assert_eq!(edit.text, "  first\n\tsecond\n\n third");
    edit.set_caret(7);
    press(&mut edit, &mut rules, Key::Enter, false);
    assert_eq!(edit.text, "  first\n  \n\tsecond\n\n third");
    assert!(edit.blocks().iter().all(|b| *b == Block::Code));
}

#[test]
fn ksg_code_closing_fence_and_two_trailing_blank_lines() {
    for (text, code) in [
        ("code\n```", "code"),
        ("code\n\n", "code"),
        ("code\n \n  ", "code"),
    ] {
        let mut edit = draft(text);
        edit.set_blocks(vec![Block::Code; text.lines().count() + 1]);
        edit.set_caret(text.len());
        press(&mut edit, &mut Rules::default(), Key::Enter, false);
        assert_eq!(edit.text, format!("{code}\n"));
        assert_eq!(edit.active_block(), Block::Paragraph);
        assert_eq!(markdown(&edit), format!("```\n{code}\n```\n"));
    }
    let mut edit = draft("code\n");
    edit.set_blocks(vec![Block::Code; 2]);
    edit.set_caret(edit.text.len());
    press(&mut edit, &mut Rules::default(), Key::Enter, false);
    assert_eq!(edit.active_block(), Block::Code);
    assert_eq!(edit.text, "code\n\n");
}

#[test]
fn ksg_formats_code_when_opening_tick_is_typed_last() {
    for prefix in ["", "this is a "] {
        let mut edit = draft(format!("{prefix}test` after"));
        edit.set_caret(prefix.len());
        let mut rules = Rules::default();
        type_text(&mut edit, &mut rules, "`");
        assert_eq!(markdown(&edit), format!("{prefix}`test` after"));
        assert_eq!(edit.focus, prefix.len());
        type_text(&mut edit, &mut rules, "new ");
        assert_eq!(markdown(&edit), format!("{prefix}`new test` after"));
    }
}
