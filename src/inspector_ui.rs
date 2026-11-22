//! The Inspector: the one right-side panel. Ported from KSG Code's `ChatRightPanel` and
//! `RightPanelTabs`; where KSG's strip scrolls and holds any surface, this one keeps Work,
//! Context and Changes in a fixed 36 pt strip, with a Captain's Brief first on its home.
//!
//! A thread's own details are not here: they are the floating card in `details_ui`, as in
//! KSG. Work lists the thread's provider subagents and its captain's crew. Context is
//! `context_ui::tab` (documents, scope, memory and earlier conversations, plus an open
//! captain document) and Changes is `diff_panel`.
use super::crew_ui::{active_wash, hover_wash, motion};
use super::*;

/// The strip's tabs, in order. `Overview` is the Brief and shows on a Captain's home only.
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
pub(super) fn crew_members(s: &State) -> Vec<crew_ui::RosterMember> {
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

/// The strip atop the panel: the tabs, then the maximize and close controls.
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
        if tab == PanelView::Overview && !s.has_brief_tab() {
            continue;
        }
        let (key, glyph, title, kind) = match tab {
            PanelView::Overview => (
                "right-panel-segment-overview",
                "info",
                "Brief",
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

/// The Brief tab, on a Captain's home only: `State::right_panel` sends every other thread
/// to Work, and a thread's own details are the floating card (`details_ui`).
fn overview_body(s: &State, p: Palette) -> El {
    captain_ui::home_brief_captain(s).map_or_else(el, |captain| {
        captain_ui::brief_panel_for(s, p, captain, false)
    })
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
