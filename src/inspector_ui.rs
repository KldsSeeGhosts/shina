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
    } else {
        session_body(s, p)
    }
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
        Some(captain) => col()
            .key("crew-tab")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(captain_ui::crew_section(s, p, captain)),
        None => col()
            .key("crew-tab")
            .min_h(0.0)
            .w_full()
            .child(captain_ui::quiet_note("This thread has no crew.", p)),
    }
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
