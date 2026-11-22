//! The Inspector: the one right-side panel, holding four fixed tabs. Ported from
//! KSG Code's `ChatRightPanel` and `RightPanelTabs`; where KSG's strip scrolls and
//! holds any surface, this one keeps Details, Work, Context and Changes in a fixed
//! 36 pt strip, in the same order on every thread.
//!
//! Details is the captain's Brief or the thread's session details. Work lists the
//! thread's provider subagents and its captain's crew. Context is `context_ui::tab`
//! (documents, scope, memory and earlier conversations, plus an open captain
//! document) and Changes is `diff_panel`.
use super::crew_ui::{active_wash, hover_wash, motion};
use super::*;

/// The strip's tabs, in order. Every thread has all four.
pub(super) const TABS: [crate::right_panel::PanelView; 4] = {
    use crate::right_panel::PanelView;
    [
        PanelView::Overview,
        PanelView::Work,
        PanelView::Context,
        PanelView::Diff,
    ]
};

/// One tab of the strip, as KSG's `RightPanelTabs` item: a 24 px borderless row with an
/// icon and its title. The active tab takes `lift` and `ink`; the others stay `muted`
/// and take the hover wash.
fn segment(
    s: &State,
    p: Palette,
    key: &str,
    glyph: &str,
    title: &str,
    badge: Option<(String, Color)>,
    on: bool,
    glyph_only: bool,
    action: impl Fn(&mut State) + 'static,
) -> El {
    let mut content = row()
        .key(key.to_string())
        .role("tab")
        .label(title.to_string())
        .help(title.to_string())
        .focus_selected(on)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .h(24.0)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(6.0)
        .fill(if on { p.lift } else { Color::TRANSPARENT })
        .on_press(action)
        .child(icon(glyph, if on { p.ink } else { p.muted }).size(12.0, 12.0));
    if !on {
        content = content
            .hover(hover_wash(s, p))
            .press(active_wash(s, p))
            .hover_motion(motion(), motion());
    }
    if !glyph_only {
        content = content.child(
            label(
                title.to_string(),
                TextStyle::new(12.0, 500.0),
                if on { p.ink } else { p.muted },
            )
            .truncate(),
        );
    }
    if let Some((badge, ink)) = badge {
        content = content.child(label(badge, TIMER, ink));
    }
    content
}

/// The crew a captain put beside this thread: the home's roster, or a peer worker's
/// siblings.
fn crew_members(s: &State) -> Vec<crew_ui::RosterMember> {
    let t = s.current();
    if s.is_captain_home(t) {
        return t
            .captain
            .map(|captain| crew_ui::roster_members(s, t.id, captain))
            .unwrap_or_default();
    }
    t.peer_origin
        .as_ref()
        .map(|origin| crew_ui::peer_sibling_members(s, t.id, origin))
        .unwrap_or_default()
}

/// The count on the Work tab: crew waiting on the user in brass, else everything
/// working (crew and subagents) in sea. Absent at zero.
fn work_badge(s: &State, p: Palette) -> Option<(String, Color)> {
    let members = crew_members(s);
    let count = |want: Status| members.iter().filter(|m| m.status == want).count();
    let waiting = count(Status::Waiting);
    if waiting > 0 {
        return Some((waiting.to_string(), p.brass));
    }
    let working = count(Status::Running) + crate::agents::active_count(s.current());
    (working > 0).then(|| (working.to_string(), p.sea))
}

/// The strip atop the panel: the four tabs, then the maximize and close controls.
/// Fixed at 36 pt. Under 340 pt the inactive tabs show their icon only.
fn strip(s: &State, p: Palette, width: f32) -> El {
    use crate::right_panel::{PanelView, SingletonKind};
    let panel = s.right_panel();
    let mut segments = row()
        .key("right-panel-tabs")
        .role("tablist")
        .grow(1.0)
        .min_w(0.0)
        .center_y()
        .gap(2.0);
    let compact = width < 340.0;
    for tab in TABS {
        let (key, glyph, title, kind) = match tab {
            PanelView::Overview => (
                "right-panel-segment-overview",
                "info",
                "Details",
                SingletonKind::Overview,
            ),
            PanelView::Work => (
                "right-panel-segment-work",
                "agents",
                "Work",
                SingletonKind::Work,
            ),
            PanelView::Context => (
                "right-panel-segment-context",
                "book",
                "Context",
                SingletonKind::Context,
            ),
            PanelView::Diff => (
                "right-panel-segment-diff",
                "diff",
                "Changes",
                SingletonKind::Diff,
            ),
        };
        let badge = (tab == PanelView::Work).then(|| work_badge(s, p)).flatten();
        let on = panel.view == tab;
        segments = segments.child(segment(
            s,
            p,
            key,
            glyph,
            title,
            badge,
            on,
            compact && !on,
            move |s| {
                let key = s.panel_key();
                s.right_panels.open(&key, kind);
            },
        ));
    }
    row()
        .key("right-panel-strip")
        .h(36.0)
        .pl(8.0)
        .pr(8.0)
        .center_y()
        .gap(4.0)
        .child(segments)
        .child(
            row()
                .center_y()
                .gap(2.0)
                .child(icon_button(
                    "right-panel-maximize",
                    if panel.maximized {
                        "Restore panel"
                    } else {
                        "Maximize panel"
                    },
                    if panel.maximized {
                        "minimize"
                    } else {
                        "expand"
                    },
                    p,
                    |s: &mut State| s.right_panel_toggle_maximized(),
                ))
                .child(icon_button(
                    "right-panel-close",
                    "Close panel",
                    "close",
                    p,
                    move |s: &mut State| {
                        s.right_panel_close_active();
                    },
                )),
        )
}

/// Ported from KSG Code's `RightPanelResizeHandle`: a 4 px hit strip pinned on the panel's
/// left edge, holding a 1 px line that shows `p.frame` on hover and `p.coral` while
/// dragging. Left/Right step by KSG's 16 px; Home/End jump to the minimum and maximum.
fn resize(s: &State, p: Palette, zoom: f32, max: f32) -> El {
    let dragging = s.right_panel_resizing;
    el().key("right-panel-resize")
        .role("separator")
        .label("Resize panel")
        .focusable(true)
        .w(4.0)
        .h_full()
        .cursor(rune::Cursor::ResizeColumn)
        .hover_group()
        .center()
        .pin(Some(0.0), Some(0.0), None, Some(0.0))
        .on_drag(move |s: &mut State, drag, _| {
            if drag.phase == rune::Phase::Down {
                drag.data = s.right_panel_width;
                s.right_panel_resizing = true;
            }
            if drag.cancelled() {
                s.right_panel_width = drag.data;
            } else {
                // The handle sits on the left edge, so dragging left widens the panel.
                let width = drag.data - drag.dx / zoom;
                s.right_panel_width = crate::right_panel::clamp_width(width, max);
            }
            if drag.phase == rune::Phase::Up {
                s.right_panel_resizing = false;
                s.checkpoint();
            }
        })
        .on_key(move |s: &mut State, ev, _| {
            use crate::right_panel::{KEYBOARD_STEP, MIN_WIDTH};
            let next = match ev.key {
                rune::Key::Left => Some(s.right_panel_width - KEYBOARD_STEP),
                rune::Key::Right => Some(s.right_panel_width + KEYBOARD_STEP),
                rune::Key::Home => Some(MIN_WIDTH),
                rune::Key::End => Some(max),
                _ => None,
            };
            let Some(next) = next else {
                return false;
            };
            s.right_panel_width = crate::right_panel::clamp_width(next, max);
            s.checkpoint();
            true
        })
        .child(
            el().w(1.0)
                .h_full()
                .a11y_hidden()
                .fill(if dragging { p.coral } else { p.frame })
                .group_opacity(if dragging { 1.0 } else { 0.0 }, 1.0),
        )
}

/// KSG Code's `ThreadDetailsSection` card: `surface-tray rounded-pane p-1`, a 14 px tray
/// with 4 px of padding that holds an optional heading and one inner surface.
fn details_tray(key: &str, title: &str, p: Palette) -> El {
    col()
        .key(key)
        .role("group")
        .label(title)
        .w_full()
        .min_w(0.0)
        .padding(4.0)
        .radius(14.0)
        .fill(p.tray)
        .border(1.0, p.line)
}

/// The card's 28 px heading (`flex h-7 items-center gap-1 ps-3`): the title, then a muted
/// caption at the end.
fn details_heading(title: &str, summary: Option<El>, p: Palette) -> El {
    let mut heading = row()
        .h(28.0)
        .pl(12.0)
        .pr(8.0)
        .gap(4.0)
        .center_y()
        .child(meta_label(title, p.muted))
        .child(spacer());
    if let Some(summary) = summary {
        heading = heading.child(summary);
    }
    heading
}

/// The card's inner list: `surface-composer rounded-lg p-0.5`.
fn details_surface(p: Palette) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .padding(2.0)
        .radius(10.0)
        .fill(p.card_top)
}

/// KSG's `THREAD_DETAILS_PANEL_ROW_CLASS`: a 28 px row, 10 px in and 4 px at the end, with
/// a 16 px muted icon 10 px before a medium label. Callers add the trailing meta or action.
fn details_row(key: &str, glyph: &str, title: &str, p: Palette) -> El {
    row()
        .key(key)
        .h(28.0)
        .min_w(0.0)
        .pl(10.0)
        .pr(4.0)
        .gap(10.0)
        .center_y()
        .radius(8.0)
        .child(icon(glyph, p.muted).size(16.0, 16.0))
        .child(label(title, TITLE, p.ink).truncate().grow(1.0).min_w(0.0))
}

/// A `details_row` the user can press: KSG's `hover:surface-selected-soft`.
fn details_action(s: &State, p: Palette, row_: El, a11y: String) -> El {
    row_.role("button")
        .cursor(rune::Cursor::Pointer)
        .label(a11y)
        .focusable(true)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
}

/// A row's trailing caption (`THREAD_DETAILS_PANEL_META_CLASS`): tabular and muted.
fn details_meta(value: impl Into<String>, p: Palette) -> El {
    label(value, TIMER, p.muted).pr(6.0)
}

/// The workspace card, ported from KSG Code's `ThreadDetailsPanel` Workspace section: where
/// the thread runs (machine, workspace, branch, changes) in the inner surface, then the
/// context meter under it (`ThreadDetailsContext`).
fn workspace_card(s: &State, p: Palette) -> El {
    let t = s.current();
    let mut rows = details_surface(p).child(details_row(
        "overview-machine",
        "monitor",
        &s.machine_label(&s.machines.selected),
        p,
    ));
    if let Some(project) = s.project(t.project) {
        rows = rows.child(details_row(
            "overview-workspace",
            "folder",
            &project.name,
            p,
        ));
    }
    if let Some(branch) = &t.branch {
        let copy = branch.clone();
        rows = rows.child(
            details_row("overview-branch", "branch", branch, p).child(
                icon_button(
                    "overview-branch-copy",
                    "Copy branch name",
                    "copy",
                    p,
                    |_| {},
                )
                .on_press_tx(move |_: &mut State, tx| tx.copy(copy.clone())),
            ),
        );
    }
    if let Some(ready) = s
        .diffs
        .get(&s.panel_key())
        .and_then(diff_panel::ThreadDiff::ready)
        && (ready.additions > 0 || ready.deletions > 0)
    {
        let stat =
            diff_panel::diff_stat_label("overview-diff-stat", ready.additions, ready.deletions, p);
        rows = rows.child(
            details_action(
                s,
                p,
                details_row("overview-changes", "diff", "Changes", p),
                "Open changes".into(),
            )
            .on_press(|s: &mut State| {
                let key = s.panel_key();
                s.right_panels
                    .open(&key, crate::right_panel::SingletonKind::Diff);
            })
            .child(stat.pr(6.0)),
        );
    }
    let mut card = details_tray("overview-workspace-card", "Workspace", p).child(rows);
    // KSG's context row: a 32 px line with the gauge, a 4 px meter and the percentage. It
    // opens the Context tab, where the breakdown lives.
    if let Some(percent) = t.context.usage.percent {
        let amount = (percent as f32 / 100.0).clamp(0.0, 1.0);
        card = card.child(
            details_action(
                s,
                p,
                row()
                    .key("overview-context")
                    .h(32.0)
                    .pl(12.0)
                    .pr(8.0)
                    .gap(6.0)
                    .center_y()
                    .radius(8.0)
                    .child(icon("gauge", p.muted).size(16.0, 16.0))
                    .child(label("Context", TITLE, p.ink)),
                format!("Context, {percent:.0}% used. Open the Context tab"),
            )
            .on_press(|s: &mut State| {
                let key = s.panel_key();
                s.right_panels
                    .open(&key, crate::right_panel::SingletonKind::Context);
            })
            .child(
                row()
                    .h(4.0)
                    .ml(6.0)
                    .min_w(48.0)
                    .grow(1.0)
                    .radius(2.0)
                    .fill(p.line)
                    .child(el().grow(amount).h_full().radius(2.0).fill(p.ink))
                    .child(el().grow(1.0 - amount)),
            )
            .child(label(context_percent_text(percent), TIMER, p.muted)),
        );
        if let (Some(tokens), Some(limit)) =
            (t.context.usage.tokens, t.context.usage.context_window)
        {
            card = card.child(
                details_surface(p).child(
                    row()
                        .key("overview-context-tokens")
                        .h(32.0)
                        .px(10.0)
                        .gap(8.0)
                        .center_y()
                        .child(label("Used / max", TITLE, p.ink).grow(1.0).min_w(0.0))
                        .child(label(
                            format!(
                                "{} / {}",
                                context_tokens_text(tokens),
                                context_tokens_text(limit)
                            ),
                            TIMER,
                            p.muted,
                        )),
                ),
            );
        }
    }
    card
}

/// The session card. KSG Code has no equivalent, because its composer always shows the
/// model, effort and access: this is a Shina-only card in the same tray-and-rows shape, so
/// those facts, the saved session ID and any session notices keep one place to be read.
fn session_card(s: &State, p: Palette) -> El {
    let t = s.current();
    let status = row()
        .center_y()
        .gap(6.0)
        .child(status_mark(t.status, p, 8.0))
        .child(label(
            t.started
                .filter(|_| t.status == Status::Running)
                .map_or_else(
                    || t.status.label().into(),
                    |at| format!("Working for {}", timer_label(at.elapsed().as_secs())),
                ),
            TIMER,
            status_ink(t.status, p),
        ));
    let model = if t.thinking.is_empty() || t.thinking == "off" {
        t.model.clone()
    } else {
        format!("{} · {}", t.model, effort_name(&t.thinking))
    };
    let mut rows = details_surface(p)
        .child(details_row("overview-model", "chip", &model, p))
        .child(details_row(
            "overview-access",
            "shield",
            t.native.access.label(),
            p,
        ))
        .child(details_row(
            "overview-harness",
            "terminal",
            &harness_name(s, &t.harness),
            p,
        ));
    if let Some(updated) = relative_time(t.updated) {
        rows = rows.child(
            details_row("overview-updated", "clock", "Updated", p).child(details_meta(updated, p)),
        );
    }
    if let Some(session) = &t.session {
        let copy = session.clone();
        rows = rows.child(
            row()
                .key("overview-session")
                .h(28.0)
                .min_w(0.0)
                .pl(10.0)
                .pr(4.0)
                .gap(10.0)
                .center_y()
                .child(icon("file", p.muted).size(16.0, 16.0))
                .child(
                    label(session, MONO_META, p.muted)
                        .truncate_middle()
                        .selectable(true)
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    icon_button(
                        "overview-session-copy",
                        "Copy session ID",
                        "copy",
                        p,
                        |_| {},
                    )
                    .on_press_tx(move |_: &mut State, tx| tx.copy(copy.clone())),
                ),
        );
    }
    let mut card = details_tray("overview-session-card", "Session", p)
        .child(details_heading("Session", Some(status), p))
        .child(rows);
    if !t.diagnostics.is_empty() {
        let key = s.panel_key();
        let expanded = s.session_notices.contains(&key);
        let mut notices = details_surface(p).mt(4.0).child(
            details_action(
                s,
                p,
                details_row("overview-session-notices", "info", "Session notices", p),
                format!("Session notices, {}", t.diagnostics.len()),
            )
            .expanded(expanded)
            .on_press(move |s: &mut State| {
                if !s.session_notices.remove(&key) {
                    s.session_notices.insert(key.clone());
                }
            })
            .child(details_meta(t.diagnostics.len().to_string(), p))
            .child(
                icon(if expanded { "chevron" } else { "open" }, p.faint)
                    .size(12.0, 12.0)
                    .mr(6.0),
            ),
        );
        if expanded {
            for diagnostic in &t.diagnostics {
                notices = notices.child(
                    label(truncate(diagnostic, 600), SMALL, p.muted)
                        .wrap_anywhere()
                        .selectable(true)
                        .px(10.0)
                        .py(4.0),
                );
            }
        }
        card = card.child(notices);
    }
    card
}

/// The lineage card on a crew worker, in the shape of KSG Code's `ThreadRelationshipsPanel`
/// Lineage card: who delegated the thread, pressable back to the captain's home, and where
/// it runs.
fn lineage_card(s: &State, p: Palette) -> Option<El> {
    let t = s.current();
    let captain = t
        .captain
        .or(t.crew_captain)
        .filter(|c| *c < s.captains.len());
    let mut rows = details_surface(p);
    let mut any = false;
    if let Some(captain) = captain {
        let profile = &s.captains[captain];
        any = true;
        rows = rows.child(
            details_action(
                s,
                p,
                row()
                    .key("open-captain-from-overview")
                    .h(28.0)
                    .min_w(0.0)
                    .pl(10.0)
                    .pr(4.0)
                    .gap(10.0)
                    .center_y()
                    .radius(8.0)
                    .child(avatar(s, p, captain, 16.0, s.captain_status(captain)))
                    .child(
                        label(&profile.name, TITLE, p.ink)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
                format!("Open {}", profile.name),
            )
            .on_press(move |s: &mut State| s.open_captain(captain))
            .child(details_meta("Captain", p))
            .child(icon("open", p.faint).size(12.0, 12.0).mr(6.0)),
        );
    } else if let Some(origin) = &t.peer_origin {
        let server_id = origin.server_id.clone();
        let home_id = origin.parent_thread_id;
        let linked = s
            .client
            .as_ref()
            .is_some_and(|c| c.linked.contains_key(&server_id));
        any = true;
        let base = details_row(
            "open-captain-from-overview",
            "crew",
            &format!("{} on {}", origin.captain, origin.server_label),
            p,
        );
        rows = rows.child(if linked {
            details_action(
                s,
                p,
                base,
                format!("Open {} on {}", origin.captain, origin.server_label),
            )
            .on_press(move |s: &mut State| s.open_machine_thread(&server_id, home_id))
            .child(icon("open", p.faint).size(12.0, 12.0).mr(6.0))
        } else {
            base
        });
    }
    if let Some(placement) = crew_ui::worker_placement(s, t.id) {
        let machine = crew_ui::placement_machine(s, t.id, &placement);
        any = true;
        rows = rows.child(
            label(
                crew_ui::placement_details(&placement, &machine),
                SMALL,
                p.muted,
            )
            .key("worker-placement")
            .wrap()
            .px(10.0)
            .py(6.0),
        );
    }
    any.then(|| {
        details_tray("brief-delegated", "Lineage", p)
            .child(details_heading("Lineage", None, p))
            .child(rows)
    })
}

/// The Details tab on a plain thread or a crew worker, ported from KSG Code's
/// `ThreadDetailsPanel`: a stack of tray cards 4 px apart (`thread-details-stack`), the
/// workspace card first.
fn session_body(s: &State, p: Palette) -> El {
    // KSG's `.thread-details-stack` lifts its own text a step: `--foreground` is `#f5f5f5`
    // and `--muted-foreground` `#a3a3a3` in dark, and `--foreground` `#18181b` in light.
    let p = if s.dark {
        Palette {
            ink: color("#f5f5f5"),
            muted: color("#a3a3a3"),
            ..p
        }
    } else {
        Palette {
            ink: color("#18181b"),
            ..p
        }
    };
    let mut stack = col()
        .w_full()
        .min_w(0.0)
        .padding(8.0)
        .gap(4.0)
        .child(workspace_card(s, p));
    if let Some(lineage) = lineage_card(s, p) {
        stack = stack.child(lineage);
    }
    stack = stack.child(session_card(s, p));
    col()
        .key("session-panel")
        .min_h(0.0)
        .h_full()
        .w_full()
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(stack),
        )
}

/// The Details tab: the captain's Brief sections on a home thread, else the detail cards.
fn overview_body(s: &State, p: Palette) -> El {
    if let Some(captain) = captain_ui::home_brief_captain(s) {
        captain_ui::brief_panel_for(s, p, captain, false)
    } else {
        session_body(s, p)
    }
}

/// The crew half of the Work tab: the captain's roster where a captain backs the
/// thread, or a peer worker's siblings.
fn crew_body(s: &State, p: Palette) -> El {
    let t = s.current();
    let captain = t
        .captain
        .filter(|_| s.is_captain_home(t))
        .or(t.crew_captain)
        .filter(|captain| *captain < s.captains.len());
    match captain {
        Some(captain) => captain_ui::crew_section(s, p, captain),
        None => peer_crew_body(s, p),
    }
}

/// The Work tab: everything running for this thread in one place. Provider subagents
/// (`agents_ui::view`) sit above the captain's crew roster. A thread with only one of
/// them gives it the whole tab, and a thread with neither says so. When both show, the
/// subagent list takes the height its rows need, capped at 40% of the viewport, and
/// the roster keeps its pinned footer below.
fn work_body(s: &State, p: Palette) -> El {
    let t = s.current();
    let has_agents = crate::agents::has_agents(t);
    let has_crew = s.thread_has_crew();
    let tab = col().key("work-tab").h_full().min_h(0.0).w_full();
    match (has_agents, has_crew) {
        (false, false) => tab.center().child(
            col()
                .key("work-empty")
                .max_w(240.0)
                .gap(6.0)
                .items_center()
                .child(icon("agents", p.faint).size(20.0, 20.0))
                .child(label("Nothing running", LABEL, p.ink))
                .child(
                    label(
                        "Subagents and crew show here while they work.",
                        SMALL,
                        p.muted,
                    )
                    .align(rune::Align::Center)
                    .wrap()
                    .w_full(),
                ),
        ),
        (true, false) => tab.child(agents_ui::view(s, p)),
        (false, true) => tab.child(crew_body(s, p)),
        (true, true) => {
            let runs = crate::agents::runs(t);
            let groups = runs
                .iter()
                .map(|run| run.tool_id)
                .collect::<std::collections::HashSet<_>>()
                .len();
            let needed = 48.0 + groups as f32 * 29.0 + runs.len() as f32 * 45.0;
            let cap = (s.viewport_height * 0.4).max(140.0);
            tab.child(
                col()
                    .key("work-agents")
                    .w_full()
                    .h(needed.min(cap))
                    .child(agents_ui::view(s, p)),
            )
            .child(el().h(1.0).w_full().fill(p.line).hairline())
            .child(crew_body(s, p))
        }
    }
}

/// The crew half of the Work tab on a peer-launched worker (spec §3.5): the workers the same remote
/// delegation put on this machine, with the open worker filled. There is no local
/// captain to foot the tab with, so no footer.
fn peer_crew_body(s: &State, p: Palette) -> El {
    let t = s.current();
    let Some(origin) = t.peer_origin.as_ref() else {
        return col().key("crew-tab").min_h(0.0).w_full();
    };
    let members = crew_ui::peer_sibling_members(s, t.id, origin);
    // The captain's name heads the list; the open worker's own row follows, filled.
    let server_id = origin.server_id.clone();
    let home_id = origin.parent_thread_id;
    let linked = s
        .client
        .as_ref()
        .is_some_and(|c| c.linked.contains_key(&server_id));
    let mut cap_head = row()
        .key("crew-tab-captain")
        .h(32.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .child(
            label(
                format!("Captain {} ({})", origin.captain, origin.server_label),
                TextStyle::new(12.0, 500.0).lh(16.0),
                p.muted,
            )
            .truncate()
            .min_w(0.0),
        );
    if linked {
        cap_head = cap_head
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("Open Captain {}", origin.captain))
            .focusable(true)
            .hover(hover_wash(s, p))
            .press(active_wash(s, p))
            .hover_motion(motion(), motion())
            .on_press(move |s: &mut State| s.open_machine_thread(&server_id, home_id));
    }
    let mut rows = col().gap(2.0).pt(8.0).child(cap_head);
    // The open worker's own row leads the siblings, filled (spec §3.5).
    let own_member = crew_ui::RosterMember {
        model: t.model.clone(),
        thread_id: t.id,
        title: if t.title.is_empty() {
            "Untitled".into()
        } else {
            t.title.clone()
        },
        status: if t.request.is_some() {
            crate::model::Status::Waiting
        } else {
            t.status
        },
        word: crate::model::status_word(t.status).to_string(),
        clock: crate::model::status_word(t.status).to_string(),
        remote_server: (t.machine != "local").then(|| t.machine.clone()),
        rank: 0,
        updated: t.updated,
        queued: false,
        place: None,
    };
    rows = rows.child(
        crew_ui::roster_row(s, p, &own_member, format!("crew-chip-{}", t.id), false)
            .fill(active_wash(s, p)),
    );
    for member in &members {
        rows = rows.child(crew_ui::roster_row(
            s,
            p,
            member,
            format!("crew-chip-{}", member.thread_id),
            true,
        ));
    }
    if members.is_empty() {
        rows = rows.child(
            captain_ui::quiet_note("No other crew members on this machine.", p)
                .key("crew-roster-empty"),
        );
    }
    col()
        .key("crew-tab")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(rows)
}

/// The Changes tab keeps its embedded header row, as before.
fn diff_body(s: &State, p: Palette) -> El {
    col()
        .min_h(0.0)
        .child(diff_panel::header(s, p))
        .child(el().h(12.0))
        .child(
            el().grow(1.0)
                .min_h(0.0)
                .center()
                .child(diff_panel::body(s, p)),
        )
}

/// The body a tab draws, under its `right-panel-body-{view}` key. `context_ui::view`
/// owns the entire Context tab, including its document-viewer route, so Workstream 4
/// can evolve both the Context list and the document editor header inside `context_ui.rs`.
fn body(s: &State, p: Palette) -> (String, El) {
    use crate::right_panel::PanelView;
    match s.right_panel().view {
        PanelView::Overview => ("right-panel-body-overview".into(), overview_body(s, p)),
        PanelView::Work => ("right-panel-body-work".into(), work_body(s, p)),
        PanelView::Context => context_ui::view(s, p),
        PanelView::Diff => ("right-panel-body-diff".into(), diff_body(s, p)),
    }
}

/// The Inspector shell: a 1 px `line` left edge, the tab strip and its rule,
/// then the active tab's own scroll area. `resize_max` is `Some` only when the
/// panel sits inline, so a covering panel has no resize handle.
pub(super) fn view(s: &State, p: Palette, zoom: f32, resize_max: Option<f32>) -> El {
    let (key, body) = body(s, p);
    let panel_el = el()
        .key("right-panel")
        .role("group")
        .label("Inspector")
        .h_full()
        .min_h(0.0)
        .fill(p.panel)
        .child(el().w(1.0).h_full().a11y_hidden().fill(p.line).hairline())
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .min_h(0.0)
                .child(strip(
                    s,
                    p,
                    resize_max.map_or(s.viewport_width, |max| {
                        crate::right_panel::clamp_width(s.right_panel_width, max)
                    }),
                ))
                .child(rule(p).key("right-panel-strip-rule"))
                .child(el().key(key).grow(1.0).min_h(0.0).min_w(0.0).child(body)),
        );
    match resize_max {
        Some(max) => panel_el.child(resize(s, p, zoom, max)),
        None => panel_el,
    }
}
