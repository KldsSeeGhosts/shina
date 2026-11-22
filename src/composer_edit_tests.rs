//! Cases copied from KSG's composer-rich-text.test.ts, composer-rich-text-doc.test.ts
//! and composer-code-block.test.ts. Offsets here are UTF-8 bytes, not PM positions.
use super::*;
use rune::BlockKind;

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
fn ksg_lists_round_trip_and_depth_six() {
    for source in [
        "- one\n- two",
        "* star\n* star two",
        "+ plus",
        "* star\n+ plus",
        "1. first\n2. second",
        "1) paren\n2) paren",
        "3. starts at three\n4. four",
        "01. zero padded\n02. two",
        "07)  item",
        "- parent\n  - child\n  - sibling\n- uncle",
        "1. ordered\n   - bullet child\n2. next",
        "- outer\n  1. inner ordered\n  2. more\n- outer again",
        "-   wide space item",
        "-",
        "1.",
        "- \n- second",
        "- **bold** item with @README.md",
        "- [ ] task\n- bullet after",
        "- bullet\n- [x] task after",
        "para\n- item\npara",
        "\t- tab indented\n\t- again",
        "  - leading indent only",
        "- item\n\n- after a blank",
        "- item one\n```\ncode\n```\n- item two",
        "-no space stays literal",
        "1.no space stays literal",
        "10. ten\n11. eleven",
        "- d0\n  - d1\n    - d2\n      - d3\n        - d4\n          - d5\n            - d6",
    ] {
        let edit = pasted::fragment(source, true, true, 0, &resolve_readme);
        assert_eq!(markdown(&edit), source, "{source}");
    }

    let deep = pasted::fragment(
        "- d0\n  - d1\n    - d2\n      - d3\n        - d4\n          - d5\n            - d6",
        true,
        true,
        0,
        &resolve_readme,
    );
    assert_eq!(
        (0..7).map(|i| deep.block_kind(i)).collect::<Vec<_>>(),
        (0..=6).map(BlockKind::Bullet).collect::<Vec<_>>()
    );
}

#[test]
fn ksg_code_fences_metadata_unclosed_and_adjacent_identity() {
    for source in [
        "```\ncode\n```",
        "```ts\nconst a = 1;\n```",
        "before\n```ts\nconst a = 1;\n```\nafter",
        "```\n```",
        "```ts\nline one\nline two\nline three\n```",
        "```ts\n  indented\n    deeper\n```",
        "```js title=example\ncode\n```",
        "~~~py\ncode\n~~~",
        "~~~~\n```\n~~~~",
        "```ts\ncode without a closing fence",
        "```",
        "```ts\n**not bold** and @README.md stay literal\n```",
        "```ts\ncode\n````",
        "- [ ] task\n```ts\ncode\n```\n- [ ] after",
        "  ```ts\nindented fence stays a paragraph\n  ```",
        "- item one\n```ts\ncode\n```\n- item two",
        "~~~py title=example\n**bold** @README.md $known\n~~~~ ",
    ] {
        let edit = pasted::fragment(source, true, true, 0, &resolve_readme);
        assert_eq!(markdown(&edit), source, "{source}");
        if !source.starts_with("  ```") {
            assert!(edit.marks().is_empty(), "{source}");
            assert_eq!(edit.pills().count(), 0, "{source}");
        }
    }

    // Two adjacent fences with identical text stay distinct through edit, undo and redo.
    let mut edit = pasted::fragment(
        "```\nsame\n```\n```\nsame\n```",
        true,
        true,
        0,
        &resolve_readme,
    );
    let blocks_before: Vec<_> = edit.logical_blocks().collect();
    assert_eq!(blocks_before.len(), 2);
    assert_ne!(blocks_before[0].id, blocks_before[1].id);
    assert_eq!(markdown(&edit), "```\nsame\n```\n```\nsame\n```");

    edit.set_caret(4);
    let mut rules = Rules::default();
    type_text(&mut edit, &mut rules, "!");
    press(&mut edit, &mut rules, Key::Enter, false);
    type_text(&mut edit, &mut rules, "**bold** # heading - [ ] task");
    assert!(edit.marks().is_empty());
    assert_eq!(
        markdown(&edit),
        "```\nsame!\n**bold** # heading - [ ] task\n```\n```\nsame\n```"
    );
    let blocks_edited: Vec<_> = edit.logical_blocks().collect();
    assert_eq!(blocks_edited.len(), 2);
    assert_eq!(blocks_edited[0].id, blocks_before[0].id);
    assert_eq!(blocks_edited[1].id, blocks_before[1].id);

    edit.undo();
    edit.undo();
    edit.undo();
    assert_eq!(markdown(&edit), "```\nsame\n```\n```\nsame\n```");
    let blocks_undone: Vec<_> = edit.logical_blocks().collect();
    assert_eq!(blocks_undone.len(), 2);
    assert_eq!(blocks_undone[0].id, blocks_before[0].id);
    assert_eq!(blocks_undone[1].id, blocks_before[1].id);

    edit.redo();
    edit.redo();
    edit.redo();
    assert_eq!(
        markdown(&edit),
        "```\nsame!\n**bold** # heading - [ ] task\n```\n```\nsame\n```"
    );
    assert_eq!(edit.logical_blocks().count(), 2);

    // Input rule for tilde/info fences and closing with ~~~.
    let mut edit = draft("");
    let mut rules = Rules::default();
    type_text(&mut edit, &mut rules, "~~~py title=demo");
    press(&mut edit, &mut rules, Key::Enter, false);
    assert_eq!(edit.block_kind(0), BlockKind::Code);
    press(&mut edit, &mut rules, Key::Backspace, false);
    assert_eq!(edit.block_kind(0), BlockKind::Paragraph);
    assert_eq!(edit.text, "~~~py title=demo");
    press(&mut edit, &mut rules, Key::Enter, false);
    type_text(&mut edit, &mut rules, "x = 1");
    press(&mut edit, &mut rules, Key::Enter, false);
    type_text(&mut edit, &mut rules, "~~~");
    press(&mut edit, &mut rules, Key::Enter, false);
    assert_eq!(edit.block_kind(1), BlockKind::Paragraph);
    assert_eq!(markdown(&edit), "~~~py title=demo\nx = 1\n~~~\n");
}

#[test]
fn old_and_new_draft_persistence_restart_and_queued_edit_round_trip() {
    // Old plain draft (no html) and old legacy HTML (no data-rune-line) still load.
    let old_plain = restore("plain text", None);
    assert_eq!(markdown(&old_plain), "plain text");
    assert_eq!(old_plain.block_kind(0), BlockKind::Paragraph);

    let legacy_html = "<p><b>bold</b></p><ul><li>item</li></ul><pre><code>code</code></pre>";
    let mut old_rich = restore("bold\nitem\ncode", Some(legacy_html));
    assert_eq!(old_rich.block_kind(0), BlockKind::Paragraph);
    assert_eq!(old_rich.block_kind(1), BlockKind::Bullet(0));
    assert_eq!(old_rich.block_kind(2), BlockKind::Code);
    assert_eq!(markdown(&old_rich), "**bold**\n- item\n```\ncode\n```");
    old_rich.set_caret(0);
    assert!(old_rich.is_rich());

    // Rich draft with every new block kind round-trips through Snapshot, app restart (temp dir),
    // and queued-message edit.
    let rich_source = concat!(
        "# Heading One\n",
        "######  Six\n",
        "- [X] Checked upper\n",
        "- [ ] Unchecked\n",
        "* * *\n",
        "+ plus\n",
        "03)  paren\n",
        "- d0\n  - d1\n    - d2\n      - d3\n        - d4\n          - d5\n            - d6\n",
        "~~~py title=demo\n**literal**\n~~~~ \n",
        "```ts\nunclosed"
    );

    let temp_dir = std::env::temp_dir().join(format!("shina-rich-test-{}", uuid::Uuid::new_v4()));
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
        t.draft = draft("");
        paste(&mut t.draft, rich_source, &resolve_readme);
        assert_eq!(markdown(&t.draft), rich_source);

        // Also exercise queued-message edit round trip.
        t.status = crate::model::Status::Running;
        t.model_provider = "cpa".into();
        t.native.follow_up = true;
        t.send_during_run();
        assert_eq!(t.native.queue[0].prompt.text, rich_source);
        let qid = t.native.queue[0].id;
        t.draft = draft("");
        t.edit_queued(qid);
        assert_eq!(markdown(&t.draft), rich_source);
        t.draft.set_task_checked(3, true);
        t.save_queued_edit();
        assert_eq!(
            t.native.queue[0].prompt.text,
            rich_source.replace("- [ ] Unchecked", "- [x] Unchecked")
        );
        let restored_queued = t.native.queue[0].prompt.draft.as_ref().unwrap().editor();
        assert_eq!(
            restored_queued.block_kind(3),
            BlockKind::Task {
                level: 0,
                checked: true
            }
        );

        // Save ordinary draft with all block kinds across simulated app restart.
        t.draft = draft("");
        paste(&mut t.draft, rich_source, &resolve_readme);
        state.threads = vec![t];
        state.tabs = vec![1];
        state.active = 0;
        state.checkpoint_and_flush().unwrap();
        drop(state);

        let mut restarted = crate::model::State::new(std::path::PathBuf::new(), false);
        restarted.load_global_for_test();
        let reloaded = &restarted.threads.iter().find(|t| t.id == 1).unwrap().draft;
        assert_eq!(markdown(reloaded), rich_source);
        assert_eq!(reloaded.block_kind(0), BlockKind::Heading { level: 1 });
        assert_eq!(reloaded.block_kind(1), BlockKind::Heading { level: 6 });
        assert_eq!(
            reloaded.block_kind(2),
            BlockKind::Task {
                level: 0,
                checked: true
            }
        );
        assert_eq!(reloaded.block_kind(4), BlockKind::Rule);
        assert_eq!(reloaded.block_kind(13), BlockKind::Bullet(6));
        assert_eq!(reloaded.block_kind(14), BlockKind::Code);
        assert_eq!(reloaded.block_kind(15), BlockKind::Code);
    });
}
