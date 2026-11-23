use super::*;

#[test]
fn reading_and_control_text_keep_accessible_contrast_in_both_themes() {
    fn over_srgb(base: Color, overlay: Color) -> Color {
        use rune::color::{srgb_decode, srgb_encode};
        let a = overlay.a;
        let blend = |b: f32, o: f32| {
            let (b, o) = (srgb_encode(b), srgb_encode(o));
            srgb_decode(b * (1.0 - a) + o * a)
        };
        Color {
            r: blend(base.r, overlay.r),
            g: blend(base.g, overlay.g),
            b: blend(base.b, overlay.b),
            a: 1.0,
        }
    }
    fn ratio(a: Color, b: Color) -> f32 {
        let (a, b) = (a.luminance(), b.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    for dark in [false, true] {
        let p = palette(dark);
        for (index, (foreground, background)) in [
            (p.ink, p.panel),
            (p.ink, p.bg),
            (p.muted, p.panel),
            (p.muted, p.bg),
            (p.muted, p.card),
            (p.faint, p.panel),
            (p.faint, p.card),
            (p.faint, p.card_top),
            (p.faint, p.card_bottom),
            (p.faint, p.bg),
            (p.faint, p.lift),
            (p.faint, p.bg.mix(p.selected, p.selected.a)),
            (p.faint, over_srgb(p.bg, p.selected)),
            (p.faint, p.panel.mix(p.selected, p.selected.a)),
            (p.faint, over_srgb(p.panel, p.selected)),
            (p.faint, p.card.mix(p.selected, p.selected.a)),
            (p.faint, over_srgb(p.card, p.selected)),
            (p.faint, p.code_bg),
            (p.ink, p.message_bg),
            // The send pill's icon, compact label and stop mark on the recessed `pill` fill
            // and under a single interaction wash in linear light and sRGB compositing.
            (p.ink, p.pill),
            (p.ink, p.pill.mix(p.selected, p.selected.a)),
            (p.ink, over_srgb(p.pill, p.selected)),
            (p.muted, p.pill),
            (p.muted, p.pill.mix(p.selected, p.selected.a)),
            (p.muted, over_srgb(p.pill, p.selected)),
            (p.danger, p.pill),
            (p.danger, p.pill.mix(p.selected, p.selected.a)),
            (p.danger, over_srgb(p.pill, p.selected)),
            // The "Shina" role label sits on the coral wash.
            (p.ink, p.panel.mix(p.coral_bg, p.coral_bg.a)),
            (p.ink, over_srgb(p.panel, p.coral_bg)),
            (p.muted, p.code_bg),
            (p.muted, p.bg.mix(p.selected, p.selected.a)),
            (p.muted, over_srgb(p.bg, p.selected)),
            (p.muted, p.card.mix(p.selected, p.selected.a)),
            (p.muted, over_srgb(p.card, p.selected)),
            (p.sea, p.panel),
            (p.sea, p.card),
            (p.sea, p.bg),
            (p.sea, p.sea_bg),
            (p.on_coral, p.coral),
            (p.on_brass, p.brass),
            (p.brass, p.panel),
            (p.brass, p.brass_bg),
            (p.brass, p.bg.mix(p.selected, p.selected.a)),
            (p.brass, over_srgb(p.bg, p.selected)),
        ]
        .into_iter()
        .enumerate()
        {
            // `faint` is KSG's `--placeholder`, for text that can recede (carets,
            // timestamps); it has to clear 3:1. `muted` is KSG's `#8a8a8a`, which reads
            // 4.38:1 on `lift`.
            let floor = if foreground == p.faint {
                3.0
            } else if foreground == p.muted {
                4.3
            } else {
                4.5
            };
            let contrast = ratio(foreground, background);
            assert!(
                contrast >= floor,
                "text contrast {contrast:.2} in pair {index}, dark={dark}"
            );
        }
        // WCAG 1.4.11: non-text control boundaries (fields, checkboxes, radios, active toggles)
        // must read at 3:1 or better against every surface they sit on.
        for (name, surface) in [
            ("bg", p.bg),
            ("panel", p.panel),
            ("tray", p.tray),
            ("card", p.card),
            ("card_top", p.card_top),
            ("card_bottom", p.card_bottom),
            ("lift", p.lift),
            ("pill", p.pill),
            ("pill-hover-linear", p.pill.mix(p.selected, p.selected.a)),
            ("pill-hover-srgb", over_srgb(p.pill, p.selected)),
        ] {
            let contrast = ratio(p.field_line, surface);
            assert!(
                contrast >= 3.0,
                "field_line contrast {contrast:.2} on {name}, dark={dark}"
            );
        }
    }
    // Night's two quiet inks are different steps, not one colour under two names.
    let p = palette(true);
    assert!(
        (p.muted.luminance() + 0.05) / (p.faint.luminance() + 0.05) >= 1.25,
        "dark muted and faint text read as the same colour"
    );
}

#[test]
fn older_history_controls_show_load_loading_error_and_end_states_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        {
            let t = rt.state.current_mut();
            t.messages.push(crate::model::Message {
                id: "u1".into(),
                role: "You",
                text: "Recent message".into(),
                images: vec![],
                context: None,
            });
            t.history_has_more = true;
            t.history_loading = false;
            t.history_error = None;
        }
        rt.frame(0.0);
        assert!(
            rt.presented("history-load-earlier").is_some(),
            "{width}: load button"
        );
        assert!(
            !rt.accessibility_json()
                .to_string()
                .contains("Showing recent turns."),
            "{width}: old cutoff notice must be gone"
        );

        // Loading state.
        rt.state.current_mut().history_loading = true;
        rt.invalidate();
        rt.frame(0.0);
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("Loading earlier turns…")),
            "{width}: loading label"
        );

        // Error state.
        {
            let t = rt.state.current_mut();
            t.history_loading = false;
            t.history_error = Some("History failed".into());
        }
        rt.invalidate();
        rt.frame(0.0);
        assert!(
            rt.presented("history-load-error").is_some(),
            "{width}: error label"
        );

        // End-of-history state: button and error disappear.
        {
            let t = rt.state.current_mut();
            t.history_has_more = false;
            t.history_error = None;
        }
        rt.invalidate();
        rt.frame(0.0);
        assert!(
            rt.find("history-load-earlier").is_none(),
            "{width}: hidden at end"
        );
        assert!(
            rt.find("history-load-error").is_none(),
            "{width}: error cleared"
        );
    }
}

#[test]
fn virtualised_timeline_mounts_only_viewport_window_and_overscan_for_long_threads() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        {
            let t = rt.state.current_mut();
            for turn in 0..500usize {
                t.messages.push(crate::model::Message {
                    id: format!("u-{turn}"),
                    role: "You",
                    text: format!("Prompt number {turn} with some wrapping prose."),
                    images: vec![],
                    context: None,
                });
                t.messages.push(crate::model::Message {
                    id: format!("a-{turn}"),
                    role: "Pi",
                    text: format!("Reply number {turn} with detailed explanation."),
                    images: vec![],
                    context: None,
                });
            }
        }
        rt.frame(0.0);
        let mounted_rows: Vec<_> = rt
            .elements()
            .into_iter()
            .filter(|e| {
                e.key
                    .as_deref()
                    .is_some_and(|k| k.starts_with("row-") || k.starts_with("turn-"))
            })
            .collect();
        assert!(
            !mounted_rows.is_empty() && mounted_rows.len() < 80,
            "{width}: expected virtualised window, mounted {} rows",
            mounted_rows.len()
        );
        assert!(
            rt.find("row-1-u-0").is_none(),
            "{width}: oldest row must be unmounted when pinned at bottom"
        );
        assert!(
            rt.presented("turn-1-a-499").is_some(),
            "{width}: newest row must be presented"
        );
    }
}

#[test]
fn scroll_anchor_preserves_visible_row_on_prepend_expand_image_load_and_width_change() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.compact_sidebar = true;
        {
            let t = rt.state.current_mut();
            t.history_has_more = true;
            for turn in 0..80usize {
                t.messages.push(crate::model::Message {
                    id: format!("u-{turn}"),
                    role: "You",
                    text: format!("Question {turn}: keeping anchor steady."),
                    images: if turn == 35 {
                        vec![Attachment {
                            asset_id: None,
                            uploaded: None,
                            source: None,
                            id: "img-35.png".into(),
                            name: "img-35.png".into(),
                            mime: "image/png".into(),
                            width: 400,
                            height: 120,
                            bytes: 1,
                            path: None,
                        }]
                    } else {
                        vec![]
                    },
                    context: None,
                });
                let a_idx = t.messages.len();
                t.messages.push(crate::model::Message {
                    id: format!("a-{turn}"),
                    role: "Pi",
                    text: format!("Answer {turn}: extra text to take vertical space."),
                    images: vec![],
                    context: None,
                });
                if turn == 45 {
                    t.tools.push(crate::model::Tool {
                        id: "tool-45".into(),
                        name: "read".into(),
                        kind: Some(crate::work::ToolKind::Read),
                        changes: vec![],
                        input: serde_json::json!({"path": "src/ui.rs"}),
                        detail: "src/ui.rs".into(),
                        done: true,
                        failed: false,
                        output: "line 1\nline 2\nline 3\nline 4".into(),
                        images: vec![],
                        structured: serde_json::Value::Null,
                        parent_call: None,
                        exit_code: Some(0),
                        subagents: vec![],
                        anchor: crate::model::ToolAnchor {
                            message: a_idx,
                            offset: 0,
                        },
                        block: 0,
                        order: 1,
                        expanded: false,
                        raw_expanded: false,
                        group_expanded: false,
                        started: None,
                        duration_ms: Some(10),
                    });
                    t.work.push(crate::model::AssistantWork {
                        start: a_idx,
                        stopped: false,
                        stopped_order: 0,
                        expanded: false,
                        elapsed: Some(3),
                        outcome: Some(Status::Completed),
                    });
                }
            }
        }
        rt.frame(0.0);
        // Scroll into the middle around turn 45 (scroll_center_in_instant mounts unrealized rows).
        rt.update(|_, tx| tx.scroll_center_in_instant("conversation-1", "row-1-u-45"));
        for _ in 0..60 {
            if !rt.frame(1.0 / 60.0) {
                break;
            }
        }
        let anchor_before = rt.presented("row-1-u-45").expect("u-45 visible").0;

        // (a) Prepending older turns + clearing history_has_more preserves visible row y.
        {
            let t = rt.state.current_mut();
            let mut prepended = Vec::new();
            for older in 0..20usize {
                prepended.push(crate::model::Message {
                    id: format!("old-u-{older}"),
                    role: "You",
                    text: format!("Older question {older}."),
                    images: vec![],
                    context: None,
                });
                prepended.push(crate::model::Message {
                    id: format!("old-a-{older}"),
                    role: "Pi",
                    text: format!("Older answer {older}."),
                    images: vec![],
                    context: None,
                });
            }
            let shift = prepended.len();
            prepended.append(&mut t.messages);
            t.messages = prepended;
            for tool in &mut t.tools {
                tool.anchor.message += shift;
            }
            for work in &mut t.work {
                work.start += shift;
            }
            t.history_has_more = false;
        }
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        let anchor_after_prepend = rt.presented("row-1-u-45").expect("u-45 still visible").0;
        assert!(
            (anchor_after_prepend[1] - anchor_before[1]).abs() < 1.0,
            "{width}: prepend moved anchor from {} to {}",
            anchor_before[1],
            anchor_after_prepend[1]
        );

        // (b) Expanding a work section preserves its toggle row's position.
        let toggle_key = rt
            .elements()
            .into_iter()
            .find_map(|e| {
                e.key
                    .filter(|k| k.starts_with("toggle-work-"))
                    .filter(|k| rt.presented(k).is_some())
            })
            .expect("toggle-work visible");
        let toggle_before = rt.presented(&toggle_key).unwrap().0[1];
        press(&mut rt, &toggle_key);
        rt.frame(1.0);
        let toggle_after = rt.presented(&toggle_key).unwrap().0[1];
        assert!(
            (toggle_after - toggle_before).abs() < 1.0,
            "{width}: work expand moved toggle from {toggle_before} to {toggle_after}"
        );

        // (c) Image height change above the visible anchor preserves the visible anchor.
        let before_img = rt.presented("row-1-u-45").unwrap().0[1];
        {
            let t = rt.state.current_mut();
            let u35 = t.messages.iter_mut().find(|m| m.id == "u-35").unwrap();
            u35.images[0].height = 600;
        }
        rt.invalidate();
        rt.frame(1.0);
        let after_img = rt.presented("row-1-u-45").unwrap().0[1];
        assert!(
            (after_img - before_img).abs() < 1.0,
            "{width}: image load moved anchor from {before_img} to {after_img}"
        );

        // (d) Width change keeps the visible anchor in view at the same offset.
        // Put this row at the viewport anchor: a different width can reflow rows
        // between the topmost row and a row that was only centred in the viewport.
        rt.update(|_, tx| tx.scroll_key_to("row-1-u-45", 0.0));
        rt.frame(1.0);
        let before_resize = rt.presented("row-1-u-45").unwrap().0[1];
        rt.set_viewport((width + 160.0, 720.0), 1.0);
        for _ in 0..120 {
            if !rt.frame(1.0 / 60.0) {
                break;
            }
        }
        let after_resize = rt
            .presented("row-1-u-45")
            .expect("u-45 still presented after resize")
            .0[1];
        assert!(
            (after_resize - before_resize).abs() < 2.0,
            "{width}: resize moved anchor from {before_resize} to {after_resize}"
        );
    }
}

#[test]
fn stick_to_bottom_follows_streaming_and_unpins_when_user_scrolls_up() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        {
            let t = rt.state.current_mut();
            for turn in 0..40usize {
                t.messages.push(crate::model::Message {
                    id: format!("u-{turn}"),
                    role: "You",
                    text: format!("Prompt {turn}"),
                    images: vec![],
                    context: None,
                });
                t.messages.push(crate::model::Message {
                    id: format!("a-{turn}"),
                    role: "Pi",
                    text: format!("Reply {turn}"),
                    images: vec![],
                    context: None,
                });
            }
            t.status = Status::Running;
        }
        rt.frame(0.0);
        assert!(
            rt.presented("turn-1-a-39").is_some(),
            "{width}: starts at bottom"
        );

        // Streaming growth while pinned keeps bottom in view.
        rt.state
            .current_mut()
            .messages
            .last_mut()
            .unwrap()
            .text
            .push_str("\n\nMore streamed lines\n\nEven more");
        rt.invalidate();
        rt.frame(1.0);
        assert!(
            rt.presented("turn-1-a-39").is_some(),
            "{width}: stays pinned while streaming"
        );

        // Scrolling up unpins and shows Scroll to end; further streaming does NOT yank back down.
        let (conv, _) = rt.presented("conversation-1").unwrap();
        for _ in 0..20 {
            rt.wheel(conv[0] + conv[2] * 0.5, conv[1] + conv[3] * 0.5, 400.0);
            rt.frame(1.0 / 60.0);
        }
        for _ in 0..120 {
            if !rt.frame(1.0 / 60.0) {
                break;
            }
        }
        assert!(
            rt.presented("turn-1-a-39")
                .is_none_or(|(r, _)| r[1] >= conv[1] + conv[3]),
            "{width}: scrolled away from bottom: {:?} vs conv {:?}",
            rt.presented("turn-1-a-39"),
            conv
        );
        assert!(
            rt.presented("scroll-to-end").is_some(),
            "{width}: scroll-to-end pill visible"
        );
        let scrolled_row = rt
            .elements()
            .into_iter()
            .find(|e| {
                e.key.as_deref().is_some_and(|k| k.starts_with("row-1-u-"))
                    && rt.presented(e.key.as_deref().unwrap()).is_some()
            })
            .map(|e| (e.key.unwrap(), e.rect[1]))
            .unwrap();

        rt.state
            .current_mut()
            .messages
            .last_mut()
            .unwrap()
            .text
            .push_str("\n\nTail chunk while reading history");
        rt.invalidate();
        rt.frame(1.0);
        assert!(
            (rt.presented(&scrolled_row.0).unwrap().0[1] - scrolled_row.1).abs() < 1.0,
            "{width}: streaming must not pull scrolled-up reader back down"
        );

        // Clicking scroll-to-end jumps back to bottom and hides the pill.
        press(&mut rt, "scroll-to-end");
        for _ in 0..240 {
            if !rt.frame(1.0 / 60.0) {
                break;
            }
        }
        assert!(
            rt.presented("turn-1-a-39").is_some(),
            "{width}: returned to bottom"
        );
        assert!(
            rt.find("scroll-to-end").is_none(),
            "{width}: pill hides at bottom"
        );
    }
}

#[test]
fn find_bar_opens_via_chat_find_displays_counts_steps_matches_highlights_and_jumps_to_match() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.on_key = Some(crate::key);
        rt.poll = Some(|s, _, tx| s.poll_thread_find(tx));
        {
            let t = rt.state.current_mut();
            for turn in 0..40usize {
                let prompt = if turn == 2 {
                    "Early message with target-canary-word here".to_owned()
                } else if turn == 38 {
                    "Late message with target-canary-word too".to_owned()
                } else {
                    format!("Ordinary message {turn}")
                };
                t.messages.push(crate::model::Message {
                    id: format!("u-{turn}"),
                    role: "You",
                    text: prompt,
                    images: vec![],
                    context: None,
                });
                t.messages.push(crate::model::Message {
                    id: format!("a-{turn}"),
                    role: "Pi",
                    text: format!("Reply {turn}"),
                    images: vec![],
                    context: None,
                });
            }
        }
        rt.frame(0.0);

        // Wire chat.find shortcut (Ctrl+F / Cmd+F).
        let mod_key = if cfg!(target_os = "macos") {
            rune::Mods {
                cmd: true,
                ..Default::default()
            }
        } else {
            rune::Mods {
                ctrl: true,
                ..Default::default()
            }
        };
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("f".into()),
            mods: mod_key,
        });
        rt.frame(0.0);
        assert!(
            rt.presented("thread-find-bar").is_some(),
            "{width}: bar opens"
        );
        assert!(
            rt.presented("thread-find-input").is_some(),
            "{width}: input shown"
        );
        assert!(
            rt.presented("thread-find-prev").is_some(),
            "{width}: prev shown"
        );
        assert!(
            rt.presented("thread-find-next").is_some(),
            "{width}: next shown"
        );
        assert!(
            rt.presented("thread-find-close").is_some(),
            "{width}: close shown"
        );

        // Bar stays inside the pane without clipping or overflowing.
        let (bar, _) = rt.presented("thread-find-bar").unwrap();
        let (pane, _) = rt.presented("chat-pane").unwrap();
        assert!(
            bar[0] >= pane[0] && bar[0] + bar[2] <= pane[0] + pane[2] + 0.5,
            "{width}: bar {bar:?} outside pane {pane:?}"
        );

        // Typing a missing query reports 0/0.
        rt.paste("missing-query-xyz");
        rt.frame(0.2);
        rt.frame(1.0 / 60.0);
        assert_eq!(
            element_text(&rt, "thread-find-count").as_deref(),
            Some("0/0"),
            "{width}"
        );

        // Replace with matching query -> shows 1/2, highlights match, and jumps to u-2!
        rt.state.set_thread_find_query("target-canary-word".into());
        rt.invalidate();
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(
            element_text(&rt, "thread-find-count").as_deref(),
            Some("1/2"),
            "{width}"
        );
        assert!(
            rt.presented("row-1-u-2").is_some(),
            "{width}: jumped to match u-2"
        );
        assert!(
            rt.elements().iter().any(|e| e.role == "mark"),
            "{width}: highlight mark rendered"
        );

        // Next -> jumps to 2/2 (u-38); Next again wraps to 1/2 (u-2).
        press(&mut rt, "thread-find-next");
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(
            element_text(&rt, "thread-find-count").as_deref(),
            Some("2/2"),
            "{width}"
        );
        assert!(
            rt.presented("row-1-u-38").is_some(),
            "{width}: jumped to match u-38"
        );

        press(&mut rt, "thread-find-prev");
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(
            element_text(&rt, "thread-find-count").as_deref(),
            Some("1/2"),
            "{width}"
        );

        // Close via close button restores composer focus.
        press(&mut rt, "thread-find-close");
        rt.frame(0.0);
        assert!(rt.find("thread-find-bar").is_none(), "{width}: bar closed");
        assert!(
            rt.state.current().draft.focused,
            "{width}: composer refocused"
        );

        // Explicit unsupported state for a peer without thread-find.
        rt.state.open_thread_find();
        rt.state.thread_find.unsupported = true;
        rt.invalidate();
        rt.frame(0.0);
        assert_eq!(
            element_text(&rt, "thread-find-count").as_deref(),
            Some("Update required"),
            "{width}: unsupported peer state"
        );
        rt.state.close_thread_find();
    }
}

#[test]
fn sidebar_search_shows_title_matches_before_message_matches_with_snippets_and_jumps_to_match() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.compact_sidebar = width < 760.0;
        rt.state.threads.push(Thread::new(2));
        rt.state.threads[0].title = "Alpha workspace task".into();
        rt.state.threads[0].renamed = true;
        rt.state.threads[1].title = "Unrelated thread".into();
        rt.state.threads[1].renamed = true;
        // Thread 2 is unvisited (no messages loaded locally), matched via server search!
        rt.state.search.replace_text("alpha");
        rt.state.sync_sidebar_search_input();
        rt.state.sidebar_search.visible_query = "alpha".into();
        rt.state.sidebar_search.visible_matches.insert(
            ("local".into(), 2),
            crate::thread_search::ThreadSearchHit {
                thread_id: 2,
                project_id: None,
                entry_id: "u-4".into(),
                source: "user".into(),
                snippet: "…matched alpha inside unvisited history…".into(),
                quality: 1.0,
                hits: 1,
                updated: 10,
            },
        );
        rt.state.sidebar_search.shown_order = vec![("local".into(), 2)];
        rt.state.sidebar_search.unsupported_servers = vec!["Studio".into()];
        rt.frame(0.0);

        let (row1, _) = rt.presented("thread-1").expect("title match shown");
        let (row2, _) = rt.presented("thread-2").expect("message match shown");
        assert!(
            row1[1] < row2[1],
            "{width}: title match must appear before message match"
        );
        assert!(
            rt.presented("sidebar-search-snippet-local-2").is_some(),
            "{width}: snippet shown for message-matched row"
        );
        assert!(
            rt.presented("sidebar-search-unsupported").is_some(),
            "{width}: unsupported server notice shown"
        );

        // Clicking the message-matched row opens thread 2 and opens find on the matched query.
        press(&mut rt, "thread-2");
        rt.frame(0.0);
        assert_eq!(rt.state.current().id, 2, "{width}: opened thread 2");
        assert!(rt.state.thread_find.open, "{width}: find bar opened");
        assert_eq!(
            rt.state.thread_find.query.text, "alpha",
            "{width}: query set"
        );
    }
}

#[test]
fn shelves_start_collapsed_keep_the_open_row_and_page_settled_threads() {
    let mut rt = runtime(1280.0);
    rt.state.populate_demo();
    rt.frame(0.0);
    // KSG's Working shelf collects running threads at the bottom of the list.
    assert!(rt.find("shelf-Working").is_some());
    // Thread 2 is running, so it lives on the collapsed Working shelf.
    assert!(rt.find("thread-2").is_none());
    // Shelves sit at the bottom of the sidebar, just above its footer.
    let mut tall = Runtime::with_text(
        State::new(".".into(), false),
        view,
        (1280.0, 1400.0),
        Env::new(false, 1.0),
        rune::text::TextSystem::new(FONTS, (SANS_FAMILY, MONO_FAMILY)),
    );
    tall.state.populate_demo();
    tall.frame(0.0);
    tall.frame(0.5);
    let (settled, _) = tall.presented("shelf-Settled").unwrap();
    let (footer, _) = tall.presented("sidebar-footer").unwrap();
    assert!(
        footer[1] - (settled[1] + settled[3]) < 40.0,
        "Settled {settled:?} is not above the footer {footer:?}"
    );
    assert!(rt.find("shelf-Snoozed").is_some());
    assert!(rt.find("shelf-Settled").is_some());
    assert!(rt.find("thread-4").is_none());
    assert!(rt.find("thread-5").is_none());
    rt.update(|_, tx| tx.scroll_into_view("shelf-Working"));
    rt.frame(0.5);
    press(&mut rt, "shelf-Working");
    assert!(rt.state.shelves[0]);
    assert!(rt.find("thread-2").is_some());
    rt.update(|_, tx| tx.scroll_into_view("shelf-Snoozed"));
    rt.frame(0.5);
    press(&mut rt, "shelf-Snoozed");
    assert!(rt.state.shelves[1]);
    assert!(rt.find("thread-4").is_some());
    rt.state.open_kept_tab(3);
    rt.frame(0.0);
    rt.update(|_, tx| tx.scroll_into_view("shelf-Snoozed"));
    rt.frame(0.5);
    press(&mut rt, "shelf-Snoozed");
    assert!(
        rt.find("thread-4").is_some(),
        "collapsed shelf retains the open thread"
    );
    for id in 11..51 {
        let mut t = Thread::new(id);
        t.life.settle(lifecycle::now() - id as u64);
        rt.state.threads.push(t);
    }
    rt.state.shelves[2] = true;
    rt.invalidate();
    rt.frame(0.0);
    let count = |s: &State| {
        s.visible_sidebar_threads()
            .iter()
            .filter(|i| s.threads[**i].life.settled())
            .count()
    };
    assert_eq!(count(&rt.state), 10);
    assert!(rt.find("settled-more").is_some());
    rt.update(|_, tx| tx.scroll_into_view("settled-more"));
    rt.frame(0.5);
    press(&mut rt, "settled-more");
    assert_eq!(count(&rt.state), 35);
}

const LONG_DRAFT: &str = "Go ahead and make this an issue on GitHub, and then I want you to deeply analyze specific features from KSG Code missing from Shina. I am looking for things like:\n1. Gaps in our agent output composition. I think the way KSG formats tool calls, thinking and agent output is far superior.\n2. In the input box, we lack a lot of functionality like the $ for skills and slash commands from the ACP integrations from providers.\n3. On that note, even ACP providers.\n4. I suspect agents don't have the deep knowledge of Shina's tooling and application like they do in KSG Code.\n5. Generally speaking there appear to be bugs.\nWhen you write the issue, list each gap with the KSG Code component it comes from, what Shina does today, and what a port would take, so we can plan the work in slices and review each one on its own.\n6. And bugs.";

/// A conversation whose composer holds `LONG_DRAFT`, focused, with the caret at its end.
fn long_draft(width: f32, zoom: f32) -> Runtime<State> {
    let mut rt = runtime(width);
    rt.state.populate_demo();
    rt.state.demo = false;
    rt.state.compact_sidebar = false;
    rt.state.zoom = zoom;
    rt.frame(0.0);
    rt.state.current_mut().draft = rune::TextEdit::new(LONG_DRAFT);
    rt.invalidate();
    rt.frame(0.0);
    // Click near the top of the field's viewport: a long field's centre can lie below it.
    rt.frame(0.25);
    let (view, _) = rt.presented("composer-scroll").unwrap();
    rt.pointer_down(view[0] + 24.0, view[1] + 12.0);
    rt.pointer_up(view[0] + 24.0, view[1] + 12.0);
    rt.frame(0.0);
    // The press put the caret where it landed. Type at the end, as the user does, so the
    // field follows the caret.
    let draft = &mut rt.state.current_mut().draft;
    draft.set_caret(draft.text.len());
    rt.invalidate();
    rt.frame(0.0);
    rt.key(rune::KeyEvent {
        key: rune::Key::Text(".".into()),
        mods: Default::default(),
    });
    rt.frame(0.5);
    rt.frame(0.5);
    rt
}

#[test]
fn a_long_draft_scrolls_inside_the_composer_beside_its_actions() {
    for zoom in [1.0, 1.25] {
        for width in [1280.0] {
            let rt = long_draft(width, zoom);
            let at = format!("width {width}, zoom {zoom}");
            assert!(rt.state.current().draft.focused, "{at}");
            assert!(
                rt.state.current().draft.text.ends_with("And bugs.."),
                "{at}"
            );
            let (card, _) = rt.presented("composer").unwrap();
            // The input's visible box is its body: the padding around the field's viewport.
            let (input, _) = rt.presented("composer-body").unwrap();
            let (actions, _) = rt.presented("composer-actions").unwrap();
            // The field scrolled to its end, as typing there keeps the caret in view: its
            // text ends in its 4 pt padding at the viewport's bottom, above the actions.
            // (Rune's reported caret rect ignores zoom, so the field's own rect is used.)
            let (view, _) = rt.presented("composer-scroll").unwrap();
            let (field, _) = rt.presented("composer-input").unwrap();
            let text_bottom = field[1] + field[3] - 4.0 * zoom;
            assert!(
                (text_bottom - (view[1] + view[3] - 4.0 * zoom)).abs() <= 1.0,
                "{at}: the field {field:?} is not scrolled to its end in {view:?}"
            );
            assert!(field[3] > view[3], "{at}: the draft fits; it should not");
            // KSG's prompt body: the actions sit beside the text at the bottom right of a
            // wrapped draft, never over it, and the card is the body's height.
            assert!(
                input[0] + input[2] <= actions[0] + 0.5,
                "{at}: the input {input:?} runs under the actions {actions:?}"
            );
            assert!(
                (input[1] + input[3] - (actions[1] + actions[3])).abs() <= 1.5,
                "{at}: the actions {actions:?} are not on the input's last line {input:?}"
            );
            assert!(
                actions[1] + actions[3] <= card[1] + card[3] + 0.5
                    && card[1] + card[3] - (actions[1] + actions[3]) <= 10.0,
                "{at}: the actions {actions:?} are not at the card's bottom {card:?}"
            );
            // A press on the last visual line lands in the field and places the caret there,
            // not on the actions row.
            // Near the start of the last line, where the caret is not already.
            let (x, y) = (view[0] + 8.0 * zoom, text_bottom - 10.0 * zoom);
            let picked = rt
                .element_at_in("composer", x, y)
                .unwrap_or_else(|| panic!("{at}: nothing at ({x}, {y})"));
            assert!(
                picked.path().contains("composer-input") || picked.key() == Some("composer-input"),
                "{at}: the last line at ({x}, {y}) reaches {picked:?}"
            );
            let mut rt = rt;
            let end = rt.state.current().draft.focus;
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.0);
            let draft = &rt.state.current().draft;
            let last = draft.text.rfind('\n').unwrap();
            assert!(
                draft.focus > last && draft.focus < end,
                "{at}: a press on the last line put the caret at {}, not after {last} and before {end}",
                draft.focus
            );
        }
    }
}

#[test]
fn typing_in_the_composer_never_moves_a_scrolled_transcript_or_grows_past_the_cap() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        rt.state.demo = false;
        rt.state.compact_sidebar = true;
        rt.frame(0.0);
        let id = rt.state.current().id;
        let viewport = format!("conversation-{id}");
        rt.update(|_, tx| tx.scroll_to(viewport.clone(), 0.0, 0.0));
        rt.frame(0.5);
        if width < 760.0 {
            let (pane, _) = rt.presented("chat-pane").unwrap();
            press(&mut rt, "sidebar-toggle");
            rt.frame(1.0);
            assert!(rt.presented("sidebar").is_none());
            assert_eq!(
                rt.presented("chat-pane").unwrap().0,
                pane,
                "closing the compact sidebar must not resize the conversation"
            );
        }
        press(&mut rt, "composer-input");
        let (lane, _) = rt.presented("chat-lane").unwrap();
        let mut heights = Vec::new();
        for _ in 0..14 {
            rt.key(rune::KeyEvent {
                key: rune::Key::Text("A line of the draft that is long enough to wrap".into()),
                mods: Default::default(),
            });
            rt.key(rune::KeyEvent {
                key: rune::Key::Enter,
                mods: rune::Mods {
                    shift: true,
                    ..Default::default()
                },
            });
            rt.frame(0.0);
            // The transcript keeps its place while the user reads back and types.
            assert_eq!(
                rt.presented("chat-lane").unwrap().0[1],
                lane[1],
                "width {width}: the transcript moved while typing"
            );
            heights.push(rt.presented("composer").unwrap().0[3]);
        }
        // The card grows with the draft, then stops at KSG's cap.
        let cap = heights.last().copied().unwrap();
        assert!(heights[0] < cap, "width {width}: {heights:?}");
        assert!(
            // The prompt body's 6 pt padding above and below the field.
            (cap - (12.0 + COMPOSER_FIELD_MAX)).abs() <= 0.5,
            "width {width}: {heights:?}"
        );
    }
}

#[test]
fn archived_captain_hides_composer_and_uses_group_restore_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state = State::headless(
            std::env::temp_dir().join(format!("shina-archived-ui-{}", uuid::Uuid::new_v4())),
        );
        rt.state.demo = true;
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.open_captain(0);
        let home = rt.state.current().id;
        let database = crate::database::Database::open(&rt.state.data_dir).unwrap();
        database.begin().unwrap();
        let preview = rt.state.captain_lifecycle_preview(0, "archive").unwrap();
        rt.state
            .captain_archive_command(
                0,
                "archive",
                preview["captainVersion"].as_str().unwrap(),
                &[],
            )
            .unwrap();
        database
            .commit(1, &rt.state.projection(), &json!([]), None, &[])
            .unwrap();
        rt.state.show_archive = true;
        rt.state.compact_sidebar = width < 760.0;
        rt.frame(0.0);
        rt.frame(1.0);
        for key in ["captain-archived-bar", "captain-archived-unarchive"] {
            let (rect, _) = rt.presented(key).unwrap_or_else(|| panic!("missing {key}"));
            assert!(
                rect[0] >= 0.0
                    && rect[1] >= 0.0
                    && rect[0] + rect[2] <= width + 0.5
                    && rect[1] + rect[3] <= 720.5,
                "{key} outside {width}: {rect:?}"
            );
        }
        assert!(rt.presented("composer-input").is_none());
        assert_eq!(rt.state.current().id, home);
        press(&mut rt, "thread-actions");
        assert!(rt.find("menu-unarchive-captain").is_some());
        assert!(rt.find("menu-archive-thread").is_none());
        assert!(rt.find("menu-delete-thread").is_none());
    }
}

#[test]
fn narrow_workspace_keeps_the_composer_inside_the_window() {
    let mut rt = runtime(620.0);
    rt.state.populate_demo();
    rt.frame(0.0);
    let (rect, _) = rt.presented("composer").unwrap();
    assert!(rect[0] >= 0.0 && rect[0] + rect[2] <= 620.5);
    assert!(rect[2] > 200.0);
    for key in ["thread-actions", "leave-example"] {
        let (rect, _) = rt.presented(key).unwrap();
        assert!(
            rect[0] >= 0.0 && rect[0] + rect[2] <= 620.5,
            "{key} overflows the window"
        );
    }
    let (bubble, _) = rt
        .presented(&format!("{}-text", message_key(rt.state.current(), 0)))
        .unwrap();
    assert!(
        bubble[0] >= 0.0 && bubble[0] + bubble[2] <= 620.0,
        "user message overflows its pane: {bubble:?}"
    );
    // A compact window gives the conversation the full width until the sidebar is asked for.
    assert!(rt.presented("sidebar").is_none());
    press(&mut rt, "sidebar-toggle");
    // Layout changes animate; let them settle before hit-testing the new sidebar.
    rt.frame(2.0);
    assert!(
        rt.state.compact_sidebar,
        "toggle did not request the sidebar"
    );
    assert!(rt.presented("sidebar").is_some());
    press(&mut rt, "sidebar-toggle");
    rt.frame(4.0);
    assert!(!rt.state.compact_sidebar, "sidebar toggle did not close it");
    assert!(rt.presented("sidebar").is_none());
}
#[test]
fn context_ring_reports_real_usage_and_opens_details_by_keyboard() {
    let mut rt = runtime(1280.0);
    let thread = rt.state.current_mut();
    thread.model = "Vision".into();
    thread.context_window = Some(200_000);
    thread.supports_images = Some(true);
    thread.context.usage = ContextUsage {
        tokens: Some(60_000),
        context_window: Some(200_000),
        percent: Some(30.0),
        breakdown: None,
    };
    thread.draft.replace_text("Ready");
    rt.invalidate();
    rt.frame(0.0);

    let ring = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some("context-usage-ring"))
        .expect("context ring");
    assert_eq!(ring.role, "button");
    assert!(ring.focusable);
    assert_eq!(ring.expanded, Some(false));
    let label = ring.label.as_deref().unwrap();
    assert!(
        label.contains("60,000 of 200,000 tokens") && label.contains("30%"),
        "{label}"
    );
    assert!(
        ring.help.is_none(),
        "the popover replaces the native tooltip"
    );

    // KSG's `ComposerControlsBar`: the ring sits in the context strip beside the model.
    let (ring_rect, _) = rt.presented("context-usage-ring").unwrap();
    let (model_rect, _) = rt.presented("model-effort").unwrap();
    let (strip, _) = rt.presented("composer-footer").unwrap();
    assert!(
        model_rect[0] + model_rect[2] <= ring_rect[0] + 0.5,
        "ring must sit beside, not over, the model"
    );
    assert!(
        (ring_rect[1] + ring_rect[3] / 2.0 - (strip[1] + strip[3] / 2.0)).abs() < 2.0,
        "the ring is centred in the context strip"
    );

    let mut tabs = 0;
    while tabs < 120 {
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(0.0);
        tabs += 1;
        if rt
            .elements()
            .into_iter()
            .any(|element| element.key.as_deref() == Some("context-usage-ring") && element.focused)
        {
            break;
        }
    }
    assert!(
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("context-usage-ring"))
            .unwrap()
            .focused,
        "the ring must be reachable by keyboard after {tabs} tabs"
    );
    rt.key(rune::KeyEvent {
        key: rune::Key::Text(" ".into()),
        mods: Default::default(),
    });
    rt.frame(0.0);
    // Primary press opens the Inspector on Context (spec §4.4).
    assert_eq!(
        rt.state.right_panel_active_view(),
        Some(crate::right_panel::PanelView::Context)
    );
    // Secondary press pins the hover popover (spec §4.4).
    let (rx, ry) = rt.find("context-usage-ring").unwrap();
    rt.secondary_down(rx, ry);
    rt.secondary_up(rx, ry);
    rt.frame(0.3);
    assert_eq!(
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("context-usage-ring"))
            .unwrap()
            .expanded,
        Some(true)
    );
    assert!(rt.find("context-details").is_some());
    assert_eq!(
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("context-details-usage"))
            .unwrap()
            .text
            .as_deref(),
        Some("60k/200k")
    );
    assert_eq!(
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("context-details-percent"))
            .unwrap()
            .text
            .as_deref(),
        Some("30%")
    );
    assert_eq!(
        rt.elements()
            .into_iter()
            .find(|element| element.focused)
            .and_then(|element| element.key),
        Some("context-usage-ring".to_owned()),
        "opening details keeps keyboard focus on the ring"
    );
    // Escape closes the pinned popover; Space and primary press toggle the Context tab.
    rt.key(rune::KeyEvent {
        key: rune::Key::Escape,
        mods: Default::default(),
    });
    rt.frame(0.5);
    assert!(
        rt.find("context-details").is_none(),
        "Escape closes the details popover"
    );
    assert!(!rt.state.current().context.details_open);

    rt.key(rune::KeyEvent {
        key: rune::Key::Text(" ".into()),
        mods: Default::default(),
    });
    rt.frame(0.3);
    assert_eq!(rt.state.right_panel_active_view(), None);

    press(&mut rt, "context-usage-ring");
    rt.frame(0.3);
    assert_eq!(
        rt.state.right_panel_active_view(),
        Some(crate::right_panel::PanelView::Context),
        "pointer activation toggles the Inspector on Context"
    );
}

#[test]
fn context_popover_opens_on_hover_and_stays_while_the_pointer_is_inside() {
    for width in [1280.0] {
        let mut rt = context_fixture(width);
        let (composer, _) = rt.presented("composer").unwrap();
        let (ring, _) = rt.presented("context-usage-ring").unwrap();
        let (rx, ry) = (ring[0] + ring[2] / 2.0, ring[1] + ring[3] / 2.0);

        // A brief pass opens nothing; resting past KSG's 150 ms delay does.
        rt.pointer_move(rx, ry);
        rt.frame(0.05);
        assert!(
            rt.find("context-details").is_none(),
            "{width}: opened early"
        );
        rt.frame(0.2);
        rt.frame(0.3);
        let (popover, _) = rt
            .presented("context-details")
            .unwrap_or_else(|| panic!("{width}: hover did not open the popover"));
        assert!(
            popover[0] >= 0.0 && popover[0] + popover[2] <= width + 0.5,
            "{width}: popover leaves the window {popover:?}"
        );
        assert!(
            popover[1] + popover[3] <= ring[1] + 0.5,
            "{width}: popover must sit above the ring {popover:?} {ring:?}"
        );
        assert!(
            (popover[0] + popover[2] - (ring[0] + ring[2])).abs() < 1.0
                || popover[0] + popover[2] >= width - 9.0,
            "{width}: popover aligns to the ring's end"
        );
        let (after, _) = rt.presented("composer").unwrap();
        assert_eq!(
            composer, after,
            "{width}: the composer moved when it opened"
        );
        for (key, text) in [
            ("context-details-percent", "31%"),
            ("context-details-usage", "61k/200k"),
        ] {
            let element = rt
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap();
            assert_eq!(element.text.as_deref(), Some(text), "{width}: {key}");
        }

        // Crossing the gap into the popover keeps it open past the close delay.
        rt.pointer_move(rx, ring[1] - 2.0);
        rt.frame(0.02);
        rt.pointer_move(popover[0] + 20.0, popover[1] + 20.0);
        rt.frame(0.02);
        rt.frame(0.4);
        rt.frame(0.4);
        assert!(
            rt.find("context-details").is_some(),
            "{width}: moving into the popover closed it"
        );

        // Leaving both closes it after the delay.
        rt.pointer_move(4.0, 200.0);
        rt.frame(0.05);
        assert!(
            rt.state.current().context.popover_open(),
            "{width}: closed before the delay"
        );
        rt.frame(0.3);
        rt.frame(0.5);
        assert!(!rt.state.current().context.popover_open(), "{width}");
        assert!(rt.find("context-details").is_none(), "{width}: still shown");
    }
}

#[test]
fn usage_limits_and_service_tiers_fit_at_supported_widths() {
    for width in [1280.0] {
        let mut rt = context_fixture(width);
        let t = rt.state.current_mut();
        t.context.details_open = true;
        t.harness_state["usageLimits"] = json!({"windows":[
            {"id":"primary","name":"5 hour","usedPercent":90,"resetsAt":2100000000},
            {"id":"secondary","name":"7 day","usedPercent":25,"resetsAt":null}]});
        rt.invalidate();
        rt.frame(0.3);
        let (card, _) = rt.presented("context-details").unwrap();
        assert!(
            card[0] >= 0.0 && card[0] + card[2] <= width + 0.5,
            "{width}: {card:?}"
        );
        for index in 0..2 {
            for prefix in ["usage-bar", "usage-reset", "usage-percent"] {
                let (rect, _) = rt.presented(&format!("{prefix}-{index}")).unwrap();
                assert!(
                    rect[0] >= card[0] && rect[0] + rect[2] <= card[0] + card[2] + 0.5,
                    "{width}: {prefix} {rect:?}"
                );
            }
        }
        assert_eq!(rt.presented("usage-bar-0").unwrap().0[3], 6.0);
        let fill = |rt: &Runtime<State>, key: &str| {
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap()
                .fill
        };
        assert_eq!(fill(&rt, "usage-fill-0"), palette(rt.state.dark).danger);
        assert_eq!(fill(&rt, "usage-fill-1"), palette(rt.state.dark).faint);
        rt.state.current_mut().context = Default::default();
        rt.state.current_mut().context_window = None;
        rt.state.current_mut().context.details_open = true;
        rt.invalidate();
        rt.frame(0.35);
        assert!(
            rt.find("context-usage-limits").is_some(),
            "limits remain accessible without context capacity"
        );
        rt.state.current_mut().harness_state["usageLimits"] = json!({"windows":[]});
        rt.invalidate();
        rt.frame(0.4);
        assert!(rt.find("context-usage-limits").is_none());
        let t = rt.state.current_mut();
        t.context.close_popover();
        t.harness = "codex".into();
        t.model_provider = "codex".into();
        t.model_id = "fixture".into();
        t.models = vec![crate::session::AgentModel::from_harness_value(&json!({
            "provider":"codex","id":"fixture","name":"Fixture","thinkingLevels":["low","high"],
            "serviceTiers":[{"id":"default","name":"Standard"},{"id":"fast","name":"Fast"}]
        }),"codex").unwrap()];
        t.thinking_levels = vec!["low".into(), "high".into()];
        t.thinking = "high".into();
        rt.state.settings_picker = Some(SettingsPicker::Thinking);
        rt.invalidate();
        rt.frame(0.5);
        let (picker, _) = rt.presented("reasoning-picker").unwrap();
        let (control, _) = rt.presented("service-tier-picker").unwrap();
        assert!(control[0] >= picker[0] && control[0] + control[2] <= picker[0] + picker[2] + 0.5);
        assert!(picker[0] >= 0.0 && picker[0] + picker[2] <= width + 0.5);
        press(&mut rt, "service-tier-1");
        assert_eq!(rt.state.current().harness_state["serviceTier"], "fast");
    }
}

#[test]
fn chat_lane_follows_ksg_gutters_and_width_at_supported_widths() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        rt.state.demo = false;
        rt.frame(0.0);
        rt.frame(0.0);
        let (main, _) = rt.presented("chat-main").unwrap();
        let (lane, _) = rt.presented("chat-lane").unwrap();
        let (card, _) = rt.presented("composer").unwrap();
        let gutter = lane_gutter(main[2]);
        assert!(
            lane[0] - main[0] >= gutter - 0.5
                && main[0] + main[2] - (lane[0] + lane[2]) >= gutter - 0.5,
            "{width}: lane {lane:?} crowds its pane {main:?}"
        );
        assert!(lane[2] <= measure(main[2] >= WIDE_PANE) + 0.5, "{width}");
        assert!(
            (card[0] - lane[0]).abs() < 0.5 && (card[2] - lane[2]).abs() < 0.5,
            "{width}: the composer {card:?} and transcript {lane:?} share one lane"
        );
        let bubble = rt
            .elements()
            .into_iter()
            .filter(|element| {
                element
                    .key
                    .as_deref()
                    .is_some_and(|key| key.starts_with("message-") && key.ends_with("-text"))
            })
            .filter_map(|element| rt.presented(element.key.as_deref()?))
            .map(|(rect, _)| rect)
            .next();
        if let Some(bubble) = bubble {
            assert!(
                bubble[2] <= lane[2] * 0.8 + 0.5,
                "{width}: bubble {bubble:?}"
            );
            assert!(bubble[0] + bubble[2] <= lane[0] + lane[2] + 0.5, "{width}");
        }
    }
}

#[test]
fn composer_shell_recessed_pill_and_quiet_access_chip_at_supported_widths() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        rt.state.demo = false;
        rt.frame(0.0);
        rt.frame(0.0);
        let p = palette(rt.state.dark);
        // KSG's `ComposerSurface.Shell`: the 16 pt tray wraps the card in the resting
        // and the expanded composer alike.
        let (shell, _) = rt
            .presented("composer-shell")
            .unwrap_or_else(|| panic!("{width}: composer-shell"));
        let (card, _) = rt
            .presented("composer")
            .unwrap_or_else(|| panic!("{width}: composer card"));
        assert!(
            card[0] >= shell[0] - 0.5
                && card[0] + card[2] <= shell[0] + shell[2] + 0.5
                && card[1] >= shell[1] - 0.5
                && card[1] + card[3] <= shell[1] + shell[3] + 0.5,
            "{width}: card {card:?} outside shell {shell:?}"
        );
        let fills: std::collections::HashMap<String, Color> = rt
            .elements()
            .into_iter()
            .filter_map(|e| e.key.map(|k| (k, e.fill)))
            .collect();
        assert_eq!(
            fills.get("composer-shell"),
            Some(&p.tray),
            "{width}: shell fill"
        );
        // KSG's `ComposerSendPill`: the recessed 32 pt action slot in `pill` fill.
        assert_eq!(
            fills.get("composer-transition-actions"),
            Some(&p.pill),
            "{width}: send pill fill"
        );
        let (pill, _) = rt.presented("composer-transition-actions").unwrap();
        assert!(
            (pill[3] - 32.0).abs() < 0.5,
            "{width}: pill height {pill:?}"
        );
        let (send, _) = rt.presented("send-message").unwrap();
        assert!(
            send[0] >= pill[0] - 0.5
                && send[0] + send[2] <= pill[0] + pill[2] + 0.5
                && send[1] >= pill[1] - 0.5
                && send[1] + send[3] <= pill[1] + pill[3] + 0.5,
            "{width}: send {send:?} outside pill {pill:?}"
        );
        // Conversation and composer share the resolved lane.
        let (lane, _) = rt.presented("chat-lane").unwrap();
        assert!(
            (card[0] - lane[0]).abs() < 0.5 && (card[2] - lane[2]).abs() < 0.5,
            "{width}: composer {card:?} and transcript {lane:?} share one lane"
        );
        // KSG's `composerControlsOwnership`: the default mode keeps the chip quiet.
        assert!(
            rt.presented("access-mode").is_none(),
            "{width}: the default access mode must not mount the chip"
        );
        assert!(
            !rt.state.composer_ui.show_access.get(),
            "{width}: default mode keeps the mode block out of the strip and the overflow"
        );
        rt.state.current_mut().native.access = crate::model::AccessMode::Supervised;
        rt.state.current_mut().draft.focused = true;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.0);
        assert!(
            rt.state.composer_ui.show_access.get(),
            "{width}: a non-default access mode shows the chip"
        );
        // The chip sits in the strip, or is measured into the overflow menu, which then
        // carries the permissions section (KSG's `overflowShowsPermissions`).
        assert!(
            rt.presented("access-mode").is_some() || rt.presented("composer-overflow").is_some(),
            "{width}: the non-default chip is in the strip or its overflow"
        );
        rt.state.current_mut().native.access = crate::model::AccessMode::Full;
    }
}

#[test]
fn tool_groups_and_individual_outputs_expand_independently() {
    let mut rt = runtime(1280.0);
    rt.state.populate_demo();
    rt.frame(0.0);
    assert!(rt.find("toggle-tool-group-1-0").is_some());
    assert!(rt.find("inline-tool-1-0").is_none());
    // Larger reply text can put the first tool group above the pinned-bottom viewport.
    rt.wheel(800.0, 200.0, 1000.0);
    rt.frame(1.0);
    press(&mut rt, "toggle-tool-group-1-0");
    rt.frame(2.0);
    assert!(rt.state.current().tools[0].group_expanded);
    assert!(rt.find("inline-tool-1-0").is_some());
    press(&mut rt, "inline-tool-1-0");
    rt.frame(4.0);
    assert!(rt.state.current().tools[0].expanded);
    assert!(rt.presented("tool-output-1-0").is_some());
    assert!(rt.presented("tool-output-1-1").is_none());
    assert!(
        !rt.state.right_panel().open,
        "tool output never opens a side panel"
    );
}

fn long_turn_runtime(width: f32, parts: usize, live: bool) -> Runtime<State> {
    let mut rt = runtime(width);
    long_turn_fixture(&mut rt.state, parts, live);
    open_work_streaks(rt.state.current_mut());
    settle(&mut rt);
    rt
}

/// Unfolds every work streak of the long turn, so each step is its own work entry.
/// Folded, a streak of hundreds of steps is one header row.
fn open_work_streaks(t: &mut Thread) {
    let firsts: Vec<usize> = turn_parts(t, 1, 2)
        .iter()
        .filter_map(|part| match part {
            TranscriptPart::Work(entries) => entry_tools(entries).next(),
            _ => None,
        })
        .collect();
    for index in firsts {
        t.tools[index].group_expanded = true;
    }
    t.timeline_cache.get_mut().touch_message(1);
}

fn mounted_work(rt: &Runtime<State>) -> usize {
    rt.elements()
        .iter()
        .filter(|element| {
            element.key.as_deref().is_some_and(|key| {
                key.starts_with("inline-tool-1-")
                    || key.starts_with("inline-thought-1-")
                    || key.starts_with("tool-output-1-")
            })
        })
        .count()
}

/// The key of the work slice `1 / divisor` of the way through the long turn.
fn slice_at(rt: &Runtime<State>, divisor: usize) -> String {
    let rows = timeline_structure(rt.state.current()).rows.clone();
    let slices: Vec<_> = rows
        .iter()
        .filter(|row| {
            matches!(
                row,
                TimelineRow::TurnPiece {
                    piece: TurnPiece::Work { .. },
                    ..
                }
            )
        })
        .collect();
    slices[slices.len() / divisor].key().to_owned()
}

fn center_on(rt: &mut Runtime<State>, key: &str) {
    let key = key.to_owned();
    rt.update(move |_, tx| tx.scroll_center_in_instant("conversation-1", key));
    settle(rt);
}

#[test]
fn long_turns_mount_work_in_proportion_to_the_viewport() {
    for live in [true, false] {
        for width in [1280.0] {
            let mut rt = long_turn_runtime(width, 1_000, live);
            let rows = timeline_structure(rt.state.current()).rows.clone();
            let slices = rows
                .iter()
                .filter(|row| {
                    matches!(
                        row,
                        TimelineRow::TurnPiece {
                            piece: TurnPiece::Work { .. },
                            ..
                        }
                    )
                })
                .count();
            assert!(
                slices >= 1_000 / SLICE_ITEMS,
                "{live} {width}: {slices} slices"
            );
            assert!(rows.iter().any(|row| row.key() == "turn-1-long-answer"));
            // A quarter in: inside the first streak of both fixtures, away from the prose
            // that splits each turn in half.
            let middle = slice_at(&rt, 4);
            center_on(&mut rt, &middle);
            assert!(rt.presented(&middle).is_some(), "{live} {width}: {middle}");
            let short = mounted_work(&rt);
            // Twice the work: the same viewport mounts the same number of rows.
            let mut longer = long_turn_runtime(width, 2_000, live);
            let middle = slice_at(&longer, 4);
            center_on(&mut longer, &middle);
            let long = mounted_work(&longer);
            assert!(short > 0 && short < 400, "{live} {width}: {short} mounted");
            // Rune mounts two viewports either side; the longer list's estimated rows can
            // carry up to two more slices of slack at that edge.
            assert!(
                long.abs_diff(short) <= 4 * SLICE_ITEMS,
                "{live} {width}: {short} mounted for 1,000 parts, {long} for 2,000"
            );
            // Every slice of a completed turn rebuilt only the mounted ones.
            take_timeline_counts();
            rt.update(|s, _| s.dark = !s.dark);
            rt.frame(1.0 / 60.0);
            let built = take_timeline_counts().built_slices as usize;
            assert!(
                built > 0 && built < slices / 2,
                "{live} {width}: built {built}"
            );
        }
    }
}

#[test]
fn a_running_turn_shows_its_steps_then_folds_into_worked_for() {
    let mut rt = runtime(620.0);
    rt.state.current_mut().apply(json!({"type":"agent_start"}));
    rt.state
        .current_mut()
        .apply(json!({"type":"message_update","assistantMessageEvent":{
            "type":"thinking_start","contentIndex":0
        }}));
    rt.frame(0.0);
    // No thinking text yet: the Thinking row speaks for the turn.
    assert!(rt.presented("live-work-1").is_none());
    assert!(rt.presented("live-status-1").is_some());
    rt.state
        .current_mut()
        .apply(json!({"type":"message_update","assistantMessageEvent":{
            "type":"thinking_delta","contentIndex":0,"delta":"Checking the file. Then the tests."
        }}));
    rt.invalidate();
    rt.frame(1.0);
    // KSG's `work-live`: the thought's first sentence above "Thinking", in one row.
    let (thought, _) = rt.presented("live-thought-1").unwrap();
    let (status, _) = rt.presented("live-work-text-1").unwrap();
    assert!(thought[1] < status[1]);
    let texts: Vec<_> = rt
        .elements()
        .iter()
        .filter_map(|e| e.text.clone())
        .collect();
    assert!(texts.iter().any(|text| text == "Checking the file."));
    assert!(texts.iter().any(|text| text == "Thinking"));
    assert!(rt.presented("live-status-1").is_none());
    assert!(rt.presented("inline-thought-1-0").is_none());
    rt.state
        .current_mut()
        .apply(json!({"type":"agent_settled"}));
    rt.invalidate();
    rt.frame(1.0);
    rt.frame(2.0);
    assert!(rt.presented("live-status-1").is_none());
    assert!(rt.presented("live-work-1").is_none());
    assert!(rt.presented("toggle-work-1-0").is_some());
    assert!(!rt.state.current().work[0].expanded);
    assert!(rt.presented("inline-thought-1-0").is_none());
}

/// A running turn shaped as KSG Code's timeline tests shape theirs: a streak of thoughts and
/// calls, interim prose, then a live streak whose last call is running.
fn streak_runtime(width: f32) -> Runtime<State> {
    let mut rt = runtime(width);
    let t = rt.state.current_mut();
    t.apply(
        json!({"type":"response","id":"history","command":"get_messages","success":true,
        "data":{"messages":[{"role":"assistant","content":[
            {"type":"thinking","thinking":"Plan the audit."},
            {"type":"toolCall","id":"a","name":"read","arguments":{"path":"a.rs"}},
            {"type":"thinking","thinking":"Now the second file."},
            {"type":"toolCall","id":"b","name":"read","arguments":{"path":"b.rs"}},
            {"type":"text","text":"Both files parse."},
            {"type":"thinking","thinking":"**Checking the tests**\n\nThe suite lives in tests/."},
            {"type":"toolCall","id":"c","name":"read","arguments":{"path":"c.rs"}},
            {"type":"toolCall","id":"d","name":"read","arguments":{"path":"d.rs"}}
        ]}]}}),
    );
    for tool in &mut t.tools {
        tool.done = true;
        tool.failed = false;
    }
    for (tool, detail) in t.tools.iter_mut().zip(["a.rs", "b.rs", "c.rs", "d.rs"]) {
        tool.detail = detail.into();
    }
    t.tools[3].done = false;
    t.status = Status::Running;
    t.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(12));
    t.work.clear();
    rt.invalidate();
    rt.frame(0.0);
    rt.frame(1.0);
    rt
}

fn texts(rt: &Runtime<State>) -> Vec<String> {
    rt.elements()
        .iter()
        .filter_map(|e| e.text.clone())
        .collect()
}

#[test]
fn thought_and_tool_streaks_fold_before_prose_and_into_one_live_row() {
    for width in [1280.0] {
        let mut rt = streak_runtime(width);
        rt.state.current_mut().harness = "pi".into();
        rt.state.current_mut().started =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(12));
        rt.invalidate();
        rt.frame(1.0);
        let t = rt.state.current();
        let parts = turn_parts(t, 0, 1);
        use WorkEntry::{Thought, Tool};
        assert_eq!(
            parts[0],
            TranscriptPart::Work(vec![Thought(0), Tool(0), Thought(1), Tool(1)])
        );
        assert!(matches!(parts[1], TranscriptPart::Text { .. }));
        assert_eq!(
            parts[2],
            TranscriptPart::Work(vec![Thought(2), Tool(2), Tool(3)])
        );
        let lane = rt.presented("chat-lane").unwrap().0;
        // A work row's hover wash starts 6 pt before the lane, so its icon slot sits on the
        // lane edge as KSG's does.
        let inside = |rect: [f32; 4]| {
            rect[0] >= lane[0] - 6.5 && rect[0] + rect[2] <= lane[0] + lane[2] + 0.5
        };
        // "Working for 12s" heads the turn.
        assert!(texts(&rt).iter().any(|text| text == "Working for "));
        assert!(texts(&rt).iter().any(|text| text == "12s"));
        // The streak before the prose is one folded header, even while the turn runs.
        let group = rt.presented("toggle-tool-group-1-0").unwrap().0;
        assert!(inside(group), "{width}: {group:?}");
        assert!(texts(&rt).iter().any(|text| text == "Read 2 files"));
        for key in ["inline-tool-1-0", "inline-tool-1-1", "inline-thought-1-0"] {
            assert!(rt.presented(key).is_none(), "{width}: {key}");
        }
        // The live streak is one row: its thought's heading over the running call.
        let live = rt.presented("live-work-1").unwrap().0;
        assert!(inside(live), "{width}: {live:?}");
        assert!(group[1] < live[1]);
        let thought = rt.presented("live-thought-1").unwrap().0;
        let step = rt.presented("live-work-text-1").unwrap().0;
        assert!(thought[1] < step[1]);
        assert!(texts(&rt).iter().any(|text| text == "Checking the tests"));
        assert!(texts(&rt).iter().any(|text| text == "Read d.rs"));
        for key in ["inline-tool-1-2", "inline-tool-1-3", "live-status-1"] {
            assert!(rt.presented(key).is_none(), "{width}: {key}");
        }

        // The live row unfolds its steps under itself, and folds them again.
        press(&mut rt, "live-work-1");
        rt.frame(1.0);
        for key in ["inline-thought-1-2", "inline-tool-1-2", "inline-tool-1-3"] {
            let rect = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"))
                .0;
            assert!(rect[1] > live[1] && inside(rect), "{width}: {key} {rect:?}");
        }
        assert!(rt.presented("live-thought-1").is_none());
        press(&mut rt, "live-work-1");
        rt.frame(1.0);
        assert!(rt.presented("inline-tool-1-3").is_none());

        // The newest call finishes: the row keeps standing for the turn, in the present
        // tense, as KSG's does until the next step starts.
        rt.state.current_mut().tools[3].done = true;
        rt.invalidate();
        rt.frame(1.0);
        assert!(texts(&rt).iter().any(|text| text == "Read d.rs"));
        assert!(rt.presented("live-status-1").is_none());

        // It failed instead: the Thinking row stands in, at the bottom, and unfolds the tail.
        rt.state.current_mut().tools[3].failed = true;
        rt.invalidate();
        rt.frame(1.0);
        assert!(rt.presented("live-work-1").is_none());
        let thinking = rt.presented("live-status-1").unwrap().0;
        assert!(thinking[1] > group[1] && inside(thinking));
        press(&mut rt, "live-status-1");
        rt.frame(1.0);
        assert!(rt.presented("inline-tool-1-3").unwrap().0[1] > thinking[1]);
        press(&mut rt, "live-status-1");
        rt.frame(1.0);

        // Interim prose after the streak: Thinking waits under it, at the very bottom.
        let t = rt.state.current_mut();
        t.tools[3].failed = false;
        t.messages[0].text.push_str(" Next, the fixtures.");
        rt.invalidate();
        rt.frame(1.0);
        let offset = rt.state.current().tools[3].anchor.offset;
        let answer = rt
            .presented(&assistant_body_key(rt.state.current(), 0, offset))
            .unwrap_or_else(|| panic!("{width}: the interim prose"))
            .0;
        let thinking = rt.presented("live-status-1").unwrap().0;
        assert!(answer[1] < thinking[1], "{width}");
        // The tail is now a finished streak before prose: folded like the first.
        assert!(rt.presented("live-work-1").is_none());
        assert!(rt.presented("toggle-tool-group-1-2").is_some());

        // The turn ends: the live rows leave and both streaks stay with the prose.
        rt.state
            .current_mut()
            .apply(json!({"type":"agent_settled"}));
        rt.invalidate();
        rt.frame(1.0);
        rt.frame(2.0);
        assert!(rt.presented("working-bar-1").is_none());
        assert!(rt.presented("live-status-1").is_none());
        assert!(rt.presented("toggle-work-1-0").is_none());
        assert!(rt.presented("toggle-tool-group-1-2").is_some());
        press(&mut rt, "toggle-tool-group-1-0");
        rt.frame(4.0);
        for key in [
            "inline-thought-1-0",
            "inline-tool-1-0",
            "inline-thought-1-1",
            "inline-tool-1-1",
        ] {
            let rect = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"))
                .0;
            assert!(inside(rect), "{width}: {key}");
        }
        // The steps sit 16 pt in from the header, as KSG's `WorkLogList` (`ps-4`).
        assert_eq!(
            rt.presented("inline-tool-1-0").unwrap().0[0],
            rt.presented("toggle-tool-group-1-0").unwrap().0[0] + WORK_LIST_INDENT,
            "{width}"
        );
    }
}

#[test]
fn working_bar_toggles_earlier_live_work_while_keeping_live_tail_visible() {
    let mut rt = streak_runtime(1280.0);
    let bar = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("working-bar-1"))
        .expect("working bar");
    assert_eq!(bar.role, "button");
    assert_eq!(bar.label.as_deref(), Some("Collapse work history"));
    assert!(rt.presented("toggle-tool-group-1-0").is_some());
    assert!(rt.presented("live-work-1").is_some());

    press(&mut rt, "working-bar-1");
    rt.frame(1.0);
    assert!(rt.state.current().live_work_collapsed);
    assert!(rt.presented("toggle-tool-group-1-0").is_none());
    assert!(rt.presented("live-work-1").is_some());
    let bar = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("working-bar-1"))
        .expect("collapsed working bar");
    assert_eq!(bar.label.as_deref(), Some("Expand work history"));

    // Active compaction arriving while the live fold is closed stays visible.
    {
        let t = rt.state.current_mut();
        let message = t.messages.len().saturating_sub(1);
        let offset = t.messages.get(message).map_or(0, |m| m.text.len());
        t.native.compactions.push(json!({
            "running": true,
            "anchor": { "message": message, "offset": offset }
        }));
        t.transcript_structure += 1;
        t.timeline_cache.get_mut().touch_disclosure();
    }
    rt.invalidate();
    rt.frame(1.5);
    assert!(rt.state.current().live_work_collapsed);
    assert!(rt.presented("toggle-tool-group-1-0").is_none());
    assert!(rt.presented("toggle-tool-group-1-2").is_some());
    assert!(rt.presented("compaction-0").is_some());

    // Tool reveal opens the live fold before resolving the hidden tool row.
    let hidden_tool_id = rt.state.current().tools[0].id.clone();
    rt.state.reveal_tool(hidden_tool_id);
    rt.invalidate();
    rt.frame(2.0);
    assert!(!rt.state.current().live_work_collapsed);
    assert!(rt.presented("toggle-tool-group-1-0").is_some());

    // Collapse again and verify thread find reveals folded interim prose in the live turn.
    press(&mut rt, "working-bar-1");
    rt.frame(2.5);
    assert!(rt.state.current().live_work_collapsed);
    let msg_id = rt.state.current().messages[0].id.clone();
    rt.state.open_thread_find();
    rt.state.thread_find.query.replace_text("checking");
    rt.state.thread_find.pending_reveal = Some(crate::thread_find::ThreadFindMatch {
        entry_id: msg_id.clone(),
        occurrence: 0,
    });
    rt.update(|s, tx| {
        s.consume_pending_find_reveal(tx);
    });
    rt.frame(3.0);
    assert!(!rt.state.current().live_work_collapsed);
    assert!(rt.presented("toggle-tool-group-1-0").is_some());
    assert!(rt.presented("compaction-0").is_some());

    // Collapse again and verify citation navigation reveals folded interim prose too.
    press(&mut rt, "working-bar-1");
    rt.frame(3.5);
    assert!(rt.state.current().live_work_collapsed);
    let citation = crate::composer_context::AssistantCitation {
        version: 1,
        environment_id: rt.state.machine_identity("local"),
        thread_id: rt.state.current().id.to_string(),
        message_id: msg_id,
        text: "checking".into(),
        comment: None,
        start: 0,
        end: 8,
        prefix: String::new(),
        suffix: String::new(),
    };
    rt.update(move |s, tx| {
        assert!(s.navigate_to_citation(&citation, tx));
    });
    rt.frame(4.0);
    assert!(!rt.state.current().live_work_collapsed);
    assert!(rt.presented("toggle-tool-group-1-0").is_some());
}
