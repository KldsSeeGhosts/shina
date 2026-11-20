//! The one 40 pt bar above the conversation. A Shina-only design (the spec's
//! §1): back and forward in `NavHistory`, the thread title as a breadcrumb
//! button, the live status, then the actions KSG's header kept at the right
//! end. The tab strip takes the title's place in the same row while
//! `tabs_enabled` is on.

use super::*;

/// Inter 13/500, the row title style the spec names `TITLE`.
pub(super) const TITLE: TextStyle = TextStyle::new(13.0, 500.0).tracking(-0.08).lh(19.0);
/// Inter 13/400, the quiet row style the spec names `ROW_QUIET`.
pub(super) const ROW_QUIET: TextStyle = TextStyle::new(13.0, 400.0).tracking(-0.08).lh(19.0);

/// The title bar, mounted as `top-strip`'s right half in `view`. Everything in
/// it is one 40 pt row: the nav cluster, the title (or the tab strip), the
/// status and the actions.
pub(super) fn bar(s: &State, p: Palette, narrow: bool, compact: bool, tabs_width: f32) -> El {
    let resolved = s.resolved_keybindings();
    let ctx = s.shortcut_context(false);
    let platform = crate::keybindings::Platform::current();
    let shortcut = |command: &str| {
        crate::keybindings::shortcut_label_for_command(&resolved, command, &ctx, platform)
    };
    let mut bar = row()
        .key("top-bar")
        .grow(1.0)
        .min_w(0.0)
        .h(36.0)
        .center_y()
        .gap(8.0)
        .pl(4.0);
    let mut nav = row().key("nav-cluster").gap(2.0).center_y();
    let mut back = icon_button("nav-back", "Back", "chevron-left", p, |s| s.nav_back())
        .disabled(s.nav_history.back().is_none());
    if let Some(shortcut) = shortcut("navigation.back") {
        back = back.description(format!("Back ({shortcut})"));
    }
    nav = nav.child(back);
    if !compact {
        let mut forward = icon_button("nav-forward", "Forward", "chevron-right", p, |s| {
            s.nav_forward()
        })
        .disabled(s.nav_history.forward().is_none());
        if let Some(shortcut) = shortcut("navigation.forward") {
            forward = forward.description(format!("Forward ({shortcut})"));
        }
        nav = nav.child(forward);
    }
    bar = bar.child(nav);
    if s.tabs_enabled {
        // The strip takes the title's place in the same row.
        bar = bar.child(header(s, p, narrow, tabs_width).grow(1.0).min_w(0.0));
    } else {
        bar = bar.child(thread_title(s, p, narrow, compact));
    }
    let t = s.current();
    let text = match (t.status, t.started) {
        (Status::Running, Some(start)) => Some(format!(
            "Working for {}",
            timer_label(start.elapsed().as_secs())
        )),
        _ => None,
    };
    // `chat-status` sits right after the title at intrinsic width. Ready draws
    // nothing, so there is no empty slot holding space.
    if !s.demo && !(text.is_none() && matches!(t.status, Status::Ready | Status::Completed)) {
        bar = bar.child(status_slot("chat-status", t.status, text, p));
    }
    bar = bar
        .child(spacer())
        .child(actions(s, p, narrow, compact, shortcut));
    bar
}

/// The thread's name as the bar's breadcrumb button: workspace (or the worker's
/// captain) as a quiet first segment, then the title in `TITLE`. Press opens
/// the quick switcher, double press the rename dialog, secondary press the
/// thread menu.
fn thread_title(s: &State, p: Palette, _narrow: bool, compact: bool) -> El {
    let t = s.current();
    let mut first: Option<El> = None;
    let mut first_label = String::new();
    // A crew worker's first segment opens its captain's home, as the spec moves
    // `open-captain` out of the parent bar. A plain child keeps
    // `breadcrumb-parent` for its parent thread.
    if let Some(captain) = t.crew_captain {
        if let Some(profile) = s.captains.get(captain) {
            let home = s.captain_home(captain);
            let name = profile.name.clone();
            first_label = name.clone();
            first = Some(
                row()
                    .key("open-captain")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label(format!("Open {name}"))
                    .focusable(true)
                    .center_y()
                    .gap(6.0)
                    .h(20.0)
                    .px(4.0)
                    .radius(6.0)
                    .hover(p.selected)
                    .press(p.selected)
                    .on_press(move |s: &mut State| {
                        if let Some(home) = home {
                            if let Some(index) = s.threads.iter().position(|t| t.id == home) {
                                s.show_thread(index);
                            } else {
                                s.open_captain(captain);
                            }
                        } else {
                            s.open_captain(captain);
                        }
                    })
                    .child(avatar(s, p, captain, 16.0, s.captain_status(captain)).a11y_hidden())
                    .child(
                        label(&name, ROW_QUIET, p.muted)
                            .key("thread-title-project")
                            .truncate()
                            .max_w(160.0),
                    ),
            );
        }
    } else if let Some(parent) = t
        .parent
        .and_then(|id| s.threads.iter().position(|t| t.id == id))
    {
        first_label = s.threads[parent].title.clone();
        first = Some(
            button("breadcrumb-parent", &first_label.clone(), p, move |s| {
                s.show_thread(parent)
            })
            .h(20.0)
            .max_w(160.0)
            .shrink()
            .min_w(0.0)
            .radius(6.0),
        );
    } else if s.is_captain_home(t) {
        // A captain home's first segment is its face and name.
        if let Some(captain) = t.captain
            && let Some(profile) = s.captains.get(captain)
        {
            first_label = profile.name.clone();
            first = Some(
                row()
                    .key("thread-title-captain")
                    .center_y()
                    .gap(6.0)
                    .shrink()
                    .min_w(0.0)
                    .child(avatar(s, p, captain, 16.0, s.captain_status(captain)).a11y_hidden())
                    .child(
                        label(&profile.name, ROW_QUIET, p.muted)
                            .key("thread-title-project")
                            .truncate()
                            .max_w(160.0),
                    ),
            );
        }
    } else if s.viewport_width >= 960.0
        && let Some(project) = s.project(t.project)
    {
        // Below 960 pt of window width the project segment hides.
        first_label = project.name.clone();
        first = Some(
            label(&project.name, ROW_QUIET, p.muted)
                .key("thread-title-project")
                .truncate()
                .max_w(160.0)
                .shrink(),
        );
    }
    // With several machines the open thread's machine names itself at the
    // title's end, where the old `chat-machine` label used to sit.
    let machine = if multiple_machines(s) {
        Some(s.machine_label(&s.machines.selected))
    } else {
        None
    };
    let mut title = row()
        .key("thread-title")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label({
            let whole = if first_label.is_empty() {
                t.title.clone()
            } else {
                format!("{first_label} / {}", t.title)
            };
            format!("{whole}. Switch thread")
        })
        .focusable(true)
        .h(28.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(8.0)
        .min_w(0.0)
        .shrink()
        .max_w(if compact { f32::MAX } else { 520.0 })
        .hover(p.selected)
        .press(p.selected)
        .hover_group()
        .on_press(|s: &mut State| {
            s.open_quick_switcher();
        })
        .on_key(|s: &mut State, ev, _| {
            if ev.key == rune::Key::Function(2) {
                let id = s.current().id;
                s.begin_rename(id);
                true
            } else {
                false
            }
        })
        .on_secondary_press(|s: &mut State, at, _| {
            let id = s.current().id;
            s.row_menu = Some(RowTarget::Thread(id));
            s.row_menu_at = Some((at.x, at.y));
        });
    // Captain home and crew worker first segments already carry the captain's 16 pt face;
    // only prepend `tab_icon` when the first segment does not.
    let first_has_face = (t.crew_captain.is_some()
        && t.crew_captain.is_some_and(|c| s.captains.get(c).is_some()))
        || (s.is_captain_home(t) && t.captain.is_some_and(|c| s.captains.get(c).is_some()));
    if !first_has_face {
        title = title.child(tab_icon(s, p, t).a11y_hidden());
    }
    if let Some(first) = first {
        title = title.child(first).child(
            icon("chevron-right", p.faint)
                .size(12.0, 12.0)
                .a11y_hidden(),
        );
    }
    title = title.child(
        label(&t.title, TITLE, p.ink)
            .key("thread-title-text")
            .truncate()
            .grow(1.0)
            .min_w(0.0),
    );
    if let Some(machine) = machine {
        let _ = machine;
        title = title.child(machine_tag(
            s,
            p,
            &s.machines.selected,
            "thread-title-machine".into(),
            SMALL,
            true,
        ));
    }
    let rename_id = t.id;
    title = title.child(
        icon_button(
            "thread-title-rename",
            "Rename thread",
            "edit",
            p,
            move |s: &mut State| s.begin_rename(rename_id),
        )
        .group_opacity(0.0, 1.0),
    );
    title = title.child(
        icon("chevron-down", p.faint)
            .size(12.0, 12.0)
            .group_opacity(0.0, 1.0)
            .a11y_hidden(),
    );
    // The help text carries the switcher's shortcut.
    let resolved = s.resolved_keybindings();
    let ctx = s.shortcut_context(false);
    if let Some(shortcut) = crate::keybindings::shortcut_label_for_command(
        &resolved,
        "commandPalette.toggle",
        &ctx,
        crate::keybindings::Platform::current(),
    ) {
        title = title.description(format!("Switch thread ({shortcut})"));
    }
    title
}

/// The right end of the bar: the demo badge, the captain's away line, the diff
/// stat and toggle, the thread menu and the Inspector toggle. The order is
/// fixed so the whole-panel control stays last.
fn actions(
    s: &State,
    p: Palette,
    narrow: bool,
    compact: bool,
    shortcut: impl Fn(&str) -> Option<String>,
) -> El {
    let mut row = row().key("top-bar-actions").gap(2.0).center_y();
    if s.demo {
        row = row.child(outlined(
            el().key("demo-badge")
                .h(20.0)
                .px(6.0)
                .mr(6.0)
                .radius(6.0)
                .center_y()
                .fill(p.card)
                .label("Demo data, read-only")
                .child(meta_label("Demo", p.brass)),
            p,
        ));
    }
    // The captain's away line takes the free middle space at wide windows only,
    // where it cannot push the title into the nav cluster.
    if s.viewport_width >= 1280.0 {
        let available = rune::memo::layout_size("captain-away-slot").map(|size| size[0]);
        if let Some(line) = captain_ui::away_status(s, p, available) {
            row = row.child(
                el().key("captain-away-slot")
                    .max_w(280.0)
                    .min_w(0.0)
                    .shrink()
                    .center_y()
                    .child(line),
            );
        }
    }
    let panel_key = s.panel_key();
    // First control to drop as the row narrows is the stat; the diff toggle
    // stays until compact, where Diff is a tab in the Inspector anyway.
    if !narrow
        && let Some(ready) = s
            .diffs
            .get(&panel_key)
            .and_then(diff_panel::ThreadDiff::ready)
        && (ready.additions > 0 || ready.deletions > 0)
    {
        row = row.child(diff_panel::diff_stat_label(
            "header-diff-stat",
            ready.additions,
            ready.deletions,
            p,
        ));
    }
    if !compact {
        let diff_open = s.right_panel_active_view() == Some(crate::right_panel::PanelView::Diff);
        let mut diff = toggle_button(
            "header-diff",
            "Toggle diff",
            "diff",
            diff_open,
            p,
            |s: &mut State| s.right_panel_toggle_diff(),
        );
        if let Some(shortcut) = shortcut("diff.toggle") {
            diff = diff.description(format!("Toggle diff ({shortcut})"));
        }
        row = row.child(diff);
    }
    row = row.child(toggle_button(
        "thread-actions",
        "More actions",
        "more",
        s.thread_menu,
        p,
        |s| s.thread_menu = !s.thread_menu,
    ));
    // The panel toggle stays at every width: at compact it is the only way to
    // the Inspector.
    let panel_open = s.right_panel().open;
    let mut panel = toggle_button(
        "header-right-panel",
        "Toggle right panel",
        "panel",
        panel_open,
        p,
        |s: &mut State| s.right_panel_toggle(),
    );
    if !panel_open && crew_ui::needs_you_shows(s) {
        panel = panel.child(
            el().size(6.0, 6.0)
                .radius(3.0)
                .fill(p.brass)
                .a11y_hidden()
                .pin(None, Some(2.0), Some(2.0), None),
        );
    }
    if let Some(shortcut) = shortcut("rightPanel.toggle") {
        panel = panel.description(format!("Toggle right panel ({shortcut})"));
    }
    row = row.child(panel);
    row
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_top_bar_is_one_row_no_taller_than_the_strip() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = crate::ui::tests::runtime(width);
            rt.state.tabs_enabled = false;
            rt.state.populate_demo();
            rt.state.compact_sidebar = width < 760.0;
            rt.frame(0.0);
            let (strip, _) = rt.presented("top-strip").unwrap();
            let (bar, _) = rt.presented("top-bar").unwrap();
            assert!(bar[3] <= strip[3] + 0.5, "{width}: {bar:?} in {strip:?}");
            assert!(rt.presented("chat-header").is_none(), "{width}");
            assert!(rt.presented("chat-header-rule").is_none(), "{width}");
            // With tabs off the title button and actions share the row without
            // touching.
            assert!(rt.presented("thread-tabs").is_none(), "{width}");
            let (title, _) = rt.presented("thread-title").unwrap();
            let (actions, _) = rt.presented("top-bar-actions").unwrap();
            let overlap =
                (title[0] + title[2]).min(actions[0] + actions[2]) - title[0].max(actions[0]);
            assert!(
                overlap <= 0.5,
                "{width}: title {title:?} overlaps actions {actions:?}"
            );
        }
    }

    #[test]
    fn chat_main_starts_at_the_panes_top_edge() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = crate::ui::tests::runtime(width);
            rt.state.tabs_enabled = false;
            rt.state.populate_demo();
            rt.state.compact_sidebar = width < 760.0;
            rt.frame(0.0);
            let (pane, _) = rt.presented("chat-pane").unwrap();
            let (main, _) = rt.presented("chat-main").unwrap();
            assert!(
                (main[1] - pane[1]).abs() <= 1.5,
                "{width}: chat-main {main:?} under pane {pane:?}"
            );
        }
    }

    #[test]
    fn nav_buttons_disable_at_the_ends_and_walk_the_history() {
        for width in [960.0, 1280.0] {
            let mut rt = crate::ui::tests::runtime(width);
            rt.state.populate_demo();
            rt.frame(0.0);
            // One entry in history: nothing to go back to.
            assert!(rt.state.nav_history.back().is_none(), "{width}");
            assert!(rt.state.nav_history.forward().is_none(), "{width}");
            // Open a second thread, then Back returns to the first.
            let first = rt.state.current().id;
            rt.state.new_thread();
            rt.invalidate();
            rt.frame(0.0);
            assert!(rt.state.nav_history.back().is_some(), "{width}");
            rt.state.nav_back();
            rt.invalidate();
            rt.frame(0.0);
            assert_eq!(rt.state.current().id, first, "{width}");
            assert!(rt.state.nav_history.forward().is_some(), "{width}");
            rt.state.nav_forward();
            rt.invalidate();
            rt.frame(0.0);
            assert_ne!(rt.state.current().id, first, "{width}");
        }
    }

    #[test]
    fn a_worker_threads_title_holds_open_captain() {
        for width in [960.0, 1280.0, 1920.0] {
            let mut rt = crate::ui::tests::runtime(width);
            rt.state.tabs_enabled = false;
            rt.state.populate_demo();
            rt.state.open_captain(0);
            // Give the captain a crew worker and open it.
            let worker_id = rt.state.next_id;
            rt.state.next_id += 1;
            let mut worker = crate::model::Thread::new(worker_id);
            worker.title = "Fix the sidebar keys".into();
            worker.crew_captain = Some(0);
            worker.captain_home = false;
            rt.state.threads.push(worker);
            let index = rt.state.threads.len() - 1;
            rt.state.show_thread(index);
            rt.invalidate();
            rt.frame(0.0);
            let (bar, _) = rt.presented("top-bar").unwrap();
            let (open, _) = rt.presented("open-captain").unwrap_or_else(|| {
                panic!("{width}: a worker thread shows open-captain in the bar")
            });
            assert!(
                open[0] >= bar[0] - 0.5 && open[0] + open[2] <= bar[0] + bar[2] + 0.5,
                "{width}: open-captain {open:?} outside the bar {bar:?}"
            );
            crate::ui::tests::press(&mut rt, "open-captain");
            rt.frame(0.5);
            assert!(
                rt.state.is_captain_home(rt.state.current()),
                "{width}: open-captain opens the captain's home"
            );
        }
    }

    #[test]
    fn pressing_the_title_opens_the_switcher() {
        let mut rt = crate::ui::tests::runtime(1280.0);
        rt.state.tabs_enabled = false;
        rt.state.populate_demo();
        rt.frame(0.0);
        crate::ui::tests::press(&mut rt, "thread-title");
        rt.frame(0.5);
        assert!(rt.state.quick_switcher, "press opens the quick switcher");
        assert!(rt.presented("quick-switcher").is_some());
    }

    #[test]
    fn the_tab_strip_replaces_the_title_while_tabs_are_on() {
        for width in [960.0, 1280.0] {
            let mut rt = crate::ui::tests::runtime(width);
            rt.state.populate_demo();
            rt.state.tabs_enabled = true;
            rt.invalidate();
            rt.frame(0.0);
            assert!(rt.presented("thread-tabs").is_some(), "{width}");
            assert!(rt.presented("thread-title").is_none(), "{width}");
            // The row still has the nav cluster and the actions.
            assert!(rt.presented("nav-back").is_some(), "{width}");
            assert!(rt.presented("top-bar-actions").is_some(), "{width}");
        }
    }
}
