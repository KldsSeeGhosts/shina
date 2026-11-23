use super::*;

#[test]
fn right_panel_lays_out_inline_or_covering_and_restores_the_conversation() {
    use crate::right_panel::SingletonKind;
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        rt.state.current_mut().draft.replace_text("A draft");
        // The Diff segment plus an open document viewer.
        let key = rt.state.panel_key();
        rt.state.right_panels.open(&key, SingletonKind::Diff);
        rt.state.captains = crate::captains::Captain::presets();
        rt.state.open_captain_document(&key, 0, "NOTES.md");
        rt.state.right_panels.open(&key, SingletonKind::Diff);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.5);
        rt.frame(0.5);

        let (panel, _) = rt
            .presented("right-panel")
            .unwrap_or_else(|| panic!("panel mounts at {width}"));
        let (main, _) = rt
            .presented("chat-main")
            .unwrap_or_else(|| panic!("conversation column mounts at {width}"));
        let inside = |rect: [f32; 4]| {
            rect[0] >= -0.5
                && rect[1] >= -0.5
                && rect[0] + rect[2] <= width + 0.5
                && rect[1] + rect[3] <= 720.0 + 0.5
        };
        assert!(
            inside(panel),
            "panel stays in the window at {width}: {panel:?}"
        );
        assert!(
            inside(main),
            "conversation stays in the window at {width}: {main:?}"
        );
        // Spec §0.1 / §3.3: Inspector covers below 1050 pt (620, 960) and docks at 1280+.
        let covering = width < 1050.0 || crate::right_panel::covers(width - 16.0);
        if covering {
            // KSG's sheet mode: the panel covers the conversation column outright.
            assert!(
                panel[0] <= main[0] + 0.5 && panel[0] + panel[2] >= main[0] + main[2] - 0.5,
                "panel covers the conversation at {width}: {panel:?} vs {main:?}"
            );
        } else {
            assert!(
                panel[0] >= main[0] + main[2] - 0.5,
                "panel and conversation do not overlap at {width}: {panel:?} vs {main:?}"
            );
            assert!(rt.presented("composer-input").is_some(), "{width}");
        }

        // The Context segment switches to the document body.
        press(&mut rt, "right-panel-segment-context");
        rt.frame(0.5);
        let key = rt.state.panel_key();
        assert_eq!(
            rt.state.right_panels.get(&key).view,
            crate::right_panel::PanelView::Context,
            "{width}"
        );
        assert!(
            rt.presented("right-panel-body-captain-doc:0:NOTES.md")
                .is_some(),
            "{width}"
        );

        // The Diff segment switches back.
        press(&mut rt, "right-panel-segment-diff");
        rt.frame(0.5);
        assert_eq!(
            rt.state.right_panels.get(&key).view,
            crate::right_panel::PanelView::Diff,
            "{width}"
        );

        // Close drops the viewer first, then ends the panel and restores the conversation's full width.
        press(&mut rt, "right-panel-close");
        rt.frame(0.5);
        assert!(rt.state.right_panels.get(&key).viewer.is_none(), "{width}");
        assert!(rt.presented("right-panel").is_some(), "{width}");
        press(&mut rt, "right-panel-close");
        rt.frame(0.5);
        assert!(rt.presented("right-panel").is_none(), "{width}");
        let (restored, _) = rt.presented("chat-main").unwrap();
        if covering {
            assert!(
                (restored[2] - main[2]).abs() <= 0.5,
                "{width}: {restored:?}"
            );
        } else {
            assert!(restored[2] > main[2] + 100.0, "{width}: {restored:?}");
        }
    }
}

/// A patch parsed into loaded `FileEntry`s, the way `preview_done` leaves a
/// non-deferred source.
fn parsed_ready(patch: &str) -> diff_panel::Done {
    let Some(crate::review_patch::RenderablePatch::Files(files)) =
        crate::review_patch::parse(patch)
    else {
        panic!("test patch parses");
    };
    let stat = crate::review_patch::line_stat(&files);
    let files = crate::review_tree::order_like_tree(&files);
    diff_panel::Done::Ready(Box::new(diff_panel::Ready {
        files: files
            .into_iter()
            .map(|file| diff_panel::FileEntry {
                additions: file.additions,
                deletions: file.deletions,
                file,
                body: diff_panel::FileBody::Loaded,
                truncated: false,
            })
            .collect(),
        raw: None,
        additions: stat.additions,
        deletions: stat.deletions,
        truncated: false,
        incomplete: false,
        cwd: "/repo".into(),
        source_kind: crate::review::ReviewSourceKind::BranchRange,
        base_ref: None,
        rows: vec![],
        rows_version: 0,
        tree_rows: vec![],
        tree_version: 0,
    }))
}

#[test]
fn diff_panel_lists_files_collapses_and_virtualizes_at_all_widths() {
    use crate::right_panel::SingletonKind;
    // Two files: +2 -1 on src/a.rs, +3 on src/b.rs — "+5 -1" total.
    const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n\
        --- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n keep\n-old\n+new\n+more\n tail\n\
        diff --git a/src/b.rs b/src/b.rs\n\
        --- a/src/b.rs\n+++ b/src/b.rs\n@@ -5,0 +5,3 @@\n+one\n+two\n+three\n";
    for width in [960.0, 1280.0, 1920.0] {
        let mut rt = runtime(width);
        let key = rt.state.panel_key();
        rt.state.right_panels.open(&key, SingletonKind::Diff);
        // `last_refresh_at` is fresh so the mount does not read git.
        rt.state.diffs.insert(
            key.clone(),
            diff_panel::ThreadDiff {
                done: Some(parsed_ready(PATCH)),
                last_refresh_at: Some(std::time::Instant::now()),
                ..Default::default()
            },
        );
        rt.state.diff_rebuild_rows(&key);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.5);

        let (panel, _) = rt
            .presented("right-panel")
            .unwrap_or_else(|| panic!("panel mounts at {width}"));
        for file_key in ["diff-file-0", "diff-file-1"] {
            let (rect, _) = rt
                .presented(file_key)
                .unwrap_or_else(|| panic!("{file_key} at {width}"));
            assert!(
                rect[0] >= panel[0] - 0.5
                    && rect[1] >= panel[1] - 0.5
                    && rect[0] + rect[2] <= panel[0] + panel[2] + 0.5
                    && rect[1] + rect[3] <= panel[1] + panel[3] + 0.5,
                "{file_key} sits inside the panel at {width}: {rect:?} vs {panel:?}"
            );
        }
        // The header total reads KSG's `+N -N` with its group label.
        let stat = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("diff-stat"))
            .unwrap_or_else(|| panic!("diff-stat at {width}"));
        assert_eq!(
            stat.label.as_deref(),
            Some("5 additions, 1 deletions"),
            "{width}"
        );
        assert!(rt.presented("diff-line-0-0-0").is_some(), "{width}");
        assert!(rt.presented("diff-line-1-0-0").is_some(), "{width}");

        // Folding one file hides only its rows; collapse-all hides the rest.
        press(&mut rt, "diff-file-toggle-0");
        rt.frame(0.5);
        assert!(rt.presented("diff-line-0-0-0").is_none(), "{width}");
        assert!(rt.presented("diff-line-1-0-0").is_some(), "{width}");
        press(&mut rt, "diff-collapse-all");
        rt.frame(0.5);
        assert!(rt.presented("diff-line-1-0-0").is_none(), "{width}");

        // A 20,000-line diff still builds only the visible rows.
        let mut big = crate::review_patch::FileDiff {
            path: "big.rs".into(),
            previous_path: None,
            change: crate::review_patch::ChangeKind::Modified,
            binary: false,
            old_mode: None,
            new_mode: None,
            hunks: vec![],
            additions: 20_000,
            deletions: 0,
        };
        big.hunks.push(crate::review_patch::Hunk {
            old_start: 1,
            old_count: 0,
            new_start: 1,
            new_count: 20_000,
            section: String::new(),
            lines: (1..=20_000)
                .map(|n| crate::review_patch::Line {
                    kind: crate::review_patch::LineKind::Added,
                    text: format!("line {n}"),
                    old_number: None,
                    new_number: Some(n),
                    no_newline: false,
                })
                .collect(),
        });
        rt.state.diffs.insert(
            key.clone(),
            diff_panel::ThreadDiff {
                done: Some(diff_panel::Done::Ready(Box::new(diff_panel::Ready {
                    files: vec![diff_panel::FileEntry {
                        additions: big.additions,
                        deletions: big.deletions,
                        file: big,
                        body: diff_panel::FileBody::Loaded,
                        truncated: false,
                    }],
                    raw: None,
                    additions: 20_000,
                    deletions: 0,
                    truncated: false,
                    incomplete: false,
                    cwd: "/repo".into(),
                    source_kind: crate::review::ReviewSourceKind::BranchRange,
                    base_ref: None,
                    rows: vec![],
                    rows_version: 0,
                    tree_rows: vec![],
                    tree_version: 0,
                }))),
                last_refresh_at: Some(std::time::Instant::now()),
                ..Default::default()
            },
        );
        rt.state.diff_rebuild_rows(&key);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.5);
        let mounted = rt
            .elements()
            .into_iter()
            .filter(|e| {
                e.key
                    .as_deref()
                    .is_some_and(|key| key.starts_with("diff-line-"))
            })
            .count();
        assert!(
            mounted < 400,
            "{width}: {mounted} line rows mounted of 20,000"
        );
    }
}

/// Six files across three folder levels — the spec's tree fixture. Each
/// file gets fifteen added lines so the diff outgrows the 600 pt viewport
/// and the reveal has somewhere to scroll.
const TREE_PATCH: &str = "diff --git a/a.txt b/a.txt\n\
    --- a/a.txt\n+++ b/a.txt\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
    diff --git a/src/lib.rs b/src/lib.rs\n\
    --- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
    diff --git a/src/ui/app.rs b/src/ui/app.rs\n\
    --- a/src/ui/app.rs\n+++ b/src/ui/app.rs\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
    diff --git a/src/ui/widgets/list.rs b/src/ui/widgets/list.rs\n\
    --- a/src/ui/widgets/list.rs\n+++ b/src/ui/widgets/list.rs\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
    diff --git a/tests/app.test.ts b/tests/app.test.ts\n\
    --- a/tests/app.test.ts\n+++ b/tests/app.test.ts\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n\
    diff --git a/src/ui/widgets/deep/item.rs b/src/ui/widgets/deep/item.rs\n\
    --- a/src/ui/widgets/deep/item.rs\n+++ b/src/ui/widgets/deep/item.rs\n@@ -1,0 +1,15 @@\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n+l\n";

/// KSG `FileExplorerPane` beside the diff: the tree column sits inside the
/// panel without overlapping the diff list, pressing a file reveals its
/// header, a folder press folds, the toggle unmounts it, and a 5,000-file
/// diff still builds only the rows on screen.
#[test]
fn diff_tree_lays_out_reveals_and_virtualizes_at_all_widths() {
    use crate::right_panel::SingletonKind;
    // A docked panel resized to its 360 px minimum leaves a lane under the
    // tree's 416 px minimum, so the tree and its toggle hide outright
    // (KSG's implied threshold).
    let mut rt = runtime(1280.0);
    rt.state.right_panel_width = crate::right_panel::MIN_WIDTH;
    let key = rt.state.panel_key();
    rt.state.right_panels.open(&key, SingletonKind::Diff);
    rt.state.diffs.insert(
        key.clone(),
        diff_panel::ThreadDiff {
            done: Some(parsed_ready(TREE_PATCH)),
            last_refresh_at: Some(std::time::Instant::now()),
            ..Default::default()
        },
    );
    rt.state.diff_rebuild_rows(&key);
    rt.state.diff_tree_open = true;
    rt.invalidate();
    rt.frame(0.0);
    rt.frame(0.5);
    rt.frame(0.5);
    assert!(rt.presented("diff-rows").is_some());
    assert!(
        rt.presented("diff-tree").is_none(),
        "tree hides on a 360 panel"
    );
    assert!(
        rt.presented("diff-tree-toggle").is_none(),
        "toggle hides on a 360 panel"
    );

    for width in [1280.0, 1920.0] {
        let mut rt = runtime(width);
        let key = rt.state.panel_key();
        rt.state.right_panels.open(&key, SingletonKind::Diff);
        rt.state.diffs.insert(
            key.clone(),
            diff_panel::ThreadDiff {
                done: Some(parsed_ready(TREE_PATCH)),
                last_refresh_at: Some(std::time::Instant::now()),
                ..Default::default()
            },
        );
        rt.state.diff_rebuild_rows(&key);
        rt.state.diff_tree_open = true;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.5);
        rt.frame(0.5);

        let (panel, _) = rt
            .presented("right-panel")
            .unwrap_or_else(|| panic!("panel mounts at {width}"));
        let (tree, _) = rt
            .presented("diff-tree")
            .unwrap_or_else(|| panic!("tree mounts at {width}"));
        let (rows, _) = rt
            .presented("diff-rows")
            .unwrap_or_else(|| panic!("list mounts at {width}"));
        // The tree sits inside the panel's right side, beside the list.
        assert!(
            tree[0] >= panel[0] - 0.5
                && tree[0] + tree[2] <= panel[0] + panel[2] + 0.5
                && tree[1] >= panel[1] - 0.5
                && tree[1] + tree[3] <= panel[1] + panel[3] + 0.5,
            "{width}: tree {tree:?} inside panel {panel:?}"
        );
        assert!(
            tree[0] >= rows[0] + rows[2] - 0.5,
            "{width}: tree {tree:?} beside list {rows:?}"
        );
        // KSG's bounds: the lane holds 160..=560 minus the diff reserve.
        assert!(
            (crate::ui::diff_tree::MIN_W..=crate::ui::diff_tree::MAX_W).contains(&tree[2]),
            "{width}: tree width {}",
            tree[2]
        );

        // The toggle rides the header while the lane fits.
        assert!(rt.presented("diff-tree-toggle").is_some(), "{width}");
        // File and folder rows mount; the tree's file-3 is the diff's
        // file-3 (src/ui/widgets/list.rs).
        assert!(rt.presented("diff-tree-file-3").is_some(), "{width}");
        assert!(rt.presented("diff-tree-dir-src/").is_some(), "{width}");

        // Pressing the file reveals its diff header at the list's top.
        press(&mut rt, "diff-tree-file-3");
        rt.frame(0.5);
        rt.frame(0.5);
        let (revealed, _) = rt
            .presented("diff-file-3")
            .unwrap_or_else(|| panic!("diff-file-3 at {width}"));
        let (list_top, _) = rt.presented("diff-rows").unwrap();
        assert!(
            (revealed[1] - list_top[1]).abs() <= 1.0,
            "{width}: diff-file-3 {revealed:?} tops the list {list_top:?}"
        );
        // The marked file carries the tree's current-file sync too.
        let key = rt.state.panel_key();
        assert_eq!(rt.state.diffs[&key].current_file, Some(3), "{width}");

        // Pressing a folder row removes its file rows. `order_like_tree`
        // puts the deep item at file 4 and `tests/app.test.ts` at file 5,
        // so folding `src/` leaves the test file as `diff-tree-file-5`.
        press(&mut rt, "diff-tree-dir-src/");
        rt.frame(0.5);
        assert!(rt.presented("diff-tree-file-1").is_none(), "{width}");
        assert!(rt.presented("diff-tree-file-3").is_none(), "{width}");
        assert!(rt.presented("diff-tree-file-4").is_none(), "{width}");
        assert!(rt.presented("diff-tree-file-5").is_some(), "{width}");

        // The toggle hides the tree and the diff list takes the width.
        press(&mut rt, "diff-tree-toggle");
        rt.frame(0.5);
        assert!(rt.presented("diff-tree").is_none(), "{width}");
        let (wide_rows, _) = rt.presented("diff-rows").unwrap();
        assert!(
            wide_rows[2] > rows[2] + 100.0,
            "{width}: {wide_rows:?} vs {rows:?}"
        );
    }

    // 5,000 changed files still build only the visible tree rows.
    let mut rt = runtime(1280.0);
    let key = rt.state.panel_key();
    rt.state.right_panels.open(&key, SingletonKind::Diff);
    let patch: String = (0..5_000)
        .map(|i| {
            format!(
                "diff --git a/dir{}/sub/f{}.rs b/dir{}/sub/f{}.rs\n\
                --- a/dir{}/sub/f{}.rs\n+++ b/dir{}/sub/f{}.rs\n@@ -1,0 +1,1 @@\n+x\n",
                i % 50,
                i,
                i % 50,
                i,
                i % 50,
                i,
                i % 50,
                i
            )
        })
        .collect();
    rt.state.diffs.insert(
        key.clone(),
        diff_panel::ThreadDiff {
            done: Some(parsed_ready(&patch)),
            last_refresh_at: Some(std::time::Instant::now()),
            ..Default::default()
        },
    );
    rt.state.diff_rebuild_rows(&key);
    rt.state.diff_tree_open = true;
    rt.invalidate();
    rt.frame(0.0);
    rt.frame(0.5);
    rt.frame(0.5);
    let mounted = rt
        .elements()
        .into_iter()
        .filter(|e| {
            e.key.as_deref().is_some_and(|key| {
                key.starts_with("diff-tree-file-") || key.starts_with("diff-tree-dir-")
            })
        })
        .count();
    assert!(mounted < 200, "5,000 files: {mounted} tree rows built");
}

/// KSG `PanelLayoutControls` in the top bar: the diff toggle opens and
/// closes the Diff tab, the panel toggle the whole panel, and the loaded
/// diff's totals sit beside the diff toggle once the row is wide enough.
#[test]
fn top_bar_diff_and_panel_buttons_toggle_the_right_panel_at_all_widths() {
    // Two files: +2 -1 on src/a.rs, +3 on src/b.rs — "+5 -1" total.
    const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n\
        --- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n keep\n-old\n+new\n+more\n tail\n\
        diff --git a/src/b.rs b/src/b.rs\n\
        --- a/src/b.rs\n+++ b/src/b.rs\n@@ -5,0 +5,3 @@\n+one\n+two\n+three\n";
    for width in [1280.0] {
        let mut rt = runtime(width);
        // A loaded diff, fresh enough that mounting the tab does not re-read git.
        let key = rt.state.panel_key();
        rt.state.diffs.insert(
            key.clone(),
            diff_panel::ThreadDiff {
                done: Some(parsed_ready(PATCH)),
                last_refresh_at: Some(std::time::Instant::now()),
                ..Default::default()
            },
        );
        rt.state.diff_rebuild_rows(&key);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.5);

        let (bar, _) = rt.presented("top-bar").unwrap();
        // Every presented right-side control stays inside the bar and
        // clear of its neighbours and the status text's slot.
        let mut controls = Vec::new();
        for key in [
            "chat-status",
            "thread-actions",
            "header-diff-stat",
            "header-diff",
            "header-right-panel",
        ] {
            if let Some((rect, _)) = rt.presented(key) {
                controls.push((key, rect));
            }
        }
        for (name, rect) in &controls {
            assert!(
                rect[0] >= bar[0] - 0.5 && rect[0] + rect[2] <= bar[0] + bar[2] + 0.5,
                "{width}: {name} {rect:?} escapes bar {bar:?}"
            );
        }
        for (i, (a, ar)) in controls.iter().enumerate() {
            for (b, br) in &controls[i + 1..] {
                let overlap = (ar[0] + ar[2]).min(br[0] + br[2]) - ar[0].max(br[0]);
                assert!(overlap <= 0.5, "{width}: {a} {ar:?} overlaps {b} {br:?}");
            }
        }

        // At compact widths the diff toggle drops; Diff is a tab in the
        // Inspector there. Wider, it opens the panel on the Diff tab and
        // presses off again.
        if width >= 760.0 {
            assert!(rt.presented("header-diff").is_some(), "{width}");
            press(&mut rt, "header-diff");
            rt.frame(0.5);
            let key = rt.state.panel_key();
            assert_eq!(
                rt.state.right_panels.active_view(&key),
                Some(crate::right_panel::PanelView::Diff),
                "{width}"
            );
            assert!(rt.presented("right-panel").is_some(), "{width}");
            press(&mut rt, "header-diff");
            rt.frame(0.5);
            assert!(
                rt.presented("right-panel").is_none(),
                "{width}: a second press closes the panel"
            );
        } else {
            assert!(rt.presented("header-diff").is_none(), "{width}");
        }

        // The panel toggle shows and hides the whole panel at every width.
        press(&mut rt, "header-right-panel");
        rt.frame(0.5);
        assert!(rt.presented("right-panel").is_some(), "{width}");
        press(&mut rt, "header-right-panel");
        rt.frame(0.5);
        assert!(rt.presented("right-panel").is_none(), "{width}");

        // The stat reads the loaded diff's totals at and above the narrow fold.
        assert_eq!(
            rt.presented("header-diff-stat").is_some(),
            width >= 1050.0,
            "{width}: the stat only shows once the row is wide enough"
        );
        if width >= 1050.0 {
            let stat = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some("header-diff-stat"))
                .unwrap();
            assert_eq!(
                stat.label.as_deref(),
                Some("5 additions, 1 deletions"),
                "{width}"
            );
        }
    }
}

#[test]
fn questionnaire_replaces_composer_and_supports_keyboard_and_previous_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.on_key = Some(crate::key);
        rt.state.current_mut().draft.replace_text("Preserved draft");
        rt.state.current_mut().apply(json!({"type":"extension_ui_request","id":"form","method":"input",
            "questions":[
                {"id":"one","header":"Choose storage","question":"Where should the working data go?",
                 "multiSelect":true,"options":[{"label":"Disk","description":"Keep it on this computer"},{"label":"Server","description":"Use a remote server"}]},
                {"id":"two","header":"Details","question":"Anything else?","options":[]}
            ]}));
        rt.frame(0.0);
        for key in [
            "questionnaire-drawer",
            "dialog-option-0",
            "dialog-option-1",
            "dialog-reply",
            "questionnaire-next",
        ] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 1.0,
                "{key}: {rect:?}"
            );
            assert!(
                rect[1] >= 0.0 && rect[1] + rect[3] <= 721.0,
                "{key}: {rect:?}"
            );
        }
        assert!(rt.find("composer-input").is_none());
        assert!(rt.find("send-message").is_none());
        // Digit shortcuts work before any drawer control has focus.
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("1".into()),
            mods: Default::default(),
        });
        rt.frame(0.0);
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("2".into()),
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert_eq!(
            rt.state
                .current()
                .native
                .questionnaire
                .as_ref()
                .unwrap()
                .answers[0]
                .selected,
            vec!["Disk", "Server"]
        );
        press(&mut rt, "questionnaire-next");
        assert_eq!(
            rt.state
                .current()
                .native
                .questionnaire
                .as_ref()
                .unwrap()
                .index,
            1
        );
        assert!(rt.find("dialog-submit").is_some());
        rt.state.current_mut().reply.replace_text("Free text");
        press(&mut rt, "questionnaire-previous");
        assert_eq!(
            rt.state
                .current()
                .native
                .questionnaire
                .as_ref()
                .unwrap()
                .index,
            0
        );
        press(&mut rt, "questionnaire-next");
        assert_eq!(rt.state.current().reply.text, "Free text");
        assert_eq!(rt.state.current().draft.text, "Preserved draft");
        rt.state.current_mut().reply.focused = true;
        assert!(!pi_ui::questionnaire_key(
            &mut rt.state,
            &rune::KeyEvent {
                key: rune::Key::Text("1".into()),
                mods: Default::default()
            }
        ));
    }
}

#[test]
fn composer_file_pill_opens_a_transient_preview_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.frame(0.0);
        let file = rt.state.data_dir.join(format!("mention-{width}.txt"));
        std::fs::write(&file, "mention preview body").unwrap();
        let token = crate::composer_tokens::Token::File {
            path: file,
            directory: false,
            source: None,
        };
        let id = token.id();
        rt.state
            .current_mut()
            .draft
            .replace_with_pill(0..0, id.clone(), "mention.txt");
        rt.invalidate();
        rt.frame(0.0);
        // The headless runtime cannot point at a pill inside a text field,
        // so press through the same handler `on_input_pill_press` calls.
        rt.update(|s, _| s.press_composer_pill(&id));
        rt.frame(0.0);
        let (dialog, _) = rt
            .presented("attachment-preview")
            .unwrap_or_else(|| panic!("{width}"));
        assert!(
            dialog[0] >= 0.0 && dialog[0] + dialog[2] <= width + 0.5,
            "{width}: {dialog:?}"
        );
        let preview = rt.state.attachment_preview.as_ref().unwrap();
        assert!(preview.transient && preview.error.is_none());
        assert!(rt.state.current().draft_images.is_empty());
        let draft = rt.state.current().draft.text.clone();
        press(&mut rt, "close-attachment-preview");
        assert!(rt.state.attachment_preview.is_none());
        assert_eq!(rt.state.current().draft.text, draft);
    }
}

#[test]
fn attachment_previews_open_from_tray_and_messages_and_fit_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        let directory = rt.state.data_dir.clone();
        let file_path = directory.join("preview.txt");
        std::fs::write(&file_path, "Preview body").unwrap();
        let file = crate::attachments::attach_file(&file_path).unwrap();
        let movie_path = directory.join("preview.mp4");
        std::fs::write(
            &movie_path,
            include_bytes!("../../assets/tests/attachment-preview.mp4"),
        )
        .unwrap();
        let video = crate::attachments::attach_file(&movie_path).unwrap();
        let image_path = directory.join("preview.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([60, 80, 100, 255]))
            .save(&image_path)
            .unwrap();
        let image = crate::attachments::attach_file(&image_path).unwrap();
        rt.state.current_mut().draft_images = vec![image.clone(), file.clone(), video.clone()];
        rt.state.current_mut().draft.replace_text("Inspect");
        rt.state.current_mut().draft.focused = true;
        rt.frame(0.0);
        press(&mut rt, &format!("attachment-thumb-{}", image.id));
        rt.frame(0.0);
        assert!(rt.presented("attachment-preview-image").is_some());
        press(&mut rt, "close-attachment-preview");
        for attachment in [file.clone(), image.clone(), video.clone()] {
            for draft in [true, false] {
                rt.update(|s, _| {
                    s.preview_attachments(vec![attachment.clone()], &attachment.id, draft)
                });
                rt.frame(0.0);
                let (bounds, _) = rt.presented("attachment-preview").unwrap();
                assert!(
                    bounds[0] >= 0.0 && bounds[0] + bounds[2] <= width + 0.5,
                    "{width}: {bounds:?}"
                );
                assert!(
                    bounds[1] >= 0.0 && bounds[1] + bounds[3] <= rt.state.viewport_height + 0.5
                );
                if attachment.id == video.id {
                    assert!(rt.presented("attachment-preview-video").is_some());
                    assert!(rt.presented("attachment-preview-seek").is_some());
                    assert!(rt.presented("attachment-preview-media-controls").is_some());
                    let player = rt
                        .state
                        .attachment_preview
                        .as_ref()
                        .unwrap()
                        .player()
                        .unwrap()
                        .clone();
                    // Rune has no video playback on Linux yet, so only the
                    // layout is checked there.
                    if cfg!(any(target_os = "macos", windows)) {
                        rt.frame(0.25);
                        assert!(
                            !matches!(player.state(), rune::PlayerState::Failed(_)),
                            "{:?}",
                            player.state()
                        );
                        assert!(player.duration().is_some_and(|duration| duration > 0.0));
                        rt.frame(0.25);
                        assert!(player.time() > 0.0);
                    }
                } else if attachment.id == file.id {
                    assert!(
                        rt.elements()
                            .iter()
                            .any(|e| e.text.as_deref() == Some("Preview body")),
                        "{width} draft {draft}: {:?}",
                        rt.elements()
                            .iter()
                            .filter(|e| e.path.contains("attachment-preview"))
                            .map(|e| (&e.key, &e.text, e.rect))
                            .collect::<Vec<_>>()
                    );
                }
                press(&mut rt, "close-attachment-preview");
                assert!(rt.state.attachment_preview.is_none());
            }
        }
    }
}

#[test]
fn question_collapse_reopens_preserves_drafts_and_blocks_hidden_shortcuts_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state
            .current_mut()
            .apply(json!({"type":"extension_ui_request","id":"collapse",
            "method":"input","responseCapability":{"type":"message"},"questions":[
                {"id":"one","header":"Storage","question":"Choose **working storage**.",
                    "multiSelect":true,"options":[{"label":"Disk"}]},
                {"id":"two","header":"Details","question":"Any details?","options":[]}
            ]}));
        rt.state.current_mut().reply.replace_text("Unsent answer");
        rt.frame(0.0);
        let expanded = rt.presented("questionnaire-drawer").unwrap().0;
        press(&mut rt, "questionnaire-toggle");
        assert!(rt.state.current().questionnaire_collapsed(), "{width}");
        rt.frame(0.25);
        assert!(rt.find("questionnaire-markdown").is_none());
        assert!(rt.find("dialog-option-0").is_none());
        assert!(rt.presented("questionnaire-summary").is_some());
        assert!(rt.presented("questionnaire-count").is_some());
        assert!(rt.presented("dialog-reply").is_some());
        let collapsed = rt.presented("questionnaire-drawer").unwrap().0;
        assert!(
            collapsed[3] < expanded[3],
            "{width}: {collapsed:?} {expanded:?}"
        );
        for key in [
            "questionnaire-toggle",
            "questionnaire-summary",
            "dialog-cancel",
        ] {
            let rect = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"))
                .0;
            assert!(rect[0] >= 0.0 && rect[0] + rect[2] <= width + 0.5);
            assert!(rect[1] >= 0.0 && rect[1] + rect[3] <= 720.5);
        }
        assert!(!pi_ui::questionnaire_key(
            &mut rt.state,
            &rune::KeyEvent {
                key: rune::Key::Text("1".into()),
                mods: Default::default()
            }
        ));
        press(&mut rt, "questionnaire-toggle");
        rt.frame(0.25);
        assert!(rt.presented("dialog-option-0").is_some());
        assert_eq!(rt.state.current().reply.text, "Unsent answer");
        press(&mut rt, "questionnaire-toggle");
        press(&mut rt, "questionnaire-next");
        rt.frame(0.25);
        assert!(rt.presented("questionnaire-markdown").is_some());
        assert!(rt.find("questionnaire-summary").is_none());
        rt.state.current_mut().respond(json!({"cancelled":true}));
        rt.state
            .current_mut()
            .apply(json!({"type":"extension_ui_request","id":"new-request",
            "method":"input","questions":[{"id":"one","question":"New request?","options":[]}]}));
        rt.invalidate();
        rt.frame(0.0);
        assert!(rt.presented("questionnaire-markdown").is_some());
        assert!(rt.find("questionnaire-count").is_none());
        assert!(
            rt.find("dialog-cancel").is_none(),
            "live questions cannot dismiss"
        );
    }
}

#[test]
fn harness_picker_is_available_before_first_send_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.state.harnesses.agents = vec![crate::harness::Agent {
            id: "fixture".into(),
            name: "Fixture ACP".into(),
            command: "fixture-agent".into(),
            args: vec![],
            env: vec![],
        }];
        rt.state.settings_picker = Some(SettingsPicker::Model);
        rt.frame(0.0);
        for key in [
            "harness-group",
            "choose-harness-pi",
            "choose-harness-codex",
            "choose-harness-claude",
            "choose-harness-fixture",
            "harness-tag-fixture",
            "model-list",
            "model-search",
        ] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 1.0,
                "{key}: {rect:?}"
            );
        }
        assert!(
            rt.presented("harness-tag-pi").is_none(),
            "Pi is built in, not an ACP agent"
        );
        assert!(rt.presented("harness-tag-codex").is_none());
        assert!(rt.presented("harness-tag-claude").is_none());
        assert!(
            find(&rt.accessibility_json(), "choose-harness-claude")
                .and_then(|node| node["states"].as_array().cloned())
                .is_some_and(|states| states.iter().any(|x| x == "disabled")),
            "{width}: claude should be disabled when missing"
        );
        rt.state.model_picker_tab = Some(crate::model_picker::ModelPickerTab::Instance(
            "claude".into(),
        ));
        rt.invalidate();
        rt.frame(0.0);
        assert!(rt.presented("claude-unavailable").is_some());
        rt.state.claude_catalog.installed = true;
        rt.state.model_picker_tab = None;
        rt.invalidate();
        rt.frame(0.0);
        press(&mut rt, "choose-harness-claude");
        assert_eq!(
            rt.state.model_picker_tab,
            Some(crate::model_picker::ModelPickerTab::Instance(
                "claude".into()
            ))
        );
        press(&mut rt, "choose-harness-fixture");
        assert_eq!(
            rt.state.model_picker_tab,
            Some(crate::model_picker::ModelPickerTab::Instance(
                "fixture".into()
            ))
        );
        press(&mut rt, "choose-model-0");
        assert_eq!(rt.state.current().harness, "fixture");
        rt.state.current_mut().life.latest_user_message_at = Some(1);
        rt.state.open_model_picker(SettingsPicker::Model);
        rt.invalidate();
        rt.frame(0.0);
        press(&mut rt, "choose-harness-pi");
        assert_eq!(
            rt.state.current().harness,
            "fixture",
            "switching the tab does not move the thread"
        );
        assert_eq!(
            rt.state.model_picker_tab,
            Some(crate::model_picker::ModelPickerTab::Instance("pi".into())),
            "{width}: pi stays reachable on a started thread via context handoff"
        );
    }
}

#[test]
fn queued_edit_controls_save_cancel_and_shortcuts_fit_at_supported_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        let (pi, _, _) = crate::backend::Pi::test_connection();
        let t = rt.state.current_mut();
        t.pi = Some(pi);
        t.model_provider = "cpa".into();
        t.status = Status::Running;
        t.native.follow_up = true;
        t.draft.replace_text("Queued original");
        t.send_during_run();
        let id = t.native.queue[0].id;
        t.draft.replace_text("Ordinary draft");
        t.draft.set_caret(0);
        assert!(pi_ui::composer_pi_key(
            &mut rt.state,
            &rune::KeyEvent {
                key: rune::Key::Up,
                mods: rune::Mods {
                    alt: true,
                    ..Default::default()
                },
            },
            &rune::Tx::default(),
            None
        ));
        assert_eq!(rt.state.current().draft.text, "Queued original");
        rt.frame(0.0);
        for key in [
            "composer",
            "queued-messages",
            "send-message",
            &format!("queue-edit-{id}"),
        ] {
            let (rect, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(rect[0] >= 0.0 && rect[0] + rect[2] <= width + 1.0);
        }
        rt.state.current_mut().draft.replace_text("Edited queued");
        rt.state.send();
        assert_eq!(
            rt.state.current().native.queue[0].prompt.text,
            "Edited queued"
        );
        assert_eq!(rt.state.current().draft.text, "Ordinary draft");
        rt.state.current_mut().edit_queued(id);
        rt.state.current_mut().draft.replace_text("Cancel me");
        rt.state.current_mut().cancel_queued_edit();
        assert_eq!(rt.state.current().draft.text, "Ordinary draft");
        assert!(pi_ui::composer_pi_key(
            &mut rt.state,
            &rune::KeyEvent {
                key: rune::Key::Enter,
                mods: rune::Mods {
                    cmd: true,
                    ctrl: cfg!(not(target_os = "macos")),
                    shift: true,
                    ..Default::default()
                },
            },
            &rune::Tx::default(),
            None
        ));
        assert!(rt.state.current().native.queue.is_empty());
    }
}

#[test]
fn codex_compact_button_and_progress_rows_fit_at_supported_widths() {
    // Uses the existing KSG ContextWindowMeter and context-compaction row layout.
    for width in [1280.0] {
        let mut rt = runtime(width);
        let (pi, commands, _) = crate::backend::Pi::test_connection();
        let t = rt.state.current_mut();
        t.pi = Some(pi);
        t.harness = "codex".into();
        t.session = Some("codex-fixture-thread".into());
        t.model_provider = "codex".into();
        t.context_window = Some(200000);
        t.context
            .absorb(&json!({"contextUsage":{"tokens":100000,"contextWindow":200000}}));
        t.context.details_open = true;
        rt.frame(0.0);
        press(&mut rt, "compact-context");
        assert!(commands.try_iter().any(|r| r["type"] == "compact"));
        rt.state
            .current_mut()
            .apply(json!({"type":"compaction_start"}));
        rt.state.current_mut().context.details_open = true;
        rt.invalidate();
        rt.frame(0.0);
        for key in ["compact-context", "compaction-0"] {
            let (r, _) = rt
                .presented(key)
                .unwrap_or_else(|| panic!("{width}: {key}"));
            assert!(
                r[0] >= 0.0 && r[0] + r[2] <= width + 1.0,
                "{width}: {key} {r:?}"
            );
        }
        rt.state
            .current_mut()
            .apply(json!({"type":"compaction_end"}));
        rt.state.current_mut().native.compaction_expanded = true;
        rt.frame(0.0);
        assert!(rt.presented("compaction-0").is_some());
    }
}

#[test]
fn slash_menu_supports_keyboard_selection_and_provider_commands_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.frame(0.0);
        press(&mut rt, "composer-input");
        rt.state.current_mut().apply(json!({"type":"response","command":"get_commands","success":true,"data":{"commands":[{"name":"skill:review","description":"Review the patch","source":"skill"}]}}));
        rt.state.current_mut().draft.replace_text("/skill:");
        rt.invalidate();
        rt.frame(0.0);
        let (r, _) = rt.presented("slash-menu").unwrap();
        assert!(r[0] >= 0.0 && r[0] + r[2] <= width + 1.0);
        rt.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert_eq!(rt.state.current().draft.text, "/skill:review ");
        assert!(rt.find("slash-menu").is_none());
    }
}

#[test]
fn inline_slash_selection_preserves_the_rest_of_the_draft() {
    let mut rt = runtime(960.0);
    rt.frame(0.0);
    press(&mut rt, "composer-input");
    let t = rt.state.current_mut();
    t.apply(
        json!({"type":"response","command":"get_commands","success":true,
        "data":{"commands":[{"name":"skill:review","source":"skill"}]}}),
    );
    t.draft.replace_text("please /skill:re after");
    t.draft.set_caret(16);
    rt.frame(0.0);
    rt.key(rune::KeyEvent {
        key: rune::Key::Tab,
        mods: Default::default(),
    });
    rt.frame(0.0);
    assert_eq!(rt.state.current().draft.text, "please /skill:review  after");
    rt.state
        .current_mut()
        .draft
        .replace_text("please /does-not-exist");
    rt.invalidate();
    rt.frame(0.0);
    assert!(rt.find("slash-menu").is_some());
    rt.key(rune::KeyEvent {
        key: rune::Key::Escape,
        mods: Default::default(),
    });
    rt.frame(0.0);
    assert!(rt.find("slash-menu").is_none());
}

#[test]
fn skill_menu_uses_the_same_rows_and_inserts_a_native_skill_pill_at_all_widths() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        rt.frame(0.0);
        press(&mut rt, "composer-input");
        rt.state.current_mut().apply(json!({"type":"response","command":"get_commands","success":true,
            "data":{"commands":[{"name":"skill:review","description":"Review changes","source":"skill"}]}}));
        rt.key(rune::KeyEvent {
            key: rune::Key::Text("please $rev".into()),
            mods: Default::default(),
        });
        rt.frame(0.0);
        rt.frame(0.25);
        let (menu, _) = rt.presented("skill-menu").unwrap();
        let (item, _) = rt.presented("skill-item-0").unwrap();
        assert_eq!(item[3], 32.0);
        assert!(
            menu[0] >= 8.0 && menu[0] + menu[2] <= width - 7.0,
            "{width}: {menu:?}"
        );
        rt.key(rune::KeyEvent {
            key: rune::Key::Enter,
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert_eq!(rt.state.current().draft.pills().count(), 1);
        assert_eq!(
            crate::composer_rich::markdown(&rt.state.current().draft),
            "please $review "
        );
        assert!(rt.find("skill-menu").is_none());
    }
}

#[test]
fn heading_task_rule_and_fence_render_inside_the_composer_card_at_620_and_1280() {
    let source = "# Title\n- [x] Done\n---\n```rs\nlet x = 1;\n```";
    for width in [620.0, 1280.0] {
        let mut rt = runtime(width);
        {
            let t = rt.state.current_mut();
            t.draft = crate::composer_rich::draft("");
            crate::composer_rich::paste_with_path_query(&mut t.draft, source, false, &|_| None);
            t.draft.focused = true;
        }
        rt.frame(0.0);
        let (card, _) = rt.presented("composer-natural").unwrap();
        let (input, _) = rt.presented("composer-input").unwrap();
        assert!(
            input[0] >= card[0]
                && input[0] + input[2] <= card[0] + card[2] + 1.0
                && input[1] >= card[1]
                && input[1] + input[3] <= card[1] + card[3] + 1.0,
            "{width}: input {input:?} outside card {card:?}"
        );
        assert!(
            input[3] > 60.0,
            "{width}: expected multiline height, got {input:?}"
        );
        let draft = &rt.state.current().draft;
        let kinds: Vec<_> = draft.logical_blocks().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                rune::BlockKind::Heading { level: 1 },
                rune::BlockKind::Task {
                    level: 0,
                    checked: true,
                },
                rune::BlockKind::Rule,
                rune::BlockKind::Code,
            ]
        );
        assert_eq!(crate::composer_rich::markdown(draft), source);
    }
}

/// KSG's ComposerCodeBlockLanguagePicker: a header trigger per code block, a
/// searchable menu that fits the window, and a language change that rewrites only
/// that block's fence.
#[test]
fn code_block_language_picker_edits_only_its_own_fence_at_all_widths() {
    let source = "```python\nfirst()\n```\n\nprose\n\n```rust\nsecond()\n```";
    let changed = "```python\nfirst()\n```\n\nprose\n\n```dart\nsecond()\n```";
    for width in [1280.0] {
        let mut rt = runtime(width);
        {
            let t = rt.state.current_mut();
            t.draft = crate::composer_rich::draft("");
            crate::composer_rich::paste_with_path_query(&mut t.draft, source, false, &|_| None);
            t.draft.focused = true;
        }
        rt.invalidate();
        rt.frame(0.0);
        let (card, _) = rt.presented("composer-natural").unwrap();
        for n in 0..2 {
            let key = format!("composer-code-language-{n}");
            let (trigger, _) = rt
                .presented(&key)
                .unwrap_or_else(|| panic!("{width}: {key} not presented"));
            assert!(
                trigger[0] >= card[0]
                    && trigger[0] + trigger[2] <= card[0] + card[2] + 1.0
                    && trigger[1] >= card[1]
                    && trigger[1] + trigger[3] <= card[1] + card[3] + 1.0,
                "{width}: {key} {trigger:?} outside card {card:?}"
            );
        }
        press(&mut rt, "composer-code-language-1");
        let (menu, _) = rt
            .presented("composer-code-language-menu")
            .unwrap_or_else(|| panic!("{width}: menu not presented"));
        assert!(
            menu[0] >= 0.0
                && menu[0] + menu[2] <= width + 1.0
                && menu[1] >= 0.0
                && menu[1] + menu[3] <= 721.0,
            "{width}: menu {menu:?} outside window"
        );
        assert!(
            rt.presented("composer-code-language-search").is_some(),
            "{width}"
        );
        press(&mut rt, "composer-code-language-option-dart");
        assert_eq!(
            crate::composer_rich::markdown(&rt.state.current().draft),
            changed,
            "{width}"
        );
        assert!(
            rt.presented("composer-code-language-menu").is_none(),
            "{width}"
        );
        assert!(rt.state.current().draft.focused, "{width}");
        // Reopen and Escape: the menu closes and the draft does not change.
        press(&mut rt, "composer-code-language-1");
        assert!(
            rt.presented("composer-code-language-menu").is_some(),
            "{width}"
        );
        rt.key(rune::KeyEvent {
            key: rune::Key::Escape,
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert!(
            rt.presented("composer-code-language-menu").is_none(),
            "{width}"
        );
        assert_eq!(
            crate::composer_rich::markdown(&rt.state.current().draft),
            changed,
            "{width}"
        );
    }
}

#[test]
fn steering_and_rewind_controls_send_native_commands_without_overflow() {
    for width in [1280.0] {
        let mut rt = runtime(width);
        let (pi, commands, _) = crate::backend::Pi::test_connection();
        let t = rt.state.current_mut();
        t.pi = Some(pi);
        t.model_provider = "cpa".into();
        t.status = Status::Running;
        t.draft.replace_text("Steer now");
        t.draft.focused = true;
        rt.frame(0.0);
        for key in ["dispatch-mode", "send-message", "stop-agent"] {
            let (r, _) = rt.presented(key).unwrap();
            assert!(r[0] + r[2] <= width + 1.0, "{key}");
        }
        press(&mut rt, "send-message");
        let send = commands.try_iter().find(|r| r["type"] == "prompt").unwrap();
        assert_eq!(send["streamingBehavior"], "steer");
        assert_eq!(rt.state.current().status, Status::Running);
        let t = rt.state.current_mut();
        t.status = Status::Ready;
        t.session = Some("source.jsonl".into());
        t.native.conversation = Some(crate::model::ConversationMode::Rewind);
        t.native.fork_points = vec![json!({"entryId":"point","text":"Repeated prompt"})];
        rt.invalidate();
        rt.frame(0.0);
        press(&mut rt, "fork-point-0");
        assert!(rt.find("rewind-confirm").is_some());
        press(&mut rt, "rewind-confirm");
        let rewind = commands.try_iter().find(|r| r["type"] == "fork").unwrap();
        assert_eq!(rewind["entryId"], "point");
    }
}
