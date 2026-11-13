//! Cases copied from KSG's composer-rich-text.test.ts, composer-rich-text-doc.test.ts
//! and composer-code-block.test.ts. Offsets here are UTF-8 bytes, not PM positions.
use super::*;
use rune::{BlockKind, EditorSelection};

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

#[test]
fn ksg_paste_cases_parse_markdown_and_preserve_code_literal() {
    for source in [
        "hello **bold** world",
        "a *italic* word and `code` here",
        "struck ~~out~~ now",
        "**`x`**",
        "*`x`*",
        "~~`x`~~",
        "**a `code` c**",
        "***bold italic*** keeps nesting",
        "*a **b** c*",
        "*a**b***",
        "**a*b***",
        "**a *b* c**",
        "line one\nline two",
        "trailing newline\n",
        "1. foo\n2. asdf\n",
        "snake_case stays literal",
        "unmatched ** stays literal",
    ] {
        let mut edit = draft("");
        paste(&mut edit, source, &|_| None);
        assert_eq!(markdown(&edit), source);
        if source.contains("bold") || source.contains("`x`") {
            assert!(!edit.marks().is_empty(), "{source}");
        }
        edit.undo();
        assert_eq!(edit.text, "", "{source}");
        edit.redo();
        assert_eq!(markdown(&edit), source);
    }
    let mut edit = draft("code: ");
    edit.set_block(Block::Code);
    paste(&mut edit, "**bold**\r\n@file $skill", &|_| {
        Some(("pill".into(), "label".into()))
    });
    assert_eq!(edit.text, "code: **bold**\n@file $skill");
    assert!(edit.marks().is_empty());
}

#[test]
fn ksg_pasted_chips_share_marks_and_get_boundary_spaces() {
    let token = crate::composer_tokens::Token::Skill {
        name: "my-skill".into(),
        harness: "test".into(),
    };
    let resolver = |source: &str| (source == "$my-skill").then(|| (token.id(), "my-skill".into()));
    let mut edit = draft("before");
    paste(&mut edit, "$my-skill", &resolver);
    assert_eq!(markdown(&edit), "before $my-skill ");
    assert_eq!(edit.pills().count(), 1);
    let mut edit = draft("");
    paste(&mut edit, "**before $my-skill after** $unknown", &resolver);
    assert_eq!(markdown(&edit), "**before $my-skill after** $unknown");
    assert_eq!(edit.pills().count(), 1);
    let pill = edit.pills().next().unwrap().0;
    assert!(
        edit.marks()
            .iter()
            .any(|m| m.mark == Mark::Bold && m.start <= pill.start && pill.end <= m.end)
    );
}

#[test]
fn ksg_fence_serialization_only_lengthens_for_actual_closing_lines() {
    for (text, expected) in [
        ("", "```\n```"),
        ("a ``` inline", "```\na ``` inline\n```"),
        ("```suffix", "```\n```suffix\n```"),
        ("a\n```", "````\na\n```\n````"),
        ("a\n````  ", "`````\na\n````  \n`````"),
    ] {
        let mut edit = draft(text);
        edit.set_blocks(vec![Block::Code; text.split('\n').count()]);
        assert_eq!(markdown(&edit), expected);
    }
}

#[test]
fn ksg_shift_enter_splits_blocks_instead_of_inserting_soft_breaks() {
    let mut edit = draft("bold");
    edit.set_selection(0, 4);
    edit.set_mark(Mark::Bold);
    edit.set_caret(2);
    press(&mut edit, &mut Rules::default(), Key::Enter, true);
    assert_eq!(markdown(&edit), "**bo**\n**ld**");
    assert!(edit.soft_breaks().is_empty());
}

#[test]
fn ksg_path_query_paste_keeps_the_path_literal_but_styles_following_text() {
    for path in [
        "src/__test__.tsx",
        "src/__tests__/a.ts",
        "pkg/__init__.py",
        "a/**b**/c",
    ] {
        let mut edit = draft("Open @");
        paste_with_path_query(&mut edit, path, true, &|_| None);
        assert_eq!(markdown(&edit), format!("Open @{path}"));
        assert!(edit.marks().is_empty());
    }
    let mut edit = draft("Open @");
    paste_with_path_query(&mut edit, "src/__test__.tsx and **this**", true, &|_| None);
    assert_eq!(markdown(&edit), "Open @src/__test__.tsx and **this**");
    assert_eq!(edit.marks().len(), 1);
    let mut edit = draft("Open @");
    paste_with_path_query(&mut edit, "__hello__ world", false, &|_| None);
    assert_eq!(markdown(&edit), "Open @**hello** world");
    let mut edit = draft("Open @src/");
    edit.set_selection(6, edit.text.len());
    edit.set_mark(Mark::Bold);
    edit.set_caret(edit.text.len());
    paste_with_path_query(&mut edit, "__test__.tsx", true, &|_| None);
    assert_eq!(markdown(&edit), "Open @**src/**__test__.tsx");
}

#[test]
fn ksg_code_delimiters_do_not_pair_with_an_earlier_literal_closer() {
    let mut edit = draft("is `enabled`");
    let mut rules = Rules::default();
    type_text(&mut edit, &mut rules, " and `");
    assert_eq!(edit.text, "is `enabled` and `");
    assert!(edit.marks().is_empty());
    type_text(&mut edit, &mut rules, "persist`");
    assert_eq!(edit.text, "is `enabled` and persist");
    assert_eq!(markdown(&edit), "is `enabled` and `persist`");
}

#[test]
fn ksg_emphasis_delimiters_inside_earlier_code_stay_literal() {
    for (open, mark) in [
        ("*", Mark::Italic),
        ("**", Mark::Bold),
        ("~~", Mark::Strike),
    ] {
        let mut edit = draft(format!("{open}foo bar `"));
        edit.set_selection(0, open.len() + 3);
        edit.set_mark(Mark::Code);
        edit.set_caret(edit.text.len());
        type_text(&mut edit, &mut Rules::default(), open);
        assert!(!edit.marks().iter().any(|m| m.mark == mark));
    }
}

#[test]
fn ksg_block_paste_splits_prose_at_start_middle_and_end() {
    for (at, source, expected) in [
        (0, "- item", "- item\nabcd"),
        (2, "- item", "ab\n- item\ncd"),
        (4, "- item", "abcd\n- item"),
        (0, "X\nY", "X\nY\nabcd"),
        (2, "X\nY", "ab\nX\nY\ncd"),
        (4, "X\nY", "abcd\nX\nY"),
        (2, "```\ncode\n```", "ab\n```\ncode\n```\ncd"),
    ] {
        let mut edit = draft("abcd");
        edit.set_caret(at);
        paste(&mut edit, source, &|_| None);
        assert_eq!(markdown(&edit), expected, "{at}: {source}");
        edit.undo();
        assert_eq!(markdown(&edit), "abcd");
    }
}

fn resolve_readme(source: &str) -> Option<(String, String)> {
    (source == "@README.md").then(|| {
        (
            crate::composer_tokens::Token::File {
                path: "README.md".into(),
                directory: false,
                source: Some("@README.md".into()),
            }
            .id(),
            "README.md".into(),
        )
    })
}

#[test]
fn ksg_headings_round_trip() {
    for source in [
        "# Heading",
        "## Two",
        "###### Six",
        "####### seven hashes stays a paragraph",
        "#  two spaces",
        "#\tTab",
        "# Trailing hashes stay literal #",
        "#1234",
        "#1234 is a pull request, not a heading",
        "# Heading with **bold** and @README.md",
        "#",
        "# ",
        "text\n# Heading\ntext",
        "# Heading\n- item\n> quote\n---",
    ] {
        let edit = pasted::fragment(source, true, true, 0, &resolve_readme);
        assert_eq!(markdown(&edit), source, "{source}");
        let mut pasted_edit = draft("");
        paste(&mut pasted_edit, source, &resolve_readme);
        let expected = if source.ends_with("@README.md") {
            format!("{source} ")
        } else {
            source.to_string()
        };
        assert_eq!(markdown(&pasted_edit), expected, "{source}");
    }
    let seven = pasted::fragment("####### seven", true, true, 0, &resolve_readme);
    assert_eq!(seven.block_kind(0), BlockKind::Paragraph);

    let h6 = pasted::fragment("###### Six", true, true, 0, &resolve_readme);
    assert_eq!(h6.block_kind(0), BlockKind::Heading { level: 6 });

    let rich = pasted::fragment(
        "# Heading with **bold** and @README.md",
        true,
        true,
        0,
        &resolve_readme,
    );
    assert_eq!(rich.block_kind(0), BlockKind::Heading { level: 1 });
    assert!(rich.marks().iter().any(|m| m.mark == Mark::Bold));
    assert_eq!(rich.pills().count(), 1);
}

#[test]
fn ksg_tasks_round_trip() {
    for source in [
        "- [ ] buy milk",
        "-   [ ]  buy milk",
        "\t-\t[x]\t\titem",
        "- [ ]  ",
        "- [ ]\n  - [ ] child",
        "  - [ ] first\n - [ ] second\n  - [ ] child",
        "- [x] done\n- [ ] next",
        "- [X] uppercase",
        "- [ ] parent\n  - [ ] child\n  - [ ] sibling\n- [ ] uncle",
        "- [ ] empty task follows\n- [ ]",
        "- [ ] **bold** task with @README.md",
        "para\n- [ ] task\npara",
        "- [ ]No space stays literal",
        "-[ ] also literal",
    ] {
        let edit = pasted::fragment(source, true, true, 0, &resolve_readme);
        assert_eq!(markdown(&edit), source, "{source}");
        let mut pasted_edit = draft("");
        paste(&mut pasted_edit, source, &resolve_readme);
        let expected = if source.ends_with("@README.md") {
            format!("{source} ")
        } else {
            source.to_string()
        };
        assert_eq!(markdown(&pasted_edit), expected, "{source}");
    }
    let mut upper = draft("");
    paste(&mut upper, "- [X] uppercase", &resolve_readme);
    assert_eq!(
        upper.block_kind(0),
        BlockKind::Task {
            level: 0,
            checked: true
        }
    );
    upper.set_task_checked(0, false);
    assert_eq!(markdown(&upper), "- [ ] uppercase");
    upper.undo();
    assert_eq!(markdown(&upper), "- [X] uppercase");
    upper.redo();
    assert_eq!(markdown(&upper), "- [ ] uppercase");

    let mut literal = draft("");
    paste(&mut literal, "- [ ]No space stays literal", &resolve_readme);
    assert_eq!(literal.block_kind(0), BlockKind::Paragraph);
}

#[test]
fn ksg_rules_round_trip_and_selection_deletion() {
    for source in [
        "---",
        "***",
        "___",
        "- - -",
        "* * *",
        "-----",
        "---   ",
        "text\n---\nmore",
        "- item\n---\n- item two",
        "```\n---\n```",
        "--",
        "-- -",
        "---text",
    ] {
        let mut edit = draft("");
        paste(&mut edit, source, &resolve_readme);
        assert_eq!(markdown(&edit), source, "{source}");
    }
    let mut ordered_kinds = draft("");
    paste(
        &mut ordered_kinds,
        "- - -\n***\n___\ntext\n---\n- item",
        &resolve_readme,
    );
    assert_eq!(
        (0..6)
            .map(|i| ordered_kinds.block_kind(i))
            .collect::<Vec<_>>(),
        vec![
            BlockKind::Rule,
            BlockKind::Rule,
            BlockKind::Rule,
            BlockKind::Paragraph,
            BlockKind::Rule,
            BlockKind::Bullet(0),
        ]
    );

    // Selection, navigation, deletion, undo, redo.
    let mut edit = draft("");
    paste(&mut edit, "above\n* * *\nbelow", &resolve_readme);
    let rule_id = edit.logical_blocks().nth(1).unwrap().id;
    assert!(edit.select_block(rule_id));
    assert_eq!(edit.selection(), EditorSelection::Block(rule_id));
    assert_eq!(edit.selected(), "* * *");
    press(&mut edit, &mut Rules::default(), Key::Backspace, false);
    assert_eq!(markdown(&edit), "above\nbelow");
    edit.undo();
    assert_eq!(markdown(&edit), "above\n* * *\nbelow");
    assert_eq!(edit.selection(), EditorSelection::Block(rule_id));
    edit.redo();
    assert_eq!(markdown(&edit), "above\nbelow");
}

#[test]
fn heading_task_and_rule_input_rules_backspace_and_enter() {
    // Headings 1..6: immediate Backspace reversal, start-of-heading revert, Enter to paragraph.
    for level in 1..=6u8 {
        let prefix = format!("{} ", "#".repeat(level as usize));
        let mut edit = draft("");
        let mut rules = Rules::default();
        type_text(&mut edit, &mut rules, &prefix);
        assert_eq!(edit.block_kind(0), BlockKind::Heading { level });
        assert_eq!(markdown(&edit), prefix);
        press(&mut edit, &mut rules, Key::Backspace, false);
        assert_eq!(edit.block_kind(0), BlockKind::Paragraph);
        assert_eq!(edit.text, prefix);

        let mut edit = draft("");
        let mut rules = Rules::default();
        type_text(&mut edit, &mut rules, &format!("{prefix}Title"));
        assert_eq!(edit.block_kind(0), BlockKind::Heading { level });
        assert_eq!(markdown(&edit), format!("{prefix}Title"));
        edit.set_caret(0);
        press(&mut edit, &mut rules, Key::Backspace, false);
        assert_eq!(edit.block_kind(0), BlockKind::Paragraph);
        assert_eq!(edit.text, "Title");

        let mut edit = draft("");
        let mut rules = Rules::default();
        type_text(&mut edit, &mut rules, &prefix);
        press(&mut edit, &mut rules, Key::Enter, false);
        assert_eq!(edit.block_kind(0), BlockKind::Heading { level });
        assert_eq!(edit.block_kind(1), BlockKind::Paragraph);
    }

    // Tasks: - [ ] , - [x] , - [X] , bullet-to-task [ ] , Backspace reversal, Enter continue/exit.
    for (trigger, checked) in [("- [ ] ", false), ("- [x] ", true), ("- [X] ", true)] {
        let mut edit = draft("");
        let mut rules = Rules::default();
        type_text(&mut edit, &mut rules, trigger);
        assert_eq!(edit.block_kind(0), BlockKind::Task { level: 0, checked });
        assert_eq!(markdown(&edit), trigger);
        press(&mut edit, &mut rules, Key::Backspace, false);
        assert_eq!(edit.block_kind(0), BlockKind::Paragraph);
        assert_eq!(edit.text, trigger);
    }
    let mut edit = draft("");
    let mut rules = Rules::default();
    type_text(&mut edit, &mut rules, "* [ ] item");
    assert_eq!(
        edit.block_kind(0),
        BlockKind::Task {
            level: 0,
            checked: false
        }
    );
    assert_eq!(edit.text, "item");
    press(&mut edit, &mut rules, Key::Enter, false);
    assert_eq!(
        edit.block_kind(1),
        BlockKind::Task {
            level: 0,
            checked: false
        }
    );
    press(&mut edit, &mut rules, Key::Enter, false);
    assert_eq!(edit.block_kind(1), BlockKind::Paragraph);

    // Rules: --- , *** , ___ and Backspace reversal.
    for trigger in ["---", "*** ", "___ "] {
        let mut edit = draft("");
        let mut rules = Rules::default();
        type_text(&mut edit, &mut rules, trigger);
        assert_eq!(edit.block_kind(0), BlockKind::Rule);
        assert_eq!(markdown(&edit), trigger);
        press(&mut edit, &mut rules, Key::Backspace, false);
        assert_eq!(edit.block_kind(0), BlockKind::Paragraph);
        assert_eq!(edit.text, trigger);
    }
}
