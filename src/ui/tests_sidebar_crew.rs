use super::*;

#[test]
fn a_new_roster_is_empty_and_adding_a_captain_opens_the_selector_then_editor() {
    let mut rt = runtime(1280.0);
    rt.frame(0.0);
    assert!(rt.state.captains.is_empty());
    press(&mut rt, "assign-captain");
    assert!(rt.state.captain_picker);
    assert!(rt.state.project_picker.is_none());
    assert!(rt.find("choose-no-captain").is_some());
    press(&mut rt, "picker-new-captain");
    assert!(rt.state.captain_editor.is_some());
    assert!(!rt.state.captain_picker);
    let editor = rt.state.captain_editor.as_mut().unwrap();
    assert!(editor.role.text.is_empty(), "no default role");
    assert!(
        !editor.name.text.trim().is_empty(),
        "suggested name is pre-filled"
    );
    editor.name.replace_text("Riku");
    rt.invalidate();
    rt.frame(0.5);
    press(&mut rt, "save-captain");
    assert_eq!(rt.state.captains.len(), 1);
    assert_eq!(rt.state.captains[0].role, "");
    assert!(!rt.state.captains[0].prompt().contains("Role:"));
    assert_eq!(rt.state.current().captain, Some(0));
}

#[test]
fn add_workspace_is_a_palette_that_browses_adds_and_creates_folders() {
    let root = std::env::temp_dir().join(format!("shina-add-{}", std::process::id()));
    std::fs::create_dir_all(root.join("alpha").join("inner")).unwrap();
    std::fs::create_dir_all(root.join("beta")).unwrap();
    let mut rt = runtime(1280.0);
    rt.frame(0.0);
    // Add workspace inside the New thread picker opens the palette, not a menu.
    rt.state.open_picker(ProjectPicker::NewThread);
    rt.invalidate();
    rt.frame(0.5);
    press(&mut rt, "picker-add-project");
    assert_eq!(rt.state.project_picker, Some(ProjectPicker::AddProject));
    assert!(rt.find("project-open-folder").is_some());
    press(&mut rt, "add-local-folder");
    assert!(rt.state.picker_browsing);
    let sep = std::path::MAIN_SEPARATOR;
    rt.state
        .picker_query
        .replace_text(format!("{}{sep}al", root.display()));
    finish_folder_listing(&mut rt);
    rt.invalidate();
    rt.frame(1.0);
    assert!(rt.find("browse-0").is_some() && rt.find("browse-1").is_none());
    let key = |key, cmd| rune::KeyEvent {
        key,
        mods: rune::Mods {
            cmd,
            ctrl: cmd,
            ..Default::default()
        },
    };
    rt.key(key(rune::Key::Enter, false));
    rt.frame(1.5);
    assert!(rt.state.picker_query.text.ends_with(&format!("alpha{sep}")));
    finish_folder_listing(&mut rt);
    rt.key(key(rune::Key::Enter, true));
    rt.frame(2.0);
    assert!(rt.state.project_picker.is_none());
    let added = |s: &State, name: &str| {
        s.projects
            .entries
            .iter()
            .any(|p| p.path.file_name().is_some_and(|n| n == name))
    };
    assert!(added(&rt.state, "alpha"));
    // A path that does not exist yet is created, then added.
    press(&mut rt, "project-switcher");
    press(&mut rt, "add-local-folder");
    rt.state
        .picker_query
        .replace_text(format!("{}{sep}gamma", root.display()));
    finish_folder_listing(&mut rt);
    rt.invalidate();
    rt.frame(3.0);
    press(&mut rt, "browse-add");
    assert!(root.join("gamma").is_dir());
    assert!(added(&rt.state, "gamma"));
    // Several workspaces in view: New thread asks which one.
    press(&mut rt, "new-thread");
    assert_eq!(rt.state.project_picker, Some(ProjectPicker::NewThread));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_filter_precedes_and_filters_both_captains_and_threads() {
    let mut rt = runtime(1280.0);
    rt.state.populate_demo();
    rt.frame(0.0);
    let (filter, _) = rt.presented("project-scope").unwrap();
    let (captain, _) = rt.presented("thread-7").unwrap();
    assert!(filter[1] < captain[1]);
    press(&mut rt, "project-scope");
    press(&mut rt, "choose-project-1");
    rt.frame(1.0);
    assert!(rt.find("thread-7").is_some());
    assert!(rt.find("thread-8").is_none());
    assert!(rt.find("thread-9").is_none());
    assert!(rt.find("thread-3").is_none());
    rt.state.search.replace_text("no such conversation");
    rt.invalidate();
    rt.frame(1.0);
    assert!(rt.find("thread-7").is_none());
    assert!(rt.find("assign-captain").is_none());
}

#[test]
fn sidebar_context_and_header_share_rename_pin_archive_and_delete_actions() {
    let mut rt = runtime(1280.0);
    rt.state.current_mut().draft.replace_text("A prompt");
    rt.frame(0.0);
    let (x, y) = rt.find("thread-1").unwrap();
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(0.0);
    assert_eq!(rt.state.row_menu, Some(RowTarget::Thread(1)));
    for key in [
        "menu-rename-thread",
        "menu-pin-thread",
        "menu-archive-thread",
        "menu-delete-thread",
    ] {
        assert!(rt.find(key).is_some(), "{key} missing from row menu");
    }
    press(&mut rt, "menu-rename-thread");
    rt.state.rename_edit.replace_text("Renamed in sidebar");
    rt.frame(1.0);
    press(&mut rt, "save-thread-name");
    assert_eq!(rt.state.current().title, "Renamed in sidebar");
    press(&mut rt, "thread-actions");
    press(&mut rt, "menu-pin-thread");
    assert!(rt.state.current().pinned());
    // The row has no actions button; a right-click opens its menu at the pointer.
    assert!(rt.find("thread-more-1").is_none());
    let (x, y) = rt.find("thread-1").unwrap();
    rt.secondary_down(x + 30.0, y);
    rt.secondary_up(x + 30.0, y);
    rt.frame(0.0);
    assert!(rt.state.row_menu_at.is_some());
    press(&mut rt, "menu-delete-thread");
    assert_eq!(
        rt.state.delete_thread,
        Some(1),
        "delete menu opens confirmation"
    );
    assert_eq!(
        rt.state.threads.len(),
        1,
        "confirmation does not delete yet"
    );
    press(&mut rt, "cancel-thread-dialog");
    assert_eq!(rt.state.threads.len(), 1);
    press(&mut rt, "thread-actions");
    press(&mut rt, "menu-delete-thread");
    press(&mut rt, "confirm-delete-thread");
    assert!(rt.state.threads.iter().all(|t| t.id != 1));
    assert!(rt.state.projects.entries[0].path.is_dir());
}

fn crew_runtime(width: f32) -> Runtime<State> {
    let mut rt = runtime(width);
    crate::captain_workspace_demo(&mut rt.state);
    rt.state.use_pi_for_test();
    rt.invalidate();
    rt.frame(0.0);
    rt
}

#[test]
fn captain_crew_surfaces_fit_and_open_workers_at_supported_widths() {
    for width in [1280.0] {
        let mut rt = crew_runtime(width);
        assert_eq!(rt.state.current().id, 7);
        rt.invalidate();
        rt.frame(0.0);
        assert_inside(
            &mut rt,
            width,
            &[
                "task-card-16",
                "task-clock-16",
                "result-open-1",
                "needs-you",
                "needs-you-2",
                "approve-2",
                "decline-2",
                "composer",
            ],
        );
        // Delegations are subagent rows on the tool rows' rhythm, not generic tool rows.
        // Every crew member has a row in the Crew tab.
        assert!(rt.presented("inline-tool-7-0").is_none());
        let card = rt.presented("task-card-16").unwrap().0;
        assert!((card[3] - 24.0).abs() < 0.5, "{width}: {card:?}");
        rt.state.right_panels.open(
            &rt.state.panel_key(),
            crate::right_panel::SingletonKind::Work,
        );
        rt.invalidate();
        rt.frame(0.0);
        assert_inside(
            &mut rt,
            width,
            &[
                "crew-roster-rows",
                "crew-chip-1",
                "crew-chip-16",
                "crew-chip-18",
                "crew-chip-19",
                "crew-roster-footer",
                "crew-roster-new",
                "crew-roster-recruit",
            ],
        );
        press(&mut rt, "crew-chip-16");
        rt.frame(1.0);
        assert_eq!(rt.state.current().id, 16, "{width}");
        // A healthy worker has nothing between the conversation and the composer.
        assert!(rt.presented("parent-bar").is_none(), "{width}");
        assert_inside(&mut rt, width, &["composer"]);
        // The worker's details card links back to its captain. KSG's 256 px sidebar
        // leaves the lane wide enough for the card to dock under the covering Inspector.
        assert!(rt.presented("details-card").is_some(), "{width}");
        press(&mut rt, "open-captain-from-overview");
        assert_eq!(rt.state.current().id, 7, "{width}");
        press(&mut rt, "needs-you-source-2");
        assert_eq!(rt.state.current().id, 2, "{width}");
        // A stopped worker offers Resume and Dismiss on its own notice bar.
        crew_ui::open_thread(&mut rt.state, 18);
        rt.invalidate();
        rt.frame(0.0);
        assert_eq!(rt.state.current().id, 18, "{width}");
        assert_inside(
            &mut rt,
            width,
            &[
                "parent-bar",
                "resume-task-18",
                "dismiss-task-18",
                "composer",
            ],
        );
    }
}

#[test]
fn the_crew_tab_lists_each_member_once_grouped_by_state_at_all_widths() {
    for width in [1280.0] {
        let mut rt = crew_runtime(width);
        assert!(rt.presented("crew-roster-rows").is_none());
        rt.state.right_panels.open(
            &rt.state.panel_key(),
            crate::right_panel::SingletonKind::Work,
        );
        rt.invalidate();
        rt.frame(0.0);
        assert_eq!(
            rt.state.right_panel_active_view(),
            Some(crate::right_panel::PanelView::Work),
            "{width}"
        );
        assert_inside(
            &mut rt,
            width,
            &[
                "crew-tab",
                "crew-roster-rows",
                "crew-roster-footer",
                "crew-roster-new",
                "crew-roster-recruit",
            ],
        );
        // Every member has exactly one row, 44 pt, filed under its group.
        for id in [1, 2, 16, 17, 18, 19] {
            let chip = format!("crew-chip-{id}");
            let (row, _) = rt
                .presented(&chip)
                .unwrap_or_else(|| panic!("{width}: {chip} missing"));
            assert!((row[3] - 44.0).abs() < 0.5, "{width}: {chip} {row:?}");
        }
        // The groups appear in the spec's order: needs you leads, queued before idle.
        for group in [
            "crew-group-needs-you",
            "crew-group-working",
            "crew-group-queued",
        ] {
            assert!(rt.presented(group).is_some(), "{width}: {group} missing");
        }
        let needs = rt.presented("crew-group-needs-you").unwrap().0;
        let working = rt.presented("crew-group-working").unwrap().0;
        let queued = rt.presented("crew-group-queued").unwrap().0;
        assert!(
            needs[1] < working[1] && working[1] < queued[1],
            "{width}: groups out of order"
        );
    }
}

#[test]
fn composer_request_banner_pages_thread_and_crew_requests() {
    // Unified request surface inside ComposerSurface.Shell: a Captain home with both
    // its own pending confirm dialog and a crew approval pages both in one banner.
    let mut rt = crew_runtime(1280.0);
    rt.state.current_mut().apply(json!({
        "type": "extension_ui_request",
        "id": "home-confirm",
        "method": "confirm",
        "title": "Run cargo test?",
        "message": "Allow running cargo test in this workspace."
    }));
    rt.invalidate();
    rt.frame(0.0);
    let shell = rt.presented("composer-shell").unwrap().0;
    let dialog = rt.presented("pi-dialog").unwrap().0;
    assert!(
        dialog[1] >= shell[1] - 1.0 && dialog[1] + dialog[3] <= shell[1] + shell[3] + 1.0,
        "request banner should dock inside composer-shell: {dialog:?} vs {shell:?}"
    );
    assert!(rt.presented("needs-you-2").is_none());
    assert_eq!(
        element_text(&rt, "needs-you-page-count").as_deref(),
        Some("1/2")
    );
    press(&mut rt, "needs-you-next");
    rt.frame(0.5);
    rt.frame(0.5);
    assert!(rt.presented("pi-dialog").is_none());
    assert!(rt.presented("needs-you-2").is_some());
    assert_eq!(
        element_text(&rt, "needs-you-page-count").as_deref(),
        Some("2/2")
    );
}

/// Opens `count` distinct threads. Each has a title and a sent message, so none is a blank
/// draft that `new_thread` would reuse.
fn open_distinct_threads(rt: &mut Runtime<State>, count: usize) {
    for n in 0..count {
        rt.state.new_thread();
        let thread = rt.state.current_mut();
        thread.title = format!("Thread number {n} with a longer title");
        thread.life.latest_user_message_at = Some(lifecycle::now());
    }
}

fn settle_frames(rt: &mut Runtime<State>) {
    rt.invalidate();
    rt.frame(0.0);
    for _ in 0..4 {
        rt.frame(1.0);
    }
}

/// The pressable's key is stable whether the tab is active or not, so a
/// double click always lands on the same element path.
fn tab_key(_rt: &Runtime<State>, id: usize) -> String {
    format!("tab-{id}")
}

#[test]
fn tabs_share_the_strip_and_the_active_tab_stays_whole() {
    for width in [620.0, 1280.0] {
        for extra in [0usize, 2, 6, 12] {
            let mut rt = runtime(width);
            open_distinct_threads(&mut rt, extra);
            rt.set_viewport((width, 800.0), 1.0);
            settle_frames(&mut rt);
            type TabStage = (&'static str, Box<dyn Fn(&mut Runtime<State>)>);
            let mut stages: Vec<TabStage> = vec![
                ("the last tab", Box::new(|_| {})),
                (
                    "the first tab",
                    Box::new(|rt| {
                        let first = rt.state.tabs[0];
                        let index = rt.state.threads.iter().position(|t| t.id == first);
                        rt.state.open_kept_tab(index.unwrap());
                    }),
                ),
                (
                    "the last tab again",
                    Box::new(|rt| {
                        let last = *rt.state.tabs.last().unwrap();
                        let index = rt.state.threads.iter().position(|t| t.id == last);
                        rt.state.open_kept_tab(index.unwrap());
                    }),
                ),
                (
                    "after closing the last tab",
                    Box::new(|rt| {
                        let last = *rt.state.tabs.last().unwrap();
                        rt.state.close_tab(last);
                    }),
                ),
                (
                    "after ten sidebar previews",
                    Box::new(|rt| {
                        // Ghost tabs: sidebar clicks replace one slot
                        // instead of growing the strip.
                        let base = rt.state.next_id;
                        for n in 0..10 {
                            let mut thread = crate::model::Thread::new(base + n);
                            thread.title = format!("Sidebar preview {n}");
                            rt.state.threads.push(thread);
                        }
                        rt.state.next_id = base + 10;
                        let before = rt.state.tabs.len();
                        for index in (rt.state.threads.len() - 10)..rt.state.threads.len() {
                            rt.state.show_thread(index);
                            // The first click adds the ghost slot; the
                            // next nine replace it in place.
                            let expected = before + 1;
                            assert_eq!(
                                rt.state.tabs.len(),
                                expected,
                                "sidebar clicks reuse the ghost tab"
                            );
                        }
                    }),
                ),
            ];
            for (stage, step) in stages.drain(..) {
                step(&mut rt);
                settle_frames(&mut rt);
                let (strip, _) = rt.presented("thread-tabs").unwrap();
                let (plus, _) = rt.presented("new-tab").unwrap();
                let active = rt.presented("active-tab").map(|(rect, _)| rect).unwrap();
                let label = format!("{width}/{extra}/{stage}");
                assert!(
                    plus[0] >= strip[0] + strip[2] - 0.5,
                    "{label}: the plus overlaps the strip {strip:?} {plus:?}"
                );
                assert!(
                    active[0] >= strip[0] - 0.5
                        && active[0] + active[2] <= strip[0] + strip[2] + 0.5,
                    "{label}: the active tab {active:?} is clipped by {strip:?}"
                );
                for id in rt.state.tabs.clone() {
                    let (tab, _) = rt.presented(&tab_key(&rt, id)).unwrap();
                    assert!(
                        tab[2] >= TAB_MIN_WIDTH - 0.5 && tab[2] <= TAB_MAX_WIDTH + 0.5,
                        "{label}: {tab:?}"
                    );
                }
                let count = rt.state.tabs.len() as f32;
                if count * (TAB_MIN_WIDTH + TAB_GAP) <= strip[2] {
                    // Everything fits at the minimum, so nothing scrolls and every tab is whole.
                    for id in rt.state.tabs.clone() {
                        let (tab, _) = rt.presented(&tab_key(&rt, id)).unwrap();
                        assert!(
                            tab[0] >= strip[0] - 0.5
                                && tab[0] + tab[2] <= strip[0] + strip[2] + 0.5,
                            "{label}: tab {tab:?} escapes {strip:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn send_and_stop_share_the_same_composer_pill_slot() {
    for (width, dark) in [
        (620.0, false),
        (960.0, true),
        (1280.0, false),
        (1920.0, true),
    ] {
        let p = palette(dark);
        let fill_of = |rt: &Runtime<State>, key: &str| {
            rt.elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("{key} is missing"))
                .fill
        };
        let mut rt = runtime(width);
        rt.state.dark = dark;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert_eq!(fill_of(&rt, "composer-transition-actions"), p.pill);
        rt.state.current_mut().draft.replace_text("Hello");
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert_eq!(fill_of(&rt, "composer-transition-actions"), p.pill);
        let (send, _) = rt.presented("send-message").unwrap();
        assert_eq!((send[2], send[3]), (32.0, 32.0));
        // Without a draft, the red stop mark takes the same slot.
        let t = rt.state.current_mut();
        t.draft.replace_text("");
        t.status = Status::Running;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert_eq!(fill_of(&rt, "composer-transition-actions"), p.pill);
        assert_eq!(fill_of(&rt, "composer-stop-mark"), p.danger);
        let (stop, _) = rt.presented("stop-agent").unwrap();
        assert_eq!((stop[2], stop[3]), (32.0, 32.0));
    }
}

#[test]
fn default_permissions_stay_quiet_and_open_from_model_picker_at_all_widths() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.set_viewport((width, 900.0), 1.0);
        rt.state.composer_ui.force_expanded = true;
        rt.state.catalog.models = vec![crate::session::AgentModel {
            provider: "cpa".into(),
            id: "gpt-6-astra".into(),
            name: "GPT-6 Astra".into(),
            ..Default::default()
        }];
        rt.state.current_mut().models = rt.state.catalog.models.clone();
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);

        assert!(
            rt.presented("access-mode").is_none(),
            "{width}: default permissions chip stays quiet"
        );

        press(&mut rt, "model-effort");
        rt.frame(0.1);
        let (card, _) = rt
            .presented("pi-settings-picker")
            .unwrap_or_else(|| panic!("{width}: model picker did not open"));
        let (perm, _) = rt
            .presented("model-picker-permissions")
            .unwrap_or_else(|| panic!("{width}: model-picker-permissions missing"));
        assert!(
            perm[0] >= card[0] - 0.5
                && perm[0] + perm[2] <= card[0] + card[2] + 0.5
                && perm[1] + perm[3] <= card[1] + card[3] + 0.5,
            "{width}: permissions footer outside card: {perm:?} in {card:?}"
        );

        press(&mut rt, "model-picker-permissions");
        rt.frame(0.1);
        assert_eq!(
            rt.state.settings_picker, None,
            "{width}: model picker closed"
        );
        assert!(
            rt.state.current().native.access_picker,
            "{width}: access picker opened from model picker footer"
        );
        let (picker, _) = rt
            .presented("access-picker")
            .unwrap_or_else(|| panic!("{width}: access-picker not presented"));
        assert!(
            picker[0] >= 0.0 && picker[0] + picker[2] <= width + 0.5 && picker[1] >= 0.0,
            "{width}: access picker out of viewport: {picker:?}"
        );

        // When busy, clicking the disabled footer row does not open the access picker.
        rt.state.current_mut().native.access_picker = false;
        rt.state.current_mut().status = Status::Running;
        rt.state.settings_picker = Some(SettingsPicker::Model);
        rt.invalidate();
        rt.frame(0.2);
        press(&mut rt, "model-picker-permissions");
        assert!(
            !rt.state.current().native.access_picker,
            "{width}: disabled permissions footer must not open access picker"
        );
    }
}

#[test]
fn tab_leaves_expanded_composer_when_trailing_controls_are_disabled() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.set_viewport((width, 900.0), 1.0);
        let non_default = if rt.state.thread_defaults.access == crate::model::AccessMode::Full.id()
        {
            crate::model::AccessMode::Supervised
        } else {
            crate::model::AccessMode::Full
        };
        {
            let t = rt.state.current_mut();
            t.messages.push(crate::model::Message {
                id: "m1".into(),
                role: "You",
                text: "Working".into(),
                images: vec![],
                context: None,
            });
            t.native.access = non_default;
            t.status = Status::Running;
            t.draft.focused = true;
        }
        rt.state.composer_ui.scope_focused.set(true);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        rt.update(|_, tx| tx.focus("composer-input", true));
        rt.frame(1.0);

        assert!(
            rt.presented("composer-prompt-body").is_some(),
            "{width}: expanded composer mounted while focused"
        );
        let mut cleared = false;
        for _ in 0..16 {
            rt.key(rune::KeyEvent {
                key: rune::Key::Tab,
                mods: Default::default(),
            });
            rt.frame(0.05);
            if !rt.state.composer_ui.scope_focused.get() {
                cleared = true;
                break;
            }
        }
        assert!(
            cleared,
            "{width}: Tab never cleared scope_focused (last_focusable={:?}, focused={:?})",
            rt.state.composer_ui.last_focusable.get(),
            rt.elements()
                .into_iter()
                .find(|e| e.focused)
                .and_then(|e| e.key),
        );
        if width < 640.0 {
            rt.frame(1.0);
            assert!(
                rt.presented("composer-prompt-body").is_none(),
                "{width}: narrow composer stays stuck open after Tab leaves"
            );
        }
    }
}

#[test]
fn tab_scope_and_disabled_labeled_access_chip_cover_demo_context_and_attachments() {
    // 1. Demo footer routes Tab on `leave-example` to `control_key` and collapses narrow composer.
    {
        let mut rt = runtime(620.0);
        rt.set_viewport((620.0, 900.0), 1.0);
        rt.state.demo = true;
        rt.state.current_mut().draft.focused = true;
        rt.state.composer_ui.scope_focused.set(true);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        rt.update(|_, tx| tx.focus("leave-example", true));
        rt.frame(0.1);
        assert!(rt.presented("composer-prompt-body").is_some());
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(1.0);
        assert!(
            !rt.state.composer_ui.scope_focused.get(),
            "Tab on demo leave-example must clear scope_focused"
        );
        assert!(
            rt.presented("composer-prompt-body").is_none(),
            "narrow demo composer should collapse after Tab leaves"
        );
    }

    // 2. Context ring with open breakdown popover clears scope_focused when Tab enters legend.
    {
        use crate::context_breakdown::{BreakdownRowKind, ContextBreakdownRow};
        let mut rt = context_fixture(620.0);
        rt.state.current_mut().context.usage.breakdown = Some(vec![
            ContextBreakdownRow {
                label: "Messages".into(),
                tokens: 80_000,
                kind: BreakdownRowKind::Estimated,
            },
            ContextBreakdownRow {
                label: "System".into(),
                tokens: 20_000,
                kind: BreakdownRowKind::Inferred,
            },
        ]);
        context_toggle(&mut rt.state);
        rt.state.composer_ui.scope_focused.set(true);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        rt.update(|_, tx| tx.focus("context-usage-ring", true));
        rt.frame(0.1);
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(0.1);
        assert!(
            !rt.state.composer_ui.scope_focused.get(),
            "Tab from context-usage-ring into breakdown popover must clear scope_focused"
        );
        assert!(
            focused_key(&rt)
                .as_deref()
                .is_some_and(|k| k.starts_with("context-breakdown-legend-")),
            "focus moved into breakdown legend: {:?}",
            focused_key(&rt)
        );
    }

    // 3. Shift+Tab from Remove stays inside composer on Preview, then leaves from Preview.
    {
        let mut rt = runtime(620.0);
        rt.set_viewport((620.0, 900.0), 1.0);
        rt.state.attach_image("shot.png", &ui_png(4));
        let id = rt.state.current().draft_images[0].id.clone();
        rt.state.composer_ui.scope_focused.set(true);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let remove_key = format!("remove-attachment-{id}");
        let thumb_key = format!("attachment-thumb-{id}");
        rt.update(|_, tx| tx.focus(&remove_key, true));
        rt.frame(0.1);
        let shift = rune::Mods {
            shift: true,
            ..Default::default()
        };
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: shift,
        });
        rt.frame(0.1);
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some(thumb_key.as_str()),
            "Shift+Tab from Remove lands on Preview"
        );
        assert!(
            rt.state.composer_ui.scope_focused.get(),
            "Shift+Tab from Remove must keep scope_focused while Preview is focused"
        );
        assert!(
            rt.presented("composer-prompt-body").is_some(),
            "narrow composer stays expanded while Preview is focused"
        );
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: shift,
        });
        rt.frame(1.0);
        assert!(
            !rt.state.composer_ui.scope_focused.get(),
            "Shift+Tab from Preview must clear scope_focused"
        );
        assert!(
            rt.presented("composer-prompt-body").is_none(),
            "narrow composer collapses after Shift+Tab leaves Preview"
        );
    }

    // 4. Disabled labeled access chip applies no hover or press wash and ignores presses.
    {
        let mut rt = runtime(1280.0);
        rt.set_viewport((1280.0, 900.0), 1.0);
        let non_default = if rt.state.thread_defaults.access == crate::model::AccessMode::Full.id()
        {
            crate::model::AccessMode::Supervised
        } else {
            crate::model::AccessMode::Full
        };
        let element_of = |rt: &Runtime<State>| {
            rt.elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("access-mode"))
                .expect("labeled access-mode chip should be mounted")
        };
        {
            let t = rt.state.current_mut();
            t.native.access = non_default;
            t.status = Status::Ready;
        }
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let (rect, _) = rt
            .presented("access-mode")
            .expect("labeled access-mode chip should be mounted");
        assert!(
            rect[2] > 40.0,
            "expected labeled access chip at 1280 px, got {rect:?}"
        );
        let (cx, cy) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
        // Sanity-check that the enabled chip does activate hover and press overlays.
        rt.pointer_move(cx, cy);
        rt.frame(1.5);
        assert!(
            element_of(&rt).hover > 0.0,
            "enabled labeled access chip should report hover state"
        );
        rt.pointer_down(cx, cy);
        rt.frame(2.0);
        assert!(
            element_of(&rt).press > 0.0,
            "enabled labeled access chip should report press state"
        );
        rt.pointer_up(cx, cy);

        let mut disabled_rt = runtime(1280.0);
        disabled_rt.set_viewport((1280.0, 900.0), 1.0);
        {
            let t = disabled_rt.state.current_mut();
            t.native.access = non_default;
            t.status = Status::Running;
        }
        disabled_rt.invalidate();
        disabled_rt.frame(0.0);
        disabled_rt.frame(1.0);
        let (drect, _) = disabled_rt
            .presented("access-mode")
            .expect("disabled labeled access-mode chip should be mounted");
        assert!(
            drect[2] > 40.0,
            "expected disabled labeled access chip at 1280 px, got {drect:?}"
        );
        let (dx, dy) = (drect[0] + drect[2] / 2.0, drect[1] + drect[3] / 2.0);
        disabled_rt.pointer_move(dx, dy);
        disabled_rt.frame(1.5);
        let hovered = element_of(&disabled_rt);
        assert_eq!(
            hovered.hover, 0.0,
            "disabled labeled access chip must not get a hover wash"
        );
        disabled_rt.pointer_down(dx, dy);
        disabled_rt.frame(2.0);
        let pressed = element_of(&disabled_rt);
        assert_eq!(
            pressed.press, 0.0,
            "disabled labeled access chip must not get a press wash"
        );
        disabled_rt.pointer_up(dx, dy);
        disabled_rt.frame(2.5);
        assert!(
            !disabled_rt.state.current().native.access_picker,
            "disabled labeled access chip must not open access picker"
        );
    }
}

#[test]
fn composer_shell_and_pill_rims_overlay_cleanly_and_disabled_pill_slots_do_nothing() {
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.set_viewport((width, 900.0), 1.0);
        rt.state.composer_ui.force_expanded = true;
        rt.state.current_mut().draft.replace_text("Check rims");
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);

        let (shell, _) = rt.presented("composer-shell").unwrap();
        let (shell_rim, _) = rt.presented("composer-shell-rim").unwrap();
        assert!(
            (shell[0] - shell_rim[0]).abs() < 0.5
                && (shell[1] - shell_rim[1]).abs() < 0.5
                && (shell[2] - shell_rim[2]).abs() < 0.5
                && (shell[3] - shell_rim[3]).abs() < 0.5,
            "{width}: shell rim mismatch: shell={shell:?} rim={shell_rim:?}"
        );
        let (pill, _) = rt.presented("composer-transition-actions").unwrap();
        let (pill_rim, _) = rt.presented("composer-pill-rim").unwrap();
        assert!(
            (pill[0] - pill_rim[0]).abs() < 0.5
                && (pill[1] - pill_rim[1]).abs() < 0.5
                && (pill[2] - pill_rim[2]).abs() < 0.5
                && (pill[3] - pill_rim[3]).abs() < 0.5,
            "{width}: pill rim mismatch: pill={pill:?} rim={pill_rim:?}"
        );

        // Disabled compact toggle (e.g. offline remote machine) does not toggle
        // keep_full_history on pointer press.
        let stale_at = crate::lifecycle::now().saturating_sub(80 * 60_000);
        let (client, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
        rt.state.client = Some(client);
        rt.state.machines.selected = "offline-remote".into();
        {
            let t = rt.state.current_mut();
            t.harness = "claude".into();
            t.model_provider = "claude".into();
            t.session = Some("11111111-1111-4111-8111-111111111111".into());
            t.status = Status::Completed;
            t.context.absorb(&json!({
                "contextUsage": {
                    "tokens": 220_000,
                    "contextWindow": 1_000_000,
                    "updatedAt": stale_at
                }
            }));
            t.messages.push(crate::model::Message {
                id: "m1".into(),
                role: "You",
                text: "Old".into(),
                images: vec![],
                context: None,
            });
        }
        rt.invalidate();
        rt.frame(0.1);
        assert!(
            rt.presented("composer-compact-send").is_some(),
            "{width}: compact toggle should be mounted"
        );
        let before = rt.state.keep_full_history(rt.state.current());
        press(&mut rt, "composer-compact-send");
        assert_eq!(
            rt.state.keep_full_history(rt.state.current()),
            before,
            "{width}: disabled compact toggle fired on press"
        );

        // Disabled stop button does not trigger stop on pointer press.
        rt.state.client = None;
        rt.state.machines.selected = "local".into();
        {
            let t = rt.state.current_mut();
            t.harness_state = json!({"turnCapabilities": {"supportsInterrupt": false}});
            t.draft.replace_text("");
            t.status = Status::Running;
        }
        rt.invalidate();
        rt.frame(0.1);
        assert!(
            rt.presented("stop-agent").is_some(),
            "{width}: stop button should be mounted"
        );
        press(&mut rt, "stop-agent");
        assert_eq!(
            rt.state.current().stop_control.phase(),
            None,
            "{width}: disabled stop button fired on press"
        );
        assert_ne!(
            rt.state.current().phase,
            "Stopping",
            "{width}: disabled stop button changed phase"
        );
    }
}
