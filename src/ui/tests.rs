use super::*;
use crate::context::ContextUsage;
use rune::{Env, Runtime};
pub(crate) fn runtime(width: f32) -> Runtime<State> {
    let mut rt = Runtime::with_text(
        State::new(".".into(), false),
        view,
        (width, 720.0),
        Env::new(false, 1.0),
        rune::text::TextSystem::new(FONTS, (SANS_FAMILY, MONO_FAMILY)),
    );
    // As the app sets it in `main`.
    rt.set_tab_visits_controls(true);
    rt
}

fn connected_machines(rt: &mut Runtime<State>) {
    let projection = rt.state.projection();
    let (mut client, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
    let (remote, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
    client.linked.insert("studio".into(), remote);
    client.linked.insert(
        "offline".into(),
        crate::client::Client::new(rt.state.data_dir.clone()),
    );
    rt.state.client = Some(client);
    for server in ["local", "studio", "offline"] {
        rt.state
            .machines
            .views
            .insert(server.into(), projection.clone().into());
    }
}

#[test]
fn resting_composer_collapses_only_below_sm_and_expands_on_press() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.state.current_mut().draft.replace_text("A single line");
        rt.frame(0.0);
        rt.frame(1.0);
        assert_eq!(
            rt.presented("composer-preview").is_some(),
            width < 640.0,
            "{width}"
        );
        if width < 640.0 {
            press(&mut rt, "composer-preview");
            rt.frame(1.0);
            assert!(rt.state.current().draft.focused);
            assert!(rt.presented("composer-preview").is_none());
        }
        assert!(rt.presented("composer-input").is_some());
    }
}

#[test]
fn opening_a_thread_restores_the_composer_caret() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.current_mut().draft.replace_text("Existing draft");
        rt.state.new_thread();
        rt.state.current_mut().draft.focused = false;
        rt.state.open_kept_tab(0);
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(rt.state.current().draft.focused, "{width}");
    }
}

#[test]
fn timeline_typing_and_paste_redirect_but_controls_and_open_layers_keep_input() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.on_key = Some(crate::key);
        rt.frame(0.0);
        rt.frame(1.0);
        tap(&mut rt, rune::Key::Text("x".into()));
        assert_eq!(rt.state.current().draft.text, "x", "{width}");
        assert!(rt.state.current().draft.focused);
        rt.state.current_mut().draft.focused = false;
        rt.invalidate();
        rt.frame(1.0);
        rt.paste(" pasted");
        rt.frame(1.0);
        assert_eq!(rt.state.current().draft.text, "x pasted", "{width}");

        for key in [
            "sidebar-search",
            "model-search",
            "captain-search",
            "secret",
            "reply-input",
            "settings-search",
            "access-mode",
            "model-effort",
            "send-message",
        ] {
            assert!(
                !composer_ui::redirect_allowed(&rt.state, Some(key)),
                "{key}"
            );
        }
        let layers: &[fn(&mut State)] = &[
            |s| s.settings_menu = true,
            |s| s.settings_picker = Some(SettingsPicker::Thinking),
            |s| s.captain_picker = true,
            |s| s.project_picker = Some(ProjectPicker::NewThread),
            |s| s.rename_thread = Some(s.current().id),
            |s| s.delete_thread = Some(s.current().id),
            |s| s.thread_menu = true,
            |s| s.machines.picker = true,
            |s| s.machines.dialog = true,
            |s| s.projects.clone_open = true,
            |s| s.fleet.open = true,
            |s| s.brief.profile_open = true,
            |s| s.brief.confirm_reset = true,
            |s| s.composer_ui.overflow.set(true),
            |s| s.current_mut().native.access_picker = true,
            |s| s.current_mut().request = Some(json!({"id":"secret", "type":"input"})),
        ];
        for (index, open) in layers.iter().enumerate() {
            let mut state = State::new(".".into(), false);
            open(&mut state);
            assert!(
                !composer_ui::redirect_allowed(&state, None),
                "{width}, layer {index}"
            );
        }
    }
}

#[test]
fn multiline_composer_never_collapses_and_restoration_keeps_model_picker_focus() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.state
            .current_mut()
            .draft
            .replace_text("First line\nSecond line");
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(rt.presented("composer-preview").is_none());
        assert!(rt.presented("composer-input").is_some());
        for reason in [
            composer_ui::FocusReason::ThreadOpened,
            composer_ui::FocusReason::AttachmentInserted,
            composer_ui::FocusReason::MenuClosed,
            composer_ui::FocusReason::Inserted,
        ] {
            rt.state.settings_picker = Some(SettingsPicker::Thinking);
            rt.state.current_mut().draft.focused = false;
            rt.state.restore_composer_focus(reason);
            assert!(!rt.state.current().draft.focused, "{width}, {reason:?}");
            rt.state.settings_picker = None;
            rt.state.restore_composer_focus(reason);
            rt.invalidate();
            rt.frame(1.0);
            assert!(rt.state.current().draft.focused, "{width}, {reason:?}");
        }
    }
}

#[test]
fn tab_inside_the_expanded_composer_keeps_its_controls_mounted() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        // A non-default access mode keeps the permissions chip mounted in the strip.
        rt.state.current_mut().native.access = crate::model::AccessMode::Supervised;
        rt.frame(0.0);
        press(&mut rt, "composer-input");
        tap(&mut rt, rune::Key::Tab);
        assert_eq!(focused_key(&rt).as_deref(), Some("attach-image"), "{width}");
        assert!(rt.presented("composer-preview").is_none(), "{width}");
        for _ in 0..20 {
            if focused_key(&rt).as_deref() == Some("access-mode") {
                break;
            }
            tap(&mut rt, rune::Key::Tab);
        }
        assert_eq!(focused_key(&rt).as_deref(), Some("access-mode"), "{width}");
        tap(&mut rt, rune::Key::Tab);
        assert!(focused_key(&rt).is_some(), "{width}: leaving the footer");
    }
}

fn finish_folder_listing(rt: &mut Runtime<State>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        rt.state.poll_folder_browser();
        if rt.state.projects.browser.pending.is_none()
            && rt.state.projects.browser.request.as_ref()
                == Some(&(
                    rt.state.machines.selected.clone(),
                    rt.state.picker_query.text.clone(),
                ))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "folder listing timed out"
        );
        std::thread::yield_now();
    }
    rt.invalidate();
    rt.frame(0.0);
}

/// Tab once and return the focused element.
fn tab(rt: &mut Runtime<State>) -> rune::ElementInfo {
    rt.key(rune::KeyEvent {
        key: rune::Key::Tab,
        mods: Default::default(),
    });
    rt.frame(0.0);
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .expect("Tab focuses something")
}
/// Tab until `key` has focus.
fn tab_to(rt: &mut Runtime<State>, key: &str) {
    for _ in 0..200 {
        if tab(rt).key.as_deref() == Some(key) {
            return;
        }
    }
    panic!("Tab never reaches {key}");
}
pub(crate) fn tap(rt: &mut Runtime<State>, key: rune::Key) {
    rt.key(rune::KeyEvent {
        key,
        mods: Default::default(),
    });
    rt.frame(0.0);
    rt.frame(1.0);
}
fn focused(rt: &Runtime<State>) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .expect("something has focus")
}
/// Walks an open menu or list from the keyboard: Down onto its first row and on to its last,
/// then Home, End and Up, and Escape closes it.
fn walk_menu(rt: &mut Runtime<State>, menu: &str, wraps: bool, at: &str) {
    let (frame, _) = rt
        .presented(menu)
        .unwrap_or_else(|| panic!("{at}: {menu} is not open"));
    // Down leaves a search field for the list; Home then starts from the top whether or
    // not the menu was opened from the keyboard and already shows its first row.
    tap(rt, rune::Key::Down);
    tap(rt, rune::Key::Home);
    let first = focused(rt);
    let centre = [
        first.rect[0] + first.rect[2] / 2.0,
        first.rect[1] + first.rect[3] / 2.0,
    ];
    assert!(
        centre[0] > frame[0]
            && centre[0] < frame[0] + frame[2]
            && centre[1] > frame[1]
            && centre[1] < frame[1] + frame[3],
        "{at}: Down focuses {:?} outside {menu}",
        first.key
    );
    let mut rows = vec![first];
    for _ in 0..60 {
        tap(rt, rune::Key::Down);
        let next = focused(rt);
        if next.path == rows[0].path || next.path == rows.last().unwrap().path {
            assert_eq!(
                next.path == rows[0].path,
                wraps,
                "{at}: Down past the last row of {menu}"
            );
            break;
        }
        rows.push(next);
    }
    assert!(rows.len() >= 2, "{at}: {menu} has one row");
    for pair in rows.windows(2) {
        assert!(
            pair[1].rect[1] > pair[0].rect[1] - 0.5 || pair[1].scrolled != pair[0].scrolled,
            "{at}: Down goes up from {:?} to {:?}",
            pair[0].key,
            pair[1].key
        );
    }
    let (top, bottom) = (rows[0].path.clone(), rows.last().unwrap().path.clone());
    tap(rt, rune::Key::Home);
    assert_eq!(focused(rt).path, top, "{at}: Home in {menu}");
    tap(rt, rune::Key::End);
    assert_eq!(focused(rt).path, bottom, "{at}: End in {menu}");
    tap(rt, rune::Key::Home);
    tap(rt, rune::Key::Up);
    assert_eq!(
        focused(rt).path,
        if wraps { bottom } else { top },
        "{at}: Up from the top of {menu}"
    );
    tap(rt, rune::Key::Escape);
    assert!(
        rt.presented(menu).is_none(),
        "{at}: Escape leaves {menu} open"
    );
}
#[test]
fn arrow_keys_home_end_return_and_escape_drive_every_menu_and_list() {
    for width in [1280.0] {
        let fresh = || {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.demo = false;
            rt.frame(0.0);
            rt.frame(1.0);
            rt
        };
        let at = |name: &str| format!("{width} {name}");
        // Opened from the keyboard.
        let mut rt = fresh();
        tab_to(&mut rt, "thread-actions");
        tap(&mut rt, rune::Key::Text(" ".into()));
        walk_menu(&mut rt, "thread-menu", true, &at("thread menu"));
        assert!(!rt.state.thread_menu);
        // Return presses the row the keys are on.
        tab_to(&mut rt, "thread-actions");
        tap(&mut rt, rune::Key::Enter);
        tap(&mut rt, rune::Key::Down);
        let row = focused(&rt).key;
        tap(&mut rt, rune::Key::Enter);
        assert!(
            !rt.state.thread_menu,
            "{width}: Return on {row:?} did nothing"
        );

        let mut rt = fresh();
        if rt.presented("sidebar-settings").is_some() {
            tab_to(&mut rt, "sidebar-settings");
            tap(&mut rt, rune::Key::Enter);
            assert_eq!(focused(&rt).key.as_deref(), Some("settings-search"));
            tab_to(&mut rt, "settings-section-0");
            tap(&mut rt, rune::Key::Down);
            assert_eq!(rt.state.settings.section, 1);
            tap(&mut rt, rune::Key::Escape);
            assert!(!rt.state.settings_menu);
        }

        let mut rt = fresh();
        rt.state.thread_menu = true;
        rt.invalidate();
        rt.frame(0.0);
        press(&mut rt, "menu-snooze-thread");
        rt.frame(1.0);
        walk_menu(&mut rt, "snooze-menu", true, &at("snooze menu"));

        let mut rt = fresh();
        let id = rt.state.current().id;
        rt.state.row_menu = Some(crate::model::RowTarget::Thread(id));
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        walk_menu(&mut rt, "row-context-menu", true, &at("row menu"));

        let mut rt = fresh();
        rt.state.open_captain_selector();
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert_eq!(focused(&rt).key.as_deref(), Some("captain-search"));
        tap(&mut rt, rune::Key::Down);
        walk_menu(&mut rt, "captain-list", true, &at("captain picker"));

        let mut rt = fresh();
        rt.state.current_mut().harness = "pi".into();
        rt.state.current_mut().apply(json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
            {"provider":"cpa","id":"model-a","name":"Model A","reasoning":true},
            {"provider":"cpa","id":"model-b","name":"Model B","reasoning":true},
            {"provider":"cpa","id":"model-c","name":"Model C","reasoning":true}
        ]}}));
        rt.state.settings_picker = Some(crate::model::SettingsPicker::Model);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert_eq!(focused(&rt).key.as_deref(), Some("model-search"));
        walk_menu(&mut rt, "model-list", true, &at("model list"));

        // A palette with a search field keeps its highlight in state; Home and End reach
        // its ends as well.
        let mut rt = fresh();
        rt.state
            .open_picker(crate::model::ProjectPicker::AddProject);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        tap(&mut rt, rune::Key::Down);
        assert_eq!(rt.state.picker_highlight, 1, "{width}: Down in the palette");
        tap(&mut rt, rune::Key::End);
        assert!(rt.state.picker_highlight > 0, "{width}: End in the palette");
        tap(&mut rt, rune::Key::Home);
        assert_eq!(rt.state.picker_highlight, 0, "{width}: Home in the palette");
    }
}
#[test]
fn tab_visits_every_enabled_control_in_visual_order_and_enter_or_space_presses_it() {
    for width in [1280.0] {
        for empty in [false, true] {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.demo = false;
            rt.state.composer_ui.force_expanded = true;
            // KSG's 256 px sidebar leaves room for the docked details card here; this
            // walk covers the main column's order, so keep the card off.
            rt.state.details.docked = false;
            if empty {
                rt.state.new_thread_in(None);
            }
            // Thread-open restoration starts at the editor. This test walks visual order
            // from the window's first control, independently of that restoration policy.
            rt.state.current_mut().draft.focused = false;
            rt.frame(0.0);
            rt.frame(1.0);
            let stops: Vec<String> = rt
                .elements()
                .into_iter()
                .filter(|e| !e.inert && e.focusable && e.visible[2] > 0.0 && e.visible[3] > 0.0)
                .filter_map(|e| e.key)
                .collect();
            let mut seen: Vec<rune::ElementInfo> = Vec::new();
            for _ in 0..200 {
                let focused = tab(&mut rt);
                if seen.first().is_some_and(|first| first.path == focused.path) {
                    break;
                }
                assert!(
                    focused.focus_visible,
                    "{width}: {:?} shows no ring",
                    focused.key
                );
                seen.push(focused);
            }
            let keys: Vec<&str> = seen.iter().filter_map(|e| e.key.as_deref()).collect();
            for stop in &stops {
                assert!(keys.contains(&stop.as_str()), "{width}: Tab skips {stop}");
            }
            // Disabled controls and the layers that only take pointer presses stay out.
            for key in ["send-message", "composer-body", "machine-selector"] {
                assert!(!keys.contains(&key), "{width}/{empty}: Tab stops on {key}");
            }
            let prompt = if rt.presented("composer-preview").is_some() {
                "composer-preview"
            } else {
                "composer-input"
            };
            for key in [prompt, "attach-image", "model-effort"] {
                assert!(keys.contains(&key), "{width}/{empty}: Tab misses {key}");
            }
            // Visual order: the title strip, then the sidebar, then the pane, each top to
            // bottom and left to right along a line.
            let region = |e: &rune::ElementInfo| {
                if e.rect[1] < 40.0 {
                    0
                } else if e.role == "separator"
                    || rt
                        .presented("sidebar")
                        .is_some_and(|(r, _)| e.rect[0] < r[0] + r[2])
                {
                    1
                } else {
                    2
                }
            };
            for pair in seen.windows(2) {
                let (a, b) = (&pair[0], &pair[1]);
                let (ra, rb) = (region(a), region(b));
                assert!(ra <= rb, "{width}: {:?} comes before {:?}", a.key, b.key);
                // The tab strip scrolls: a focused tab pulls earlier tabs
                // out from under their recorded rects.
                let elements = rt.elements();
                let in_tabs = |e: &rune::ElementInfo| {
                    let mut at = Some(e);
                    while let Some(e) = at {
                        if e.key.as_deref() == Some("thread-tabs") {
                            return true;
                        }
                        at = e.parent.map(|i| &elements[i]);
                    }
                    false
                };
                if in_tabs(a) || in_tabs(b) {
                    continue;
                }
                if ra == rb && b.role != "separator" && a.role != "separator" {
                    let same_line = (a.rect[1] - b.rect[1]).abs() < 12.0;
                    assert!(
                        if same_line {
                            b.rect[0] >= a.rect[0]
                        } else {
                            b.rect[1] >= a.rect[1]
                        },
                        "{width}: {:?} at {:?} comes after {:?} at {:?}",
                        b.key,
                        b.rect,
                        a.key,
                        a.rect
                    );
                }
            }
        }
        // Space and Return press the focused control.
        let mut rt = runtime(width);
        rt.state.populate_demo();
        rt.state.demo = false;
        rt.frame(0.0);
        rt.frame(1.0);
        tab_to(&mut rt, "thread-actions");
        tap(&mut rt, rune::Key::Text(" ".into()));
        assert!(rt.state.thread_menu, "{width}: Space");
        tap(&mut rt, rune::Key::Escape);
        assert!(!rt.state.thread_menu, "{width}: Escape");
        tab_to(&mut rt, "thread-actions");
        tap(&mut rt, rune::Key::Enter);
        assert!(rt.state.thread_menu, "{width}: Return");
    }
}

#[test]
fn unreachable_machines_show_only_above_the_composer_for_the_open_thread() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        crate::fleet_demo(&mut rt.state);
        // Real machines, not demo data: their fixture clients are not connected.
        rt.state.demo = false;
        rt.state.compact_sidebar = width < 760.0;
        let poll = |rt: &mut Runtime<State>| {
            rt.update(|state, tx| {
                crate::model::poll(state, 0.0, tx);
            });
            rt.invalidate();
            rt.frame(1.0);
        };
        poll(&mut rt);
        // No stuck sidebar notice for machines the user is not using.
        assert!(rt.state.machines.notice.is_none(), "{width}");
        assert!(rt.presented("machine-unreachable").is_none(), "{width}");
        let sidebar = find(&rt.accessibility_json(), "sidebar")
            .expect("sidebar")
            .to_string();
        assert!(!sidebar.contains("unreachable"), "{width}: {sidebar}");
        // The composer banner names only the machine the open thread targets.
        assert!(
            rt.presented("composer-notice-environment-unavailable:example-studio")
                .is_none(),
            "{width}"
        );
        assert!(
            rt.presented("composer-notice-environment-unavailable:example-laptop")
                .is_none(),
            "{width}"
        );
        rt.state.open_machine_thread("example-studio", 3);
        poll(&mut rt);
        assert_eq!(rt.state.machines.selected, "example-studio");
        let tree = rt.accessibility_json();
        let notice = find(
            &tree,
            "composer-notice-environment-unavailable:example-studio",
        )
        .expect("the composer notice says so");
        assert_eq!(notice["name"], "Studio is offline: This device is offline");
        assert!(rt.presented("machine-unreachable").is_none(), "{width}");
        let sidebar = find(&rt.accessibility_json(), "sidebar")
            .expect("sidebar")
            .to_string();
        assert!(!sidebar.contains("unreachable"), "{width}: {sidebar}");
    }
}

#[test]
fn user_markdown_keeps_line_breaks_and_long_paths_inside_the_bubble() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        let source = format!(
            "step one\nstep two\nstep three\n\n/{}",
            "segment".repeat(40)
        );
        rt.state.current_mut().messages = vec![crate::model::Message {
            id: uuid::Uuid::new_v4().to_string(),
            role: "You",
            text: source.clone(),
            images: vec![],
            context: None,
        }];
        rt.frame(0.0);
        assert_eq!(rt.state.current().messages[0].text, source);
        let bubble = rt
            .elements()
            .into_iter()
            .find(|element| {
                element.key.as_deref()
                    == Some(&format!("{}-text", message_key(rt.state.current(), 0)))
            })
            .unwrap();
        let text: Vec<_> = rt
            .elements()
            .into_iter()
            .filter(|element| element.path.starts_with(&bubble.path))
            .filter_map(|element| element.text.map(|text| (text, element.rect)))
            .collect();
        assert!(
            text.iter()
                .any(|(text, _)| text == "step one\nstep two\nstep three"),
            "{text:?}"
        );
        for (_, rect) in text {
            assert!(
                rect[0] >= bubble.rect[0] - 0.5
                    && rect[0] + rect[2] <= bubble.rect[0] + bubble.rect[2] + 0.5,
                "{width}: text {rect:?} escapes bubble {:?}",
                bubble.rect
            );
        }
    }
}

#[test]
fn markdown_code_headers_fit_and_copy_only_the_code_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        let thread = rt.state.current_mut();
        thread.tools.clear();
        thread.thoughts.clear();
        thread.work.clear();
        thread.captain = None;
        thread.messages = vec![
            crate::model::Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "You",
                text: "Explain **this change**.".into(),
                images: vec![],
                context: None,
            },
            crate::model::Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "Pi",
                text: "```rust\nlet ready = true;\n```".into(),
                images: vec![],
                context: None,
            },
        ];
        let clipboard = rune::clipboard::memory();
        rt.use_clipboard(&clipboard);
        rt.frame(0.0);
        let thread = rt.state.current();
        let key = format!("copy-code-{}-{}-0-0", thread.id, thread.messages[1].id);
        let (rect, _) = rt.presented(&key).unwrap();
        assert!(
            rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
            "{width}: {rect:?}"
        );
        press(&mut rt, &key);
        assert_eq!(
            clipboard.text().as_deref().map(|text| text.as_str()),
            Some("let ready = true;")
        );
    }
}

#[test]
fn code_blocks_wrap_by_default_and_toggle_wrap_per_block() {
    let mut rt = runtime(960.0);
    rt.state.populate_demo();
    let thread = rt.state.current_mut();
    thread.tools.clear();
    thread.thoughts.clear();
    thread.work.clear();
    thread.captain = None;
    let long_line = "let very_long_identifier_for_wrapping_check = ".to_owned()
        + &"\"segment_value\" + ".repeat(14)
        + "\"end\";";
    thread.messages = vec![crate::model::Message {
        id: uuid::Uuid::new_v4().to_string(),
        role: "Pi",
        text: format!("```rust\n{long_line}\n```"),
        images: vec![],
        context: None,
    }];
    rt.frame(0.0);
    let thread = rt.state.current();
    let id = format!("{}-{}-0-0", thread.id, thread.messages[0].id);
    let wrap_key = format!("wrap-code-{id}");
    let code_key = format!("code-{id}");
    let button_label = |rt: &Runtime<State>| {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(wrap_key.as_str()))
            .and_then(|e| e.label)
    };
    assert_eq!(button_label(&rt).as_deref(), Some("Disable line wrap"));
    let wrapped_h = rt.presented(&code_key).unwrap().0[3];

    press(&mut rt, &wrap_key);
    rt.frame(1.0);
    assert!(rt.state.unwrapped_code_blocks.contains(&id));
    assert_eq!(button_label(&rt).as_deref(), Some("Wrap lines"));
    let unwrapped_h = rt.presented(&code_key).unwrap().0[3];
    assert!(
        wrapped_h > unwrapped_h,
        "wrapped height {wrapped_h} should exceed unwrapped height {unwrapped_h}"
    );

    // Appending a diff fence mid-stream keeps the preceding code block's wrap identity stable.
    rt.state.current_mut().messages[0]
        .text
        .push_str("\n\n```diff\n- old()\n+ new()\n```");
    rt.state.current_mut().transcript_content += 1;
    rt.invalidate();
    rt.frame(1.5);
    assert_eq!(button_label(&rt).as_deref(), Some("Wrap lines"));
    assert!((rt.presented(&code_key).unwrap().0[3] - unwrapped_h).abs() < 0.5);

    press(&mut rt, &wrap_key);
    rt.frame(2.0);
    assert!(!rt.state.unwrapped_code_blocks.contains(&id));
    assert_eq!(button_label(&rt).as_deref(), Some("Disable line wrap"));
    assert!((rt.presented(&code_key).unwrap().0[3] - wrapped_h).abs() < 0.5);
}

#[test]
fn machine_selector_labels_and_link_dialog_fit_supported_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.compact_sidebar = width < 760.0;
        let mut client = crate::client::Client::new(rt.state.data_dir.clone());
        let mut remote = crate::client::Client::new(rt.state.data_dir.clone());
        remote.remote = Some(crate::peer::Link {
            server_id: "fixture-server".into(),
            host: "studio-mac".into(),
            device_id: "fixture-device".into(),
            token: "fixture".into(),
            enabled: true,
        });
        client.linked.insert("fixture-server".into(), remote);
        rt.state.client = Some(client);
        rt.state.machines.views.insert(
            "fixture-server".into(),
            json!({"index":{"threads":[
                {"id":1,"title":"Remote thread","session":"fixture","captainHome":false,"lifecycle":{}},
                {"id":2,"title":"Remote captain","captainHome":true,"lifecycle":{}}
            ]}}).into());
        rt.frame(0.0);
        for key in ["machine-selector", "machine-selector-label"] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
            assert!(
                rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                "{width}: {rect:?}"
            );
        }
        for key in [
            "machine-thread-fixture-server-1",
            "machine-thread-fixture-server-2",
            "machine-portrait-fixture-server-2",
        ] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
            assert!(
                rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                "{width}: {rect:?}"
            );
        }
        assert!(rt.find("sidebar-machine-label").is_none());
        assert!(rt.find("machine-heading-fixture-server").is_none());
        press(&mut rt, "machine-selector");
        for key in [
            "machine-picker",
            "machine-option-local",
            "machine-option-fixture-server",
            "link-machine",
        ] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
        }
        press(&mut rt, "link-machine");
        for key in [
            "machine-dialog",
            "machine-host",
            "machine-code",
            "machine-link-submit",
        ] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5,
                "{width}: {rect:?}"
            );
            assert!(
                rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5,
                "{width}: {rect:?}"
            );
        }
        press(&mut rt, "close-machines");
        rt.state.machines.selected = "fixture-server".into();
        rt.state.project_picker = Some(ProjectPicker::AddProject);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.25);
        assert!(rt.presented("remote-project-path").is_some());
        assert!(
            rt.find("send-message")
                .is_none_or(|_| !rt.state.machine_connected("fixture-server"))
        );
    }
}

pub(crate) fn press(rt: &mut Runtime<State>, key: &str) {
    rt.frame(0.25);
    if key == "composer-input" && rt.presented("composer-preview").is_some() {
        let (x, y) = rt.find("composer-preview").expect("collapsed composer");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
    }
    let (x, y) = rt
        .find(key)
        .unwrap_or_else(|| panic!("{key} is not on screen"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
}

fn find<'a>(tree: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    if tree["key"] == key {
        return Some(tree);
    }
    tree["children"]
        .as_array()?
        .iter()
        .find_map(|c| find(c, key))
}

fn element_text(rt: &Runtime<State>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .and_then(|e| e.text.or(e.label))
}

fn assert_inside(rt: &mut Runtime<State>, width: f32, keys: &[&str]) {
    for key in keys {
        let (r, _) = rt
            .presented(key)
            .unwrap_or_else(|| panic!("{width}: {key} is missing"));
        assert!(
            r[0] >= 0.0 && r[0] + r[2] <= width + 1.0 && r[2] > 0.0,
            "{width}: {key} {r:?}"
        );
    }
}

fn ui_png(width: u32) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        width,
        1,
        image::Rgba([20, 40, 60, 255]),
    ))
    .write_to(&mut bytes, image::ImageFormat::Png)
    .unwrap();
    bytes.into_inner()
}

fn rect(rt: &Runtime<State>, key: &str) -> [f32; 4] {
    rt.presented(key)
        .unwrap_or_else(|| panic!("{key} is not presented"))
        .0
}

#[path = "tests_composer_flow.rs"]
mod tests_composer_flow;
#[path = "tests_panels.rs"]
mod tests_panels;
#[path = "tests_pickers.rs"]
mod tests_pickers;
#[path = "tests_sidebar_crew.rs"]
mod tests_sidebar_crew;
#[path = "tests_timeline.rs"]
mod tests_timeline;

fn context_fixture(width: f32) -> Runtime<State> {
    let mut rt = runtime(width);
    let thread = rt.state.current_mut();
    thread.context_window = Some(200_000);
    thread.context.usage = ContextUsage {
        tokens: Some(61_400),
        context_window: Some(200_000),
        percent: Some(30.7),
        breakdown: None,
    };
    thread.native.stats = json!({"tokens": {"total": 1_284_000}, "cost": 0.4183});
    thread.draft.replace_text("Ready");
    rt.invalidate();
    rt.frame(0.0);
    rt
}

fn focused_key(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|element| element.focused)
        .and_then(|element| element.key)
}

fn settle(rt: &mut Runtime<State>) {
    for _ in 0..120 {
        if !rt.frame(1.0 / 60.0) {
            break;
        }
    }
}

fn contrast(a: Color, b: Color) -> f32 {
    let (a, b) = (a.luminance(), b.luminance());
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
