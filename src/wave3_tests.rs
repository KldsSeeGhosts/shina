use super::tests::{press, runtime};
use super::*;
use crate::review::DiffBlocker;
use crate::right_panel::SingletonKind;

const WIDTHS: [f32; 4] = [620.0, 960.0, 1280.0, 1920.0];

fn key(rt: &mut rune::Runtime<State>, key: rune::Key) {
    rt.key(rune::KeyEvent {
        key,
        mods: Default::default(),
    });
    rt.frame(0.0);
}

fn bounds(rt: &rune::Runtime<State>, key: &str, width: f32) -> [f32; 4] {
    let (rect, _) = rt
        .presented(key)
        .unwrap_or_else(|| panic!("{width}: {key} missing"));
    assert!(
        rect[0] >= -0.5
            && rect[0] + rect[2] <= width + 0.5
            && rect[1] >= -0.5
            && rect[1] + rect[3] <= 720.5,
        "{width}: {key} {rect:?}"
    );
    rect
}

fn node<'a>(tree: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    if tree["key"] == key {
        return Some(tree);
    }
    tree["children"]
        .as_array()?
        .iter()
        .find_map(|child| node(child, key))
}

#[test]
fn switcher_centers_deduplicates_and_keeps_empty_enter_open() {
    for width in WIDTHS {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        let id = rt.state.current().id;
        rt.state.open_quick_switcher();
        rt.frame(0.0);
        let rect = bounds(&rt, "quick-switcher", width);
        assert!((rect[0] + rect[2] / 2.0 - width / 2.0).abs() <= 0.5);
        let keys: Vec<_> = rt
            .elements()
            .into_iter()
            .filter_map(|element| element.key)
            .filter(|key| key.starts_with("quick-switcher-row-"))
            .collect();
        assert!(!keys.is_empty());
        assert!(
            !keys.iter().any(
                |key| key.ends_with("#dup") || key == &format!("quick-switcher-row-local-{id}")
            )
        );
        assert_eq!(
            keys.len(),
            keys.iter().collect::<std::collections::HashSet<_>>().len()
        );
        let tree = rt.accessibility_json();
        assert_eq!(
            node(&tree, "quick-switcher-list").unwrap()["role"],
            "list-box"
        );
        assert_eq!(
            keys.iter()
                .filter(|key| node(&tree, key).is_some_and(|row| {
                    row["role"] == "list-box-option"
                        && row["states"].as_array().is_some_and(|states| {
                            states
                                .iter()
                                .any(|state| state == "checked" || state == "selected")
                        })
                }))
                .count(),
            1
        );
        key(&mut rt, rune::Key::Down);
        assert_eq!(rt.state.quick_switcher_highlight, 1);
        rt.state.quick_switcher_query.replace_text("zzzzzzzzzzzz");
        rt.invalidate();
        rt.frame(0.0);
        assert!(rt.presented("quick-switcher-hint-open").is_none());
        key(&mut rt, rune::Key::Enter);
        assert!(rt.state.quick_switcher);
        assert_eq!(rt.state.current().id, id);
    }
}

#[test]
fn agents_tab_and_jump_use_the_originating_thread() {
    for width in WIDTHS {
        let mut rt = runtime(width);
        rt.state.compact_sidebar = width < 760.0;
        rt.state.current_mut().title = "Investigate deployment".into();
        rt.state.current_mut().renamed = true;
        let thread = rt.state.current().id;
        assert!(!inspector_ui::tabs(&rt.state).contains(&crate::right_panel::PanelView::Agents));
        rt.state.current_mut().apply(json!({"type":"tool_execution_start","toolCallId":"dispatch","toolName":"subagent",
            "args":{"tasks":[{"agent":"scout","task":"Find the source"},{"agent":"test","task":"Check it"}]}}));
        rt.state
            .right_panels
            .open(&rt.state.panel_key(), SingletonKind::Agents);
        rt.frame(0.0);
        let panel = bounds(&rt, "agents-panel", width);
        for index in 0..2 {
            let row = bounds(&rt, &format!("agent-row-dispatch-{index}"), width);
            assert_eq!(row[3], 52.0);
            assert!(row[0] >= panel[0] && row[0] + row[2] <= panel[0] + panel[2]);
        }
        assert!(rt.presented(&format!("thread-agents-{thread}")).is_some());
        press(&mut rt, "agent-jump-dispatch-0");
        assert_eq!(
            rt.state.flash_tool.as_ref().map(|(id, _)| id.as_str()),
            Some("dispatch")
        );
        rt.state
            .current_mut()
            .apply(json!({"type":"tool_execution_end","toolCallId":"dispatch","result":{"details":{"results":[
                {"agent":"scout","task":"Find the source","finished":true},
                {"agent":"test","task":"Check it","finished":true}
            ]}}}));
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(rt.presented(&format!("thread-agents-{thread}")).is_none());
    }
}

#[test]
fn diff_blockers_keep_paths_and_actions_inside_the_panel() {
    for width in WIDTHS {
        let mut rt = runtime(width);
        let key = rt.state.panel_key();
        rt.state.right_panels.open(&key, SingletonKind::Diff);
        for blocker in [
            DiffBlocker::NotARepository {
                path: "/workspace/notes".into(),
            },
            DiffBlocker::OutsideProjects {
                path: "/workspace/source".into(),
                server: "Studio".into(),
            },
            DiffBlocker::WorktreeMissing {
                path: "/workspace/worktree".into(),
                branch: Some("repair".into()),
            },
            DiffBlocker::MachineOffline {
                machine: "Studio".into(),
            },
            DiffBlocker::GitFailed {
                path: "/workspace/source".into(),
                detail: "raw helper diagnostic".into(),
            },
        ] {
            rt.state.diffs.insert(
                key.clone(),
                diff_panel::ThreadDiff {
                    done: Some(diff_panel::Done::Blocked(blocker)),
                    ..Default::default()
                },
            );
            rt.invalidate();
            rt.frame(0.0);
            bounds(&rt, "diff-state-path", width);
            bounds(&rt, "diff-state-action", width);
            assert!(
                !rt.accessibility_json()
                    .to_string()
                    .contains("raw helper diagnostic")
            );
        }
    }
}

#[test]
fn search_pending_empty_and_clear_keep_one_query_identity() {
    for width in WIDTHS {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        rt.state.compact_sidebar = width < 760.0;
        rt.state.search.replace_text("zzzzzzzzzzzz");
        rt.state.sync_sidebar_search_input();
        rt.state.sidebar_search.pending = true;
        rt.frame(0.0);
        assert!(rt.presented("sidebar-search-header").is_some());
        let tree = rt.accessibility_json();
        assert_eq!(
            node(&tree, "sidebar-search-pending").unwrap()["role"],
            "status"
        );
        assert!(rt.presented("sidebar-empty").is_none());
        rt.state.sidebar_search.pending = false;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        bounds(&rt, "sidebar-empty", width);
        press(&mut rt, "sidebar-search-clear");
        assert!(rt.state.search.text.is_empty());
        rt.frame(1.0);
        assert!(rt.presented("sidebar-search-header").is_none());
    }
}

#[test]
fn footer_and_settings_share_update_copy_and_hide_raw_details_from_overview() {
    for width in WIDTHS {
        let mut rt = runtime(width);
        rt.state.compact_sidebar = width < 760.0;
        rt.state.updater = crate::update::Updater::test_rate_limited();
        rt.state
            .right_panels
            .open(&rt.state.panel_key(), SingletonKind::Overview);
        rt.frame(0.0);
        let summary = rt.state.update_summary();
        let tree = rt.accessibility_json();
        assert_eq!(
            node(&tree, "footer-update").unwrap()["name"],
            summary.headline
        );
        assert!(!tree.to_string().contains("API rate limit exceeded"));
        press(&mut rt, "footer-update");
        bounds(&rt, "update-popover", width);
        assert!(rt.presented("retry-update").is_none());
        let elements = rt.elements();
        assert!(elements.iter().any(|element| element.key.as_deref()
            == Some("update-popover-headline")
            && element.text.as_deref() == Some(summary.headline.as_str())));
        press(&mut rt, "open-update-settings");
        assert!(rt.elements().iter().any(|element| element.key.as_deref()
            == Some("settings-update-headline")
            && element.text.as_deref() == Some(summary.headline.as_str())));
        assert!(rt.presented("settings-update-log").is_some());
    }
}

#[test]
fn footer_marks_update_phases_and_keeps_idle_quiet() {
    let mut rt = runtime(1280.0);
    for (updater, expected, tint) in [
        (
            crate::update::Updater::test_checkable(),
            "Check for updates",
            Color::TRANSPARENT,
        ),
        (
            crate::update::Updater::test_checking(),
            "Checking for updates",
            Color::TRANSPARENT,
        ),
        (
            crate::update::Updater::test_downloading("v9.0"),
            "Downloading v9.0",
            palette(false).sea_bg,
        ),
        (
            crate::update::Updater::test_suspended(),
            "Update failed",
            palette(false).brass_bg,
        ),
        (
            crate::update::Updater::test_failed_release(),
            "Update failed",
            palette(false).brass_bg,
        ),
    ] {
        rt.state.updater = updater;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let tree = rt.accessibility_json();
        assert_eq!(node(&tree, "footer-update").unwrap()["name"], expected);
        let footer = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("footer-update"))
            .unwrap();
        assert_eq!(footer.fill, tint, "{expected}");
        assert!(!tree.to_string().contains("check-updates"));
    }
}

#[test]
fn sidebar_mounts_only_a_window_and_preserves_full_navigation() {
    for width in WIDTHS {
        for count in [200, 2_000] {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            for id in 10..10 + count {
                let mut thread = Thread::new(id);
                thread.title = format!("Conversation with a long title, number {id}");
                thread.never_sent = false;
                thread.life.latest_user_message_at = Some(id as u64);
                rt.state.threads.push(thread);
            }
            rt.frame(0.0);
            let list = bounds(&rt, "sidebar-scroll", width);
            let mounted = rt
                .elements()
                .into_iter()
                .filter(|e| {
                    e.key
                        .as_deref()
                        .is_some_and(|key| key.starts_with("thread-row-title-"))
                })
                .count();
            assert!(
                mounted > 0 && mounted < 100,
                "{width}/{count}: {mounted} mounted"
            );
            let rebuilds = rt.state.sidebar_index.borrow().rebuilds;
            assert!(sidebar_navigation(&rt.state).len() >= count);
            rt.wheel(list[0] + 20.0, list[1] + 90.0, -2000.0);
            rt.frame(0.0);
            rt.invalidate();
            rt.frame(0.0);
            assert_eq!(rt.state.sidebar_index.borrow().rebuilds, rebuilds);
        }
    }
}

#[test]
fn wave_three_status_colors_and_linear_hover_are_readable() {
    for dark in [true, false] {
        let p = palette(dark);
        let under = [p.bg, p.panel, p.card].map(|surface| surface.mix(p.selected, p.selected.a));
        for text in [p.ink, p.muted, p.faint, p.ok, p.brass, p.danger, p.sea] {
            for fill in under.into_iter().chain([p.lift, p.code_bg, p.tray]) {
                let (a, b) = (text.luminance(), fill.luminance());
                assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
            }
        }
        if dark {
            assert!(under[0].luminance() < p.row.luminance());
        }
    }
}

#[test]
fn captain_breadcrumb_has_one_segment_until_the_home_is_renamed() {
    for width in WIDTHS {
        let mut rt = runtime(width);
        rt.state.tabs_enabled = false;
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.open_captain(2);
        let name = rt.state.captains[2].name.clone();
        rt.state.current_mut().title = format!(" {} ", name.to_uppercase());
        rt.state.current_mut().renamed = true;
        rt.frame(0.0);
        let tree = rt.accessibility_json();
        let title = node(&tree, "thread-title").unwrap()["name"]
            .as_str()
            .unwrap();
        assert_eq!(title.matches(&name).count(), 1, "{title}");
        assert!(!title.contains(" / "));
        rt.state.current_mut().title = "Review the first batch".into();
        rt.invalidate();
        rt.frame(0.0);
        let tree = rt.accessibility_json();
        assert!(
            node(&tree, "thread-title").unwrap()["name"]
                .as_str()
                .unwrap()
                .contains(&format!("{name} / Review the first batch"))
        );
    }
}

#[test]
fn menu_reasons_match_the_lifecycle_gate() {
    for width in WIDTHS {
        let mut rt = runtime(width);
        let id = rt.state.current().id;
        rt.state.current_mut().status = Status::Running;
        rt.state.current_mut().session = Some("example-session".into());
        rt.state.current_mut().request = Some(json!({"method":"confirm"}));
        rt.state.thread_menu = true;
        rt.frame(0.0);
        let tree = rt.accessibility_json();
        for (key, why) in [
            ("menu-pin-thread", rt.state.lifecycle_blocker(id)),
            ("menu-settle-thread", rt.state.lifecycle_blocker(id)),
            ("menu-snooze-thread", rt.state.snooze_blocker(id)),
            (
                "menu-fork-message",
                rt.state.rewind_blocker(rt.state.current()),
            ),
        ] {
            let why = why.expect("blocked");
            let row = node(&tree, key).unwrap_or_else(|| panic!("missing {key}"));
            assert_eq!(row["description"], why, "{width}: {key}");
            assert!(
                rt.elements().iter().any(|element| element.key.as_deref()
                    == Some(&format!("{key}-reason"))
                    && element.text.as_deref() == Some(&why)),
                "{width}: {key} inline reason"
            );
        }
    }
}

#[test]
fn access_picker_focuses_and_checks_the_current_radio() {
    use crate::model::AccessMode;
    for (width, mode) in WIDTHS.into_iter().zip([
        AccessMode::ReadOnly,
        AccessMode::Supervised,
        AccessMode::Edits,
        AccessMode::Full,
    ]) {
        let mut rt = runtime(width);
        rt.state.current_mut().harness = "pi".into();
        rt.state.current_mut().native.access = mode;
        rt.frame(0.0);
        rt.frame(1.0);
        rt.state.open_access_picker();
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let tree = rt.accessibility_json();
        let radio = format!("access-{}", mode.id());
        let states = node(&tree, &radio).unwrap()["states"].as_array().unwrap();
        assert!(
            states.iter().any(|state| state == "checked"),
            "{radio}: {states:?}"
        );
        let group = node(&tree, "access-picker").unwrap();
        assert_eq!(group["role"], "radio-group");
        assert_eq!(group["name"], "Access mode");
        assert!(rt.presented(&format!("{radio}-check")).is_some());
        key(&mut rt, rune::Key::Enter);
        assert_eq!(
            rt.state.current().native.access,
            mode,
            "{radio} did not have focus"
        );
        assert!(!rt.state.current().native.access_picker);
    }
}
