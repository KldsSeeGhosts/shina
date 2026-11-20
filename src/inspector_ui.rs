//! The Inspector: the one right-side panel, holding four fixed tabs. Ported from
//! KSG Code's `ChatRightPanel` and `RightPanelTabs`, which the old `right_panel`
//! already followed; where KSG's strip scrolls, this one keeps Overview, Crew,
//! Context and Diff in a fixed 36 pt strip.
//!
//! Tab bodies are still the sections the app already had: Overview is the
//! captain's Brief or the thread's session details, Crew the roster section,
//! Context `context_ui::tab` (documents, scope, memory and earlier conversations,
//! plus an open captain document) and Diff `diff_panel`. Workstreams 3 and 4
//! reshape each body in place; the shell is settled here.
use super::crew_ui::{active_wash, hairline, hover_wash, motion};
use super::*;

/// The tabs a thread offers, in strip order. Captain homes and crew workers get
/// all four; a plain thread has no Crew tab.
pub(super) fn tabs(s: &State) -> Vec<crate::right_panel::PanelView> {
    use crate::right_panel::PanelView;
    if s.inspector_has_crew_tab() {
        return vec![
            PanelView::Overview,
            PanelView::Crew,
            PanelView::Context,
            PanelView::Diff,
        ];
    }
    vec![PanelView::Overview, PanelView::Context, PanelView::Diff]
}

/// The tab the shell actually draws: a stored view the thread does not have
/// falls back to Overview.
fn resolved_view(s: &State) -> crate::right_panel::PanelView {
    s.right_panel_resolved_view()
}

/// One segment of the strip's control: a 24 px pressable row with an icon and
/// its title, the active segment framed in `card`/`frame` as before.
fn segment(
    p: Palette,
    key: &str,
    glyph: &str,
    title: &str,
    badge: Option<String>,
    on: bool,
    action: impl Fn(&mut State) + 'static,
) -> El {
    let mut content = row()
        .key(key.to_string())
        .role("tab")
        .label(title.to_string())
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .h(24.0)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(6.0)
        .border(1.0, if on { p.frame } else { Color::TRANSPARENT })
        .fill(if on { p.card } else { Color::TRANSPARENT })
        .hover(p.selected)
        .press(p.selected)
        .on_press(action)
        .child(icon(glyph, if on { p.ink } else { p.muted }).size(12.0, 12.0))
        .child(
            label(
                title.to_string(),
                TextStyle::new(12.0, 500.0),
                if on { p.ink } else { p.muted },
            )
            .truncate(),
        );
    if let Some(badge) = badge {
        content = content.child(label(badge, TIMER, p.faint));
    }
    content
}

/// The working count on the Crew segment, absent at zero.
fn crew_working_count(s: &State) -> Option<String> {
    let t = s.current();
    let home = if s.is_captain_home(t) {
        Some(t.id)
    } else {
        None
    };
    let captain = t.captain.or(t.crew_captain)?;
    let members = match home {
        Some(home) => crew_ui::roster_members(s, home, captain),
        None => t
            .peer_origin
            .as_ref()
            .map(|origin| crew_ui::peer_sibling_members(s, t.id, origin))
            .unwrap_or_default(),
    };
    let working = members
        .iter()
        .filter(|member| member.status == Status::Running)
        .count();
    (working > 0).then(|| working.to_string())
}

/// The strip atop the panel: the four-segment control, then the maximize and
/// close controls. Fixed at 36 pt, since nothing else in the pane shares its
/// line now.
fn strip(s: &State, p: Palette) -> El {
    let panel = s.right_panel();
    let view = resolved_view(s);
    let mut segments = row()
        .key("right-panel-tabs")
        .grow(1.0)
        .min_w(0.0)
        .center_y()
        .gap(2.0);
    for tab in tabs(s) {
        let (key, glyph, title, kind) = match tab {
            crate::right_panel::PanelView::Overview => (
                "right-panel-segment-overview",
                "info",
                "Overview",
                crate::right_panel::SingletonKind::Overview,
            ),
            crate::right_panel::PanelView::Agents => (
                "right-panel-segment-agents",
                "Agents",
                "crew",
                crate::right_panel::SingletonKind::Agents,
            ),
            crate::right_panel::PanelView::Crew => (
                "right-panel-segment-crew",
                "crew",
                "Crew",
                crate::right_panel::SingletonKind::Crew,
            ),
            crate::right_panel::PanelView::Context => (
                "right-panel-segment-context",
                "book",
                "Context",
                crate::right_panel::SingletonKind::Context,
            ),
            crate::right_panel::PanelView::Diff => (
                "right-panel-segment-diff",
                "diff",
                "Diff",
                crate::right_panel::SingletonKind::Diff,
            ),
        };
        let badge = match tab {
            crate::right_panel::PanelView::Crew => crew_working_count(s),
            _ => None,
        };
        let on = view == tab;
        segments = segments.child(segment(p, key, glyph, title, badge, on, move |s| {
            let key = s.panel_key();
            s.right_panels.open(&key, kind);
        }));
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

/// The thread's session details, as the old Session drawer showed them, under
/// the `session-panel` key without its own header (the strip names the tab).
fn session_body(s: &State, p: Palette) -> El {
    let t = s.current();
    let mut details = col()
        .padding(16.0)
        .gap(16.0)
        .child(label(&t.title, TextStyle::new(14.0, 600.0), p.ink).wrap())
        .child(status(t.status, p))
        .child(rule(p))
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Model", p.muted))
                .child(label(&t.model, LABEL, p.ink).wrap()),
        )
        .child(
            col()
                .gap(5.0)
                .child(meta_label("Conversation", p.muted))
                .child(
                    label(
                        if t.session.is_some() {
                            format!("Saved locally by {}", harness_name(s, &t.harness))
                        } else {
                            "No saved session yet".to_string()
                        },
                        LABEL,
                        p.ink,
                    )
                    .wrap(),
                ),
        );
    if let Some(session) = &t.session {
        details = details.child(
            label(session, SMALL, p.muted)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    if !t.diagnostics.is_empty() {
        details = details
            .child(rule(p))
            .child(label("Session notices", LABEL, p.ink));
    }
    for diagnostic in &t.diagnostics {
        details = details.child(
            label(truncate(diagnostic, 600), SMALL, p.muted)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    for diagnostic in s.updater.diagnostics() {
        details = details.child(
            label(diagnostic, SMALL, p.muted)
                .wrap_anywhere()
                .selectable(true),
        );
    }
    col().key("session-panel").min_h(0.0).w_full().child(
        col()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(details),
    )
}

/// The Overview tab: the captain's Brief sections on a home or worker thread,
/// else the session details.
fn overview_body(s: &State, p: Palette) -> El {
    if let Some(captain) = captain_ui::home_brief_captain(s) {
        captain_ui::brief_panel_for(s, p, captain, false)
    } else if s.current().crew_captain.is_some()
        || s.current().delegated
        || s.current().peer_origin.is_some()
    {
        worker_overview_body(s, p)
    } else {
        session_body(s, p)
    }
}

/// The Overview tab on a crew worker (spec §3.4): the worker's title and status, a
/// pressable "Delegated by" row back to its captain, a "Runs" line for its placement,
/// then the session details below.
fn worker_overview_body(s: &State, p: Palette) -> El {
    let t = s.current();
    let captain = t
        .captain
        .or(t.crew_captain)
        .filter(|c| *c < s.captains.len());
    let mut identity = row()
        .key("brief-identity")
        .px(16.0)
        .pt(14.0)
        .pb(12.0)
        .gap(12.0)
        .center_y();
    if let Some(captain) = captain {
        identity = identity.child(avatar(s, p, captain, 40.0, shown_status(s)));
    }
    identity = identity.child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .gap(1.0)
            .child(label(&t.title, IDENTITY, p.ink).truncate().min_w(0.0))
            .child(label(crate::model::status_word(t.status), META, p.muted).truncate()),
    );
    let mut body = col()
        .key("brief-sections")
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(identity)
        .child(el().h(1.0).mx(16.0).fill(hairline(p)));
    // "Delegated by": the captain's face and name, pressable back to its home.
    if let Some(captain) = captain {
        let profile = &s.captains[captain];
        body = body.child(
            captain_ui::section("brief-delegated", "Delegated by", None, vec![], p).child(
                row()
                    .key("open-captain-from-overview")
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label(format!("Open {}", profile.name))
                    .focusable(true)
                    .px(10.0)
                    .min_h(40.0)
                    .gap(10.0)
                    .center_y()
                    .radius(6.0)
                    .hover(hover_wash(s, p))
                    .press(active_wash(s, p))
                    .hover_motion(motion(), motion())
                    .on_press(move |s: &mut State| s.open_captain(captain))
                    .child(avatar(s, p, captain, 24.0, s.captain_status(captain)))
                    .child(
                        label(
                            &profile.name,
                            TextStyle::new(13.0, 500.0).lh(19.0).tracking(-0.08),
                            p.ink,
                        )
                        .truncate()
                        .min_w(0.0),
                    ),
            ),
        );
    } else if let Some(origin) = &t.peer_origin {
        let server_id = origin.server_id.clone();
        let home_id = origin.parent_thread_id;
        let linked = s
            .client
            .as_ref()
            .is_some_and(|c| c.linked.contains_key(&server_id));
        let mut cap_row = row()
            .key("open-captain-from-overview")
            .px(10.0)
            .min_h(40.0)
            .gap(10.0)
            .center_y()
            .radius(6.0)
            .child(icon("crew", p.muted))
            .child(
                label(
                    format!("{} on {}", origin.captain, origin.server_label),
                    TextStyle::new(13.0, 500.0).lh(19.0).tracking(-0.08),
                    p.ink,
                )
                .truncate()
                .min_w(0.0),
            );
        if linked {
            cap_row = cap_row
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!(
                    "Open {} on {}",
                    origin.captain, origin.server_label
                ))
                .focusable(true)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| s.open_machine_thread(&server_id, home_id));
        }
        body = body.child(
            captain_ui::section("brief-delegated", "Delegated by", None, vec![], p).child(cap_row),
        );
    }
    // "Runs": the placement details the parent bar used to carry.
    if let Some(placement) = crew_ui::worker_placement(s, t.id) {
        let machine = crew_ui::placement_machine(s, t.id, &placement);
        let details = crew_ui::placement_details(&placement, &machine);
        body = body.child(
            captain_ui::section("brief-runs", "Runs", None, vec![], p).child(
                label(details, SMALL, p.muted)
                    .key("worker-placement")
                    .wrap()
                    .px(10.0)
                    .pb(4.0),
            ),
        );
    }
    body = body
        .child(el().h(1.0).mx(16.0).fill(hairline(p)))
        .child(session_body(s, p));
    col()
        .key("worker-overview")
        .min_h(0.0)
        .h_full()
        .w_full()
        .child(body)
}

/// The Crew tab: the captain's roster section where a captain backs the
/// thread, else a quiet note. Workstream 3 gives it the grouped roster.
fn crew_body(s: &State, p: Palette) -> El {
    let t = s.current();
    let captain = t
        .captain
        .filter(|_| s.is_captain_home(t))
        .or(t.crew_captain)
        .filter(|captain| *captain < s.captains.len());
    match captain {
        Some(captain) => captain_ui::crew_section(s, p, captain),
        None if t.peer_origin.is_some() => peer_crew_body(s, p),
        None => col()
            .key("crew-tab")
            .min_h(0.0)
            .w_full()
            .child(captain_ui::quiet_note("This thread has no crew.", p)),
    }
}

/// The Crew tab on a peer-launched worker (spec §3.5): the workers the same remote
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
            .h(32.0)
            .px(10.0)
            .fill(active_wash(s, p)),
    );
    for member in &members {
        rows = rows.child(
            crew_ui::roster_row(
                s,
                p,
                member,
                format!("crew-chip-{}", member.thread_id),
                true,
            )
            .h(32.0)
            .px(10.0),
        );
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

/// The Diff tab keeps its embedded header row, as before.
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
    match resolved_view(s) {
        crate::right_panel::PanelView::Overview => {
            ("right-panel-body-overview".into(), overview_body(s, p))
        }
        crate::right_panel::PanelView::Agents => {
            ("right-panel-body-agents".into(), overview_body(s, p))
        }
        crate::right_panel::PanelView::Crew => ("right-panel-body-crew".into(), crew_body(s, p)),
        crate::right_panel::PanelView::Context => context_ui::view(s, p),
        crate::right_panel::PanelView::Diff => ("right-panel-body-diff".into(), diff_body(s, p)),
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
        .child(el().w(1.0).h_full().a11y_hidden().fill(p.line))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .min_h(0.0)
                .child(strip(s, p))
                .child(rule(p).key("right-panel-strip-rule"))
                .child(el().key(key).grow(1.0).min_h(0.0).min_w(0.0).child(body)),
        );
    match resize_max {
        Some(max) => panel_el.child(resize(s, p, zoom, max)),
        None => panel_el,
    }
}
