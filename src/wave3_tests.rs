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
        rt.state.current_mut().apply(json!({"type":"tool_execution_start","toolCallId":"dispatch","toolName":"subagent",
            "args":{"tasks":[{"agent":"scout","task":"Find the source"},{"agent":"test","task":"Check it"}]}}));
        rt.state
            .right_panels
            .open(&rt.state.panel_key(), SingletonKind::Work);
        rt.frame(0.0);
        let panel = bounds(&rt, "agents-panel", width);
        for index in 0..2 {
            let row = bounds(&rt, &format!("agent-row-dispatch-{index}"), width);
            assert_eq!(row[3], 44.0);
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
fn work_tab_stacks_subagents_above_the_crew_and_says_when_nothing_runs() {
    // A plain thread with no subagents and no crew still has the tab, with a note.
    let mut plain = runtime(1280.0);
    let key = plain.state.panel_key();
    plain.state.right_panels.open(&key, SingletonKind::Work);
    plain.frame(0.0);
    bounds(&plain, "right-panel-segment-work", 1280.0);
    bounds(&plain, "work-empty", 1280.0);

    // A Captain home whose own turn also started a subagent shows both lists, the
    // subagents first, with the roster footer still on screen.
    let mut rt = runtime(1280.0);
    crate::captain_workspace_demo(&mut rt.state);
    rt.state.use_pi_for_test();
    rt.state.current_mut().apply(
        json!({"type":"tool_execution_start","toolCallId":"dispatch","toolName":"subagent",
        "args":{"tasks":[{"agent":"scout","task":"Find the source"}]}}),
    );
    let key = rt.state.panel_key();
    rt.state.right_panels.open(&key, SingletonKind::Work);
    rt.invalidate();
    rt.frame(0.0);
    let agents = bounds(&rt, "work-agents", 1280.0);
    let agent = bounds(&rt, "agent-row-dispatch-0", 1280.0);
    let roster = bounds(&rt, "crew-roster-rows", 1280.0);
    let footer = bounds(&rt, "crew-roster-footer", 1280.0);
    assert!(
        agent[1] + agent[3] <= agents[1] + agents[3] + 0.5,
        "{agent:?} {agents:?}"
    );
    assert!(
        agents[1] + agents[3] <= roster[1] + 0.5,
        "{agents:?} {roster:?}"
    );
    assert!(roster[3] >= 44.0, "{roster:?}");
    assert!(
        roster[1] + roster[3] <= footer[1] + 0.5,
        "{roster:?} {footer:?}"
    );
}

#[test]
fn details_card_docks_clear_of_the_lane_and_opens_a_subagent_in_the_work_tab() {
    // A column this wide docks the card in its corner, and the lane stops short of it.
    let mut rt = runtime(1600.0);
    rt.state.current_mut().apply(
        json!({"type":"tool_execution_start","toolCallId":"dispatch","toolName":"subagent",
        "args":{"tasks":[{"agent":"scout","task":"Find the source"}]}}),
    );
    rt.invalidate();
    rt.frame(0.0);
    rt.frame(0.25);
    let card = bounds(&rt, "details-card", 1600.0);
    let lane = rt.presented("chat-lane").expect("lane").0;
    assert!(
        lane[0] + lane[2] <= card[0] - 32.0 + 0.5,
        "{lane:?} {card:?}"
    );

    // The running subagent is a row in the Lineage card. Pressing it opens the Work tab
    // with that agent unfolded.
    press(&mut rt, "lineage-row-agent-dispatch-0");
    rt.frame(0.5);
    rt.frame(1.0);
    assert_eq!(
        rt.state.right_panel_active_view(),
        Some(crate::right_panel::PanelView::Work)
    );
    assert!(rt.presented("agent-detail-dispatch-0").is_some());

    // Beside the open panel the column is too narrow to dock the card, so the top bar's
    // button opens it as a popover.
    assert!(rt.presented("details-card").is_none());
    press(&mut rt, "header-details");
    rt.frame(1.0);
    bounds(&rt, "details-card", 1600.0);
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

/// KSG's `AssistantChangedFilesSection`: a finished turn that touched files in
/// two directories shows the card with the summed count, folders start
/// collapsed (`persistedExpanded ?? false`), the chevrons expand them all, and
/// "Open diff" opens the right panel's Changes tab.
#[test]
fn changed_files_card_counts_expands_and_open_diff_shows_the_panel() {
    let mut rt = runtime(1280.0);
    rt.state.populate_demo();
    {
        let t = rt.state.current_mut();
        t.messages.truncate(2);
        t.tools.clear();
        t.thoughts.clear();
        t.work.clear();
        t.messages.push(crate::model::Message {
            id: "cf-reply".into(),
            role: "Pi",
            text: "Done.".into(),
            images: Vec::new(),
            context: None,
        });
        t.work.push(crate::model::AssistantWork {
            start: 1,
            stopped: false,
            stopped_order: 0,
            expanded: false,
            elapsed: Some(42),
            outcome: Some(Status::Completed),
        });
        let change = |path: &str, additions: usize, deletions: usize| crate::work::FileChange {
            path: path.into(),
            old_path: None,
            operation: "update".into(),
            diff: String::new(),
            additions,
            deletions,
        };
        t.tools.push(Tool {
            id: "cf-edit".into(),
            kind: None,
            changes: vec![
                change("src/transport/socket.ts", 18, 4),
                change("tests/reconnect.test.ts", 47, 0),
            ],
            input: serde_json::Value::Null,
            raw_expanded: false,
            order: 0,
            name: "edit".into(),
            detail: "src/transport/socket.ts".into(),
            done: true,
            failed: false,
            output: String::new(),
            anchor: crate::model::ToolAnchor {
                message: 1,
                offset: 0,
            },
            block: 0,
            expanded: false,
            group_expanded: false,
            started: None,
            duration_ms: Some(120),
            subagents: vec![],
            images: vec![],
            structured: serde_json::Value::Null,
            parent_call: None,
            exit_code: None,
        });
    }
    rt.frame(0.0);
    let turn_key = {
        let t = rt.state.current();
        format!("message-{}-id-{}", t.id, t.messages[1].id)
    };
    let key = |kind: &str, rest: &str| format!("changed-files-{kind}-{turn_key}{rest}");
    assert!(rt.presented(&format!("changed-files-{turn_key}")).is_some());
    assert_eq!(
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some(&key("count", "")))
            .and_then(|element| element.text)
            .as_deref(),
        Some("2 changed files")
    );
    // Collapsed by default: the folder rows show, the file rows do not.
    assert!(rt.presented(&key("dir", "-src/transport/")).is_some());
    assert!(rt.presented(&key("dir", "-tests/")).is_some());
    assert!(
        rt.presented(&key("file", "-src/transport/socket.ts"))
            .is_none()
    );
    press(&mut rt, &format!("changed-files-toggle-{turn_key}"));
    assert!(
        rt.presented(&key("file", "-src/transport/socket.ts"))
            .is_some()
    );
    assert!(
        rt.presented(&key("file", "-tests/reconnect.test.ts"))
            .is_some()
    );
    press(&mut rt, &format!("changed-files-open-diff-{turn_key}"));
    let panel_key = rt.state.panel_key();
    let panel = rt.state.right_panels.get(&panel_key);
    assert!(panel.open);
    assert_eq!(panel.view, crate::right_panel::PanelView::Diff);
}
