use super::tests::{press, runtime};
use super::*;
use crate::review::DiffBlocker;
use crate::right_panel::SingletonKind;

const WIDTHS: [f32; 2] = [620.0, 1280.0];

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
