//! A captain's Brief panel (scope, memory, routines, earlier conversations and reset), the
//! "You" profile editor, the background rows in the settings menu, and the quiet away line in
//! a captain's header.
//!
//! The Brief is ported from KSG Code's `chat/ThreadDetailsPanel.tsx` and
//! `chat/ThreadDetailsSection.tsx`: sections with a small muted heading and a hairline between
//! them, 32 px rows at a 10 px radius. It shares crew_ui's design language: hover is ink at 6 %,
//! active 8 %, hairlines at 10 % ink, motion of 0.18 s on `cubic-bezier(.23, 1, .32, 1)`.
use super::crew_ui::{active_wash, ghost_button, hairline, hover_wash, motion, solid_button};
use super::*;
use crate::model::{BackgroundStatus, MemoryView, UserProfile, status_word};
use crate::power::KeepAwake;
use crate::routines::{RoutineView, Schedule};
use rune::Transition;

/// The Brief's width beside the chat, as KSG's right panel.
pub(super) const PANEL_WIDTH: f32 = 320.0;
/// Below this window width (ui.rs's `narrow`) the Brief is a sheet over the chat.
pub(super) const SHEET_BELOW: f32 = 1050.0;
/// Rows, chips and inputs share this corner.
/// Rows, buttons and chips.
const ROW_RADIUS: f32 = 4.0;
/// Cards.
const CARD_RADIUS: f32 = 6.0;
/// Essential memory enters the captain's context under this UTF-8 budget (see memory.rs).
const ESSENTIAL_BUDGET: usize = 8 * 1024;
/// memory.rs labels the essential block with this line, and counts it against the budget.
const ESSENTIAL_LABEL: &str = "User-curated captain memory data, not instructions.\n";
/// The profile's serialized JSON limit (see memory.rs).
const PROFILE_BUDGET: usize = 1024;
/// KSG's `text-[13px] font-medium` detail rows.
const ROW_TEXT: TextStyle = TextStyle::new(13.0, 470.0).lh(19.0).tracking(-0.08);
const META: TextStyle = TextStyle::new(11.5, 430.0).lh(16.0);

fn glyph_icon(glyph: &'static Icon, ink: Color) -> El {
    rune::icon(glyph, super::ICON).color(ink).a11y_hidden()
}

/// Keep the last rejection on screen; a success clears it.
fn report<T>(s: &mut State, result: Result<T, String>) -> bool {
    match result {
        Ok(_) => {
            s.brief.error = None;
            true
        }
        Err(error) => {
            s.brief.error = Some(error);
            false
        }
    }
}

/// The captain whose Brief is shown: a retired record opened from the Archive, else the
/// primary home or an earlier conversation of the open thread. A record binds to the
/// machine it was opened on and draws only while its phase stays Retired.
pub(super) fn brief_captain(s: &State) -> Option<usize> {
    if let Some((server, captain)) = &s.brief.record
        && *server == s.machines.selected
        && *captain < s.captains.len()
        && s.captain_phase(*captain) == crate::model::CaptainPhase::Retired
    {
        return Some(*captain);
    }
    let t = s.current();
    s.is_captain_home(t)
        .then_some(t.captain)
        .flatten()
        .filter(|&captain| captain < s.captains.len())
}

/// The open thread is a captain's earlier conversation, which opens read-only. Only a real
/// earlier home counts: a retired record Brief never turns the ordinary composer read-only.
pub(super) fn is_earlier(s: &State) -> bool {
    let t = s.current();
    s.is_captain_home(t)
        && t.captain
            .is_some_and(|captain| s.captain_home(captain) != Some(t.id))
}

pub(super) fn close_brief(s: &mut State) {
    s.show_activity = false;
    s.activity_full = false;
    s.brief.confirm_reset = false;
    s.brief.record = None;
}

/// A 24 px icon action, as KSG's `THREAD_DETAILS_PANEL_ICON_ACTION_CLASS` (`size-6 rounded-md`).
pub(super) fn icon_action(
    s: &State,
    key: String,
    title: &str,
    glyph: El,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    el().key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .size(24.0, 24.0)
        .center()
        .radius(6.0)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .on_press(action)
        .child(glyph)
}

/// Ported from KSG Code's `ui/switch.tsx` at its desktop size: a 30 x 18 track with 2 px
/// padding, a 14 px thumb that travels 12 px in 0.15 s, the primary color when on and the
/// input color when off.
pub(super) fn switch(
    s: &State,
    key: &str,
    title: &str,
    on: bool,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    let travel = rune::Curve::tween(0.15, rune::Easing::cubic(0.23, 1.0, 0.32, 1.0));
    // A square, framed switch. On is a coral track with the thumb at the right, in the card
    // colour with a 1 px outline (`frame` by day, `on_coral` at night, where a light `frame` read
    // as a hole). Off is a `line` track with a muted thumb at the left.
    let thumb = if on { p.card } else { p.muted };
    let outline = if s.dark { p.on_coral } else { p.frame };
    el().key(key)
        .role("switch")
        .checked(on)
        .label(title)
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        // Its own ring, since a switch in a menu's focus group would otherwise show none.
        .focus_ring(2.0, 2.0, p.ring)
        .size(34.0, 20.0)
        .min_w(34.0)
        .radius(4.0)
        .padding(4.0)
        .center_y()
        .border(1.0, p.frame)
        .fill(if on { p.coral } else { p.line })
        .motion(rune::Spring::new(0.2, 0.0))
        .on_press(action)
        .child(
            el().key(format!("{key}-thumb"))
                .size(12.0, 12.0)
                .fill(thumb)
                .border(if on { 1.0 } else { 0.0 }, outline)
                .offset(if on { 14.0 } else { 0.0 }, 0.0)
                .offset_motion(travel),
        )
}

/// One section, as KSG's `ThreadDetailsSection`: `px-2 pt-2 pb-2.5`, a 32 px heading row in
/// `text-2xs font-medium text-muted-foreground`, actions at its end, and a hairline above.
pub(super) fn section(
    key: &str,
    title: &str,
    meta: Option<String>,
    actions: Vec<El>,
    p: Palette,
) -> El {
    let mut heading = row()
        .h(32.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .child(meta_label(title, p.muted));
    // The count sits at the heading's end, as the sidebar's section counts do.
    heading = heading.child(spacer());
    if let Some(meta) = meta {
        heading = heading.child(label(meta, TIMER, p.faint));
    }
    for action in actions {
        heading = heading.child(action);
    }
    col()
        .key(key)
        .role("group")
        .label(title)
        .px(8.0)
        .pt(8.0)
        .pb(10.0)
        .gap(2.0)
        .child(heading)
}

pub(super) fn quiet_note(text: impl Into<String>, p: Palette) -> El {
    label(text, META, p.muted).wrap().px(10.0).py(4.0)
}

/// One cell of the Brief header's pennant, in points.

/// The Brief: a captain's standing instructions to Shina, beside its home conversation.
pub(super) fn brief_panel(s: &State, p: Palette) -> El {
    let Some(captain) = brief_captain(s) else {
        return el();
    };
    let access = s.captain_access(captain);
    // A retired record opened from the Archive: read-only throughout, with no crew,
    // workspace or routine controls and no private document edits.
    let record = s.brief.record.is_some();
    let profile = &s.captains[captain];
    let header = row()
        .key("captain-brief-header")
        .h(36.0)
        .px(8.0)
        .center_y()
        .gap(4.0)
        .child(
            row()
                .center_y()
                .gap(6.0)
                .px(8.0)
                .h(25.0)
                .radius(6.0)
                .fill(p.selected)
                .child(icon("list", p.muted))
                .child(label(
                    if record { "Captain record" } else { "Brief" },
                    SMALL,
                    p.ink,
                )),
        )
        .child(spacer())
        .child(icon_button(
            "close-brief",
            "Hide the captain's brief",
            "panel",
            p,
            close_brief,
        ));
    let mut role = profile.role.trim().to_string();
    if role.is_empty() {
        role = "Captain".into();
    }
    let identity = row()
        .key("brief-identity")
        .px(16.0)
        .pt(6.0)
        .pb(12.0)
        .gap(12.0)
        .center_y()
        .child(avatar(
            s,
            p,
            captain,
            36.0,
            if record {
                crate::model::Status::Ready
            } else {
                shown_status(s)
            },
        ))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            label(&profile.name, CAPTAIN_SERIF, p.ink)
                                .truncate()
                                .shrink()
                                .min_w(0.0),
                        )
                        // A small swatch in the captain's tile colour, so the Brief carries
                        // the same colour as the captain's portrait backdrop.
                        .child(
                            el().key("brief-pennant")
                                .size(8.0, 8.0)
                                .radius(4.0)
                                .fill(super::rgb_bytes(crate::visuals::portrait_backdrop(
                                    profile.portrait,
                                )))
                                .a11y_hidden(),
                        ),
                )
                .child(label(role, META, p.muted).truncate()),
        );
    let mut body = col().key("brief-sections").pb(8.0);
    if let Some(error) = &s.brief.error {
        body = body.child(
            row()
                .key("brief-error")
                .role("alert")
                .mx(16.0)
                .mb(8.0)
                .px(10.0)
                .py(8.0)
                .gap(8.0)
                .radius(CARD_RADIUS)
                .fill(p.danger.alpha(0.08))
                .child(icon("alert", p.danger))
                .child(label(error, META, p.danger).wrap().grow(1.0).min_w(0.0))
                .child(icon_action(
                    s,
                    "dismiss-brief-error".into(),
                    "Dismiss",
                    icon("close", p.danger),
                    p,
                    |s| s.brief.error = None,
                )),
        );
    }
    // A host without captain records or the protocol cannot know the new sections, so
    // they stay hidden rather than drawing empty state.
    let knowable = !matches!(
        access,
        crate::model::CaptainAccess::UpdateRequired | crate::model::CaptainAccess::NoRecords
    );
    let sections = [
        knowable.then(|| digest_section(s, p, captain)),
        (knowable && !record).then(|| crew_section(s, p, captain)),
        knowable.then(|| documents_section(s, p, captain)),
        (!record).then(|| {
            scope_section(s, p, captain).disabled(access != crate::model::CaptainAccess::Full)
        }),
        Some(memory_section(s, p, captain)),
        (!record).then(|| {
            routines_section(s, p, captain).disabled(access != crate::model::CaptainAccess::Full)
        }),
        Some(earlier_section(s, p, captain)),
        super::captain_lifecycle_ui::section_visible(s, captain)
            .then(|| super::captain_lifecycle_ui::section(s, p, captain)),
    ];
    for section in sections.into_iter().flatten() {
        body = body
            .child(el().h(1.0).mx(16.0).fill(hairline(p)))
            .child(section);
    }
    col()
        .key("captain-brief")
        .role("complementary")
        .label(format!("{}'s brief", profile.name))
        .min_h(0.0)
        .h_full()
        .child(header)
        // The same closing rule as the chat header, so one line runs across the pane.
        .child(rule(p).key("brief-header-rule"))
        .children(brief_notice(s, p, captain, access))
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(identity)
                .child(body),
        )
}

/// The state line under the Brief's header rule: the record and archived notices, else the
/// shared access reason. The archived notice carries the inline Unarchive action; every
/// other phase reads as it did before.
fn brief_notice(
    s: &State,
    p: Palette,
    captain: usize,
    access: crate::model::CaptainAccess,
) -> Vec<El> {
    let server = s.machines.selected.clone();
    if s.brief.record.is_some() {
        return vec![
            label(
                "Demoted to a thread. This record is read-only.",
                META,
                p.muted,
            )
            .key("brief-captain-state-notice")
            .wrap()
            .px(16.0)
            .py(6.0),
        ];
    }
    if s.captain_phase(captain) == crate::model::CaptainPhase::Archived {
        let reason = s.captain_lifecycle_unavailable(
            &server,
            captain,
            crate::model::CaptainLifecycleAction::Unarchive,
        );
        let expected = server.clone();
        let unarchive = ghost_button(s, "brief-unarchive".into(), "Unarchive", p, move |s| {
            if s.machines.selected != expected {
                return;
            }
            if let Err(error) = s.open_captain_lifecycle(
                &expected,
                captain,
                crate::model::CaptainLifecycleAction::Unarchive,
            ) {
                s.brief.error = Some(error);
            }
        })
        .disabled(reason.is_some());
        let unarchive = match reason {
            Some(reason) => unarchive.help(reason),
            None => unarchive,
        };
        return vec![
            row()
                .key("brief-captain-state-notice")
                .px(16.0)
                .py(6.0)
                .gap(8.0)
                .center_y()
                .child(
                    label(
                        "Archived. Unarchive the captain to change it.",
                        META,
                        p.muted,
                    )
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
                )
                .child(unarchive),
        ];
    }
    super::crew_membership_ui::access_reason(access)
        .map(|reason| {
            label(reason, META, p.muted)
                .key("brief-captain-state-notice")
                .wrap()
                .px(16.0)
                .py(6.0)
        })
        .into_iter()
        .collect()
}

/// The Brief beside the chat on a wide pane. A retired record always draws as a sheet over
/// the chat instead: there is no home conversation for it to sit beside.
pub(super) fn brief_side(s: &State, p: Palette) -> El {
    if s.brief.record.is_some() {
        let width = (s.viewport_width - 48.0).clamp(280.0, 360.0);
        return row()
            .key("brief-sheet")
            .instant_changes()
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .justify_end()
            .px(8.0)
            .pb(8.0)
            .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.22 }))
            .modal(false, close_brief)
            .on_press(close_brief)
            .focusable(false)
            .child(
                brief_panel(s, p)
                    .w(width)
                    .fill(p.panel)
                    .radius(12.0)
                    .border(1.0, p.frame)
                    .box_shadow(soft_shadow(p))
                    .on_press(|_: &mut State| {})
                    .transition(Transition::offset(16.0, 0.0).combined(Transition::opacity())),
            );
    }
    brief_panel(s, p).w(PANEL_WIDTH).min_w(PANEL_WIDTH)
}

/// On a narrow pane the Brief slides over the chat as a sheet, as KSG's `ui/sheet.tsx` on the
/// right: a dimmed backdrop that closes it, and the panel inset 8 px with a 12 px radius.
/// `top` keeps the title strip and its window buttons clear.
pub(super) fn brief_sheet(s: &State, p: Palette, top: f32) -> El {
    let width = (s.viewport_width - 48.0).clamp(280.0, 360.0);
    row()
        .key("brief-sheet")
        .instant_changes()
        .pin(Some(0.0), Some(top), Some(0.0), Some(0.0))
        .justify_end()
        .px(8.0)
        .pb(8.0)
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.22 }))
        .modal(false, close_brief)
        .on_press(close_brief)
        // Escape closes it from the keyboard, so the scrim is no Tab stop.
        .focusable(false)
        .child(
            brief_panel(s, p)
                .w(width)
                .fill(p.panel)
                .radius(12.0)
                .border(1.0, p.frame)
                .box_shadow(soft_shadow(p))
                .on_press(|_: &mut State| {})
                .transition(Transition::offset(16.0, 0.0).combined(Transition::opacity())),
        )
}

/// Workspace chips, added and removed through the workspace picker.
fn scope_section(s: &State, p: Palette, captain: usize) -> El {
    let scope = s.captain_scope(captain);
    let remote_scope = s.remote_captain_scope(captain);
    let add = icon_action(
        s,
        "brief-scope-add".into(),
        "Add a workspace to this captain's scope",
        icon("plus", p.muted),
        p,
        |s| s.open_picker(ProjectPicker::CaptainScope),
    );
    let mut section = section(
        "brief-scope",
        "Scope",
        (!(scope.is_empty() && remote_scope.is_empty()))
            .then(|| (scope.len() + remote_scope.len()).to_string()),
        vec![add],
        p,
    );
    if scope.is_empty() && remote_scope.is_empty() {
        return section.child(quiet_note(
            "No workspace. This captain works as a general assistant in its own folder.",
            p,
        ));
    }
    let mut chips = row().px(6.0).py(2.0).gap(6.0).flex_wrap();
    for (index, id) in scope.iter().copied().enumerate() {
        let project = s.project(Some(id));
        let name = project
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Missing workspace".into());
        let path = project
            .map(|p| p.path.display().to_string())
            .unwrap_or_default();
        chips = chips.child(
            row()
                .key(format!("brief-scope-{id}"))
                .label(if index == 0 {
                    format!("{name}, primary workspace, {path}")
                } else {
                    format!("{name}, {path}")
                })
                .h(28.0)
                .pl(8.0)
                .pr(2.0)
                .gap(6.0)
                .center_y()
                .max_w(PANEL_WIDTH - 32.0)
                .radius(6.0)
                .border(1.0, hairline(p))
                .child(project_tile(s, p, project, 14.0))
                .child(
                    label(name.clone(), TextStyle::new(12.5, 500.0), p.ink)
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                )
                .child(icon_action(
                    s,
                    format!("brief-scope-remove-{id}"),
                    &format!("Remove {name} from scope"),
                    icon("close", p.muted),
                    p,
                    move |s| {
                        let mut scope = s.captain_scope(captain);
                        scope.retain(|&p| p != id);
                        let result = s.set_captain_scope(captain, scope);
                        report(s, result);
                    },
                )),
        );
    }
    for entry in remote_scope {
        let peer = s.peers.iter().find(|peer| peer.server_id == entry.server);
        let name = peer
            .and_then(|p| p.capabilities["projects"].as_array())
            .and_then(|ps| {
                ps.iter()
                    .find(|p| p["id"].as_u64() == Some(entry.project_id as u64))
            })
            .and_then(|p| p["name"].as_str())
            .unwrap_or("Missing workspace");
        let machine = peer.map(|p| p.label.as_str()).unwrap_or(&entry.server);
        let text = format!("{name}, {machine}");
        let server = entry.server.clone();
        let id = entry.project_id;
        chips = chips.child(
            row()
                .key(format!("brief-scope-{server}-{id}"))
                .label(&text)
                .h(28.0)
                .pl(8.0)
                .pr(2.0)
                .gap(6.0)
                .center_y()
                .max_w(PANEL_WIDTH - 32.0)
                .radius(6.0)
                .border(1.0, hairline(p))
                .child(
                    label(text.clone(), TextStyle::new(12.5, 500.0), p.ink)
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                )
                .child(icon_action(
                    s,
                    format!("brief-scope-remove-{server}-{id}"),
                    &format!("Remove {text} from scope"),
                    icon("close", p.muted),
                    p,
                    move |s| toggle_remote_scope(s, server.clone(), id),
                )),
        );
    }
    section = section.child(chips);
    if scope.len() > 1 {
        section = section.child(quiet_note(
            "The first workspace hosts the conversation. Tasks name the one they work in.",
            p,
        ));
    }
    section
}

/// The Digest section: the automatic crew-activity digest, a rule, then the
/// saved DIGEST.md under its own sub-heading so the document's age never reads
/// as the age of the activity above it. The preview bounds itself to six
/// lines when activity sits above it, twelve when it stands alone. The
/// document is only read on demand and the section never writes it.
fn digest_section(s: &State, p: Palette, captain: usize) -> El {
    let meta = s
        .captain_document_meta(captain)
        .into_iter()
        .find(|meta| meta.name == "DIGEST.md");
    let empty = meta.as_ref().is_none_or(|meta| meta.bytes == 0);
    let age = meta
        .as_ref()
        .filter(|meta| meta.bytes > 0)
        .and_then(|meta| relative_time(meta.updated_at / 1000));
    let expected = s.machines.selected.clone();
    let open = ghost_button(s, "brief-digest-open".into(), "Open digest", p, move |s| {
        if s.machines.selected == expected {
            open_captain_document(s, captain, "DIGEST.md");
        }
    });
    let has_activity = super::captain_activity_ui::activity_has_entries(s, captain);
    let mut section = section("brief-digest", "Digest", None, vec![open], p);
    section = section.child(super::captain_activity_ui::activity(s, p, captain));
    // The only divider inside the section: activity above, the saved digest below.
    section = section.child(
        el().key("brief-digest-rule")
            .h(1.0)
            .mx(10.0)
            .fill(hairline(p)),
    );
    let mut summary = row()
        .key("brief-digest-summary")
        .h(20.0)
        .px(10.0)
        .center_y()
        .child(meta_label("Saved digest", p.muted))
        .child(spacer());
    if let Some(age) = age {
        summary = summary.child(meta_text(age, p.faint));
    }
    section = section.child(summary);
    if empty {
        section = section.child(
            quiet_note(
                format!(
                    "No saved digest. {} can write one; Open digest shows it.",
                    s.captains
                        .get(captain)
                        .map(|c| c.name.as_str())
                        .unwrap_or("The captain")
                ),
                p,
            )
            .key("brief-digest-empty"),
        );
        return section;
    }
    let (_, _, text) = s.captain_document_body(captain, "DIGEST.md");
    let lines = if has_activity { 6 } else { 12 };
    let preview: String = text.lines().take(lines).collect::<Vec<_>>().join("\n");
    section = section.child(
        el().key("brief-digest-preview")
            .px(10.0)
            .py(4.0)
            // Bounded lines at the reading size; long wrapped lines clip at the bound.
            .max_h(lines as f32 * 16.0)
            .clip_fade_linear(0.0, 0.0, 0.0, 16.0)
            .child(super::markdown_ui::render_with_find(
                &preview,
                p,
                s.dark,
                false,
                false,
                false,
                "brief-digest-markdown",
                None,
            )),
    );
    section
}

/// Open a Captain document's tab on the captain's home panel, closing the sheet first on
/// a window too narrow to keep both visible. From an earlier conversation of that same
/// Captain the tab opens on this thread's panel so it is actually visible and read only;
/// the earlier view does not jump to the primary home.
fn open_captain_document(s: &mut State, captain: usize, name: &str) {
    if s.viewport_width < SHEET_BELOW {
        close_brief(s);
    }
    let earlier = s.captain_access(captain) == crate::model::CaptainAccess::Earlier;
    let key = if earlier {
        s.panel_key()
    } else {
        match s.captain_home(captain) {
            Some(home) => crate::right_panel::key(&s.machines.selected, home),
            None => return,
        }
    };
    s.right_panels.open_captain_document(&key, captain, name);
}

/// Planning documents, in the row style of Shina's `earlier_section` entries: a pressable
/// row per file that opens its right-panel tab. Row order and contents follow the section
/// structure of KSG's `ThreadDetailsSection`.
fn documents_section(s: &State, p: Palette, captain: usize) -> El {
    let mut rows = col().gap(2.0);
    for name in ["NOTES.md", "AGENTS.md", "CREW.md"] {
        let meta = s
            .captain_document_meta(captain)
            .into_iter()
            .find(|meta| meta.name == name)
            .unwrap_or(crate::model::CaptainDocumentMeta {
                name: name.to_string(),
                revision: 0,
                updated_at: 0,
                bytes: 0,
            });
        let title = super::captain_docs_ui::document_title(name);
        let key = crate::model::DocumentDraftKey {
            server: s.machines.selected.clone(),
            captain,
            name: name.to_string(),
        };
        let draft = s.captain_drafts.get(&key);
        let saving =
            draft.is_some_and(|d| matches!(d.state, crate::model::DraftState::Saving { .. }));
        let dirty = draft.is_some_and(|d| {
            !matches!(d.state, crate::model::DraftState::Saving { .. })
                && d.text.text != d.base_text
        });
        let age = if meta.bytes == 0 {
            "Empty".to_string()
        } else {
            relative_time(meta.updated_at / 1000).unwrap_or_default()
        };
        let glyph = if name == "AGENTS.md" { "lock" } else { "file" };
        let help = super::captain_docs_ui::sharing_note(name);
        let mut title_line = row()
            .gap(6.0)
            .center_y()
            .min_w(0.0)
            .child(label(title, ROW_TEXT, p.ink).truncate().min_w(0.0));
        if name == "AGENTS.md" {
            title_line = title_line.child(outlined(
                el().h(16.0)
                    .px(5.0)
                    .center()
                    .radius(3.0)
                    .child(label("Private", META_LABEL, p.muted)),
                p,
            ));
        }
        let mut file_line = row().gap(8.0).center_y().min_w(0.0).child(label(
            name.to_string(),
            META_LABEL,
            p.faint,
        ));
        if dirty {
            file_line = file_line.child(
                el().size(6.0, 6.0)
                    .fill(p.coral)
                    .radius(0.0)
                    .key(format!("brief-document-draft-{name}")),
            );
        }
        if saving {
            file_line = file_line.child(label("Saving", META_LABEL, p.muted));
        }
        file_line = file_line.child(
            label(age.clone(), META_LABEL, p.muted)
                .truncate()
                .min_w(0.0),
        );
        let expected = s.machines.selected.clone();
        rows = rows.child(
            row()
                .key(format!("brief-document-{name}"))
                .role("button")
                .label(match (dirty, saving) {
                    (true, _) => format!("{title}, {name}, {age}, unsaved draft"),
                    _ => format!("{title}, {name}, updated {age}"),
                })
                .help(help)
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .px(10.0)
                .min_h(44.0)
                .center_y()
                .gap(8.0)
                .radius(6.0)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| {
                    if s.machines.selected == expected {
                        open_captain_document(s, captain, name);
                    }
                })
                .child(icon(glyph, p.muted))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(title_line)
                        .child(file_line),
                ),
        );
    }
    section("brief-documents", "Plan and rules", None, vec![], p).child(rows)
}

/// The canonical crew, in the row structure of Shina's `earlier_section`: a pressable row
/// per member that opens the thread, a status mark, and a More menu with the real blockers.
/// Pending membership changes show their in-flight word; membership itself always comes
/// from the projection. Crew membership has no KSG equivalent.
fn crew_section(s: &State, p: Palette, captain: usize) -> El {
    let access = s.captain_access(captain);
    let full = access == crate::model::CaptainAccess::Full;
    if access == crate::model::CaptainAccess::NoRecords {
        return section("brief-crew", "Crew", None, vec![], p)
            .child(quiet_note(format!("Loading from {}.", machine_label(s)), p));
    }
    let mut rows = col().gap(2.0);
    let mut members = s.captain_crew(captain);
    // Pending joins on this server appear as crew rows without touching canonical
    // membership; pending moves/releases keep their rows too.
    let pending: Vec<(usize, crate::model::CrewKind)> = s
        .crew_pending
        .iter()
        .filter(|((server, _), change)| *server == s.machines.selected && change.captain == captain)
        .map(|(_, change)| (change.thread, change.kind))
        .collect();
    for (thread, kind) in &pending {
        if matches!(kind, crate::model::CrewKind::Join) && !members.contains(thread) {
            members.push(*thread);
        }
    }
    members.sort_by(|a, b| {
        let rank = |id: &usize| {
            s.threads.iter().find(|t| t.id == *id).map_or(3, |thread| {
                if thread.request.is_some() {
                    0
                } else if thread.status.busy() {
                    1
                } else {
                    2
                }
            })
        };
        rank(a).cmp(&rank(b)).then_with(|| {
            let ua = s
                .threads
                .iter()
                .find(|t| t.id == *a)
                .map_or(0, |t| t.updated);
            let ub = s
                .threads
                .iter()
                .find(|t| t.id == *b)
                .map_or(0, |t| t.updated);
            ub.cmp(&ua)
        })
    });
    members.dedup();
    let machines = s.peers.len() > 0;
    for id in members.clone() {
        let Some(index) = s.threads.iter().position(|t| t.id == id) else {
            // The source thread can vanish (remote reset, archive) while its join is
            // still pending; keep the command for its receipt and show a quiet row.
            if pending.iter().any(|(thread, _)| *thread == id) {
                rows = rows.child(
                    label("Joining, but the thread is no longer here.", META, p.faint)
                        .key(format!("brief-crew-{id}"))
                        .px(10.0)
                        .min_h(28.0),
                );
            }
            continue;
        };
        let thread = &s.threads[index];
        let title = if thread.title.is_empty() {
            "Untitled".to_string()
        } else {
            thread.title.clone()
        };
        let pending_word = pending
            .iter()
            .find(|(thread, _)| *thread == id)
            .map(|(_, kind)| match kind {
                crate::model::CrewKind::Join => "Joining",
                crate::model::CrewKind::Release => "Leaving",
                crate::model::CrewKind::Move { .. } => "Moving",
            });
        let workspace = thread
            .project
            .and_then(|project| s.project(Some(project)))
            .map(|project| project.name.clone())
            .unwrap_or_default();
        let mut meta = row().gap(8.0).center_y();
        if let Some(word) = pending_word {
            meta = meta.child(label(word, META_LABEL, p.muted));
        } else {
            meta = meta.child(label(
                status_word(thread.status),
                META_LABEL,
                status_ink(thread.status, p),
            ));
        }
        meta = meta.child(label(workspace, META, p.muted).truncate().min_w(0.0));
        if machines && thread.machine != "local" {
            meta = meta.child(label(thread.machine.clone(), META, p.faint));
        }
        let more_key = format!("brief-crew-more-{id}");
        let open_server = s.machines.selected.clone();
        let more_server = open_server.clone();
        let mut row_ = row()
            .key(format!("brief-crew-{id}"))
            .role("button")
            .label(format!("{title}, {}", status_word(thread.status)))
            .cursor(rune::Cursor::Pointer)
            .focusable(true)
            .hover_group()
            .px(10.0)
            .min_h(44.0)
            .center_y()
            .gap(8.0)
            .radius(6.0)
            .hover(hover_wash(s, p))
            .press(active_wash(s, p))
            .hover_motion(motion(), motion())
            .on_press(move |s: &mut State| {
                if s.machines.selected != open_server {
                    return;
                }
                if s.viewport_width < SHEET_BELOW {
                    close_brief(s);
                }
                if let Some(index) = s.threads.iter().position(|t| t.id == id) {
                    s.select_thread(index);
                }
            })
            .child(super::crew_ui::mark_slot(thread.status, p))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(label(title.clone(), ROW_TEXT, p.ink).truncate())
                    .child(el().px(14.0).child(meta)),
            );
        if let Some(_) = pending_word {
            row_ = row_.child(
                icon_action(
                    s,
                    more_key,
                    &format!("Waiting for {} to confirm.", machine_label(s)),
                    icon("more", p.faint),
                    p,
                    |_| {},
                )
                .disabled(true),
            );
        } else if full {
            row_ = row_.child(
                icon_action(
                    s,
                    more_key,
                    &format!("Crew actions for {title}"),
                    icon("more", p.muted),
                    p,
                    move |s| {
                        if s.machines.selected == more_server {
                            s.brief.more = Some(id);
                        }
                    },
                )
                .group_opacity(0.0, 1.0),
            );
        }
        rows = rows.child(row_);
    }
    if members.is_empty() {
        rows = rows.child(quiet_note(
            if s.captain_scope(captain).is_empty() && s.remote_captain_scope(captain).is_empty() {
                "No crew yet. Recruit an idle thread that has no workspace."
            } else {
                "No crew yet. Recruit an idle thread from this captain's workspaces."
            },
            p,
        ));
    }
    if let Some(error) = s.crew_errors.get(&s.machines.selected) {
        rows = rows.child(
            quiet_note(error.clone(), p)
                .key("brief-crew-error")
                .color(p.danger),
        );
    }
    let draft_server = s.machines.selected.clone();
    let draft_reason = s.crew_creation_unavailable(&draft_server, captain);
    let new_thread = ghost_button(s, "brief-crew-new".into(), "New crew thread", p, move |s| {
        if s.machines.selected != draft_server {
            return;
        }
        match s.begin_crew_draft(&draft_server, captain) {
            Ok(()) => {
                if s.viewport_width < SHEET_BELOW {
                    close_brief(s);
                }
            }
            Err(error) => s.brief.error = Some(error),
        }
    })
    .disabled(draft_reason.is_some());
    let new_thread = match draft_reason {
        Some(reason) => new_thread.help(reason),
        None => new_thread,
    };
    let recruit_server = s.machines.selected.clone();
    let actions = if full {
        vec![
            new_thread,
            ghost_button(s, "brief-crew-recruit".into(), "Recruit...", p, move |s| {
                if s.machines.selected != recruit_server
                    || s.captain_access(captain) != crate::model::CaptainAccess::Full
                {
                    return;
                }
                s.brief.recruit = Some(crate::model::RecruitPicker {
                    server: recruit_server.clone(),
                    captain,
                    query: rune::TextEdit::default(),
                });
            }),
        ]
    } else {
        vec![]
    };
    section(
        "brief-crew",
        "Crew",
        Some(members.len().to_string()),
        actions,
        p,
    )
    .child(rows)
    .child(super::captain_activity_ui::policy_block(s, p, captain))
    .child(super::captain_activity_ui::recovery_block(s, p, captain))
}

fn machine_label(s: &State) -> String {
    if s.machines.selected == "local" {
        return "this machine".into();
    }
    s.peers
        .iter()
        .find(|peer| peer.server_id == s.machines.selected)
        .map(|peer| peer.label.clone())
        .unwrap_or_else(|| s.machines.selected.clone())
}

/// Bytes the essential records take in context, and how many do not fit. memory.rs keeps
/// whole records in stored order and skips any that would cross the budget.
fn essential_usage(memories: &[MemoryView]) -> (usize, usize) {
    let mut used = ESSENTIAL_LABEL.len();
    let mut skipped = 0;
    for memory in memories.iter().filter(|m| m.essential) {
        let line = serde_json::to_string(memory).map_or(0, |line| line.len() + 1);
        if used + line <= ESSENTIAL_BUDGET {
            used += line;
        } else {
            skipped += 1;
        }
    }
    (used, skipped)
}

fn kib(bytes: usize) -> String {
    format!("{:.1}", bytes as f32 / 1024.0)
}

/// How much of the essential budget is in use, as KSG's `ContextWindowMeter` bar.
fn budget_meter(memories: &[MemoryView], p: Palette) -> El {
    let (used, skipped) = essential_usage(memories);
    let fraction = (used as f32 / ESSENTIAL_BUDGET as f32).clamp(0.0, 1.0);
    let ink = if skipped > 0 {
        p.danger
    } else if fraction > 0.85 {
        p.brass
    } else {
        p.sea
    };
    let summary = format!("{} of 8 KiB", kib(used));
    let mut meter = col()
        .key("memory-budget")
        .role("meter")
        .label(format!("Essential memory uses {summary} of context"))
        .px(10.0)
        .pt(2.0)
        .pb(6.0)
        .gap(6.0)
        .child(
            row()
                .center_y()
                .child(label("Essential, always in context", META, p.muted))
                .child(spacer())
                .child(label(summary, META.features(TABULAR), p.muted)),
        )
        .child(
            row()
                .h(4.0)
                .w_full()
                .radius(4.0)
                .clip_rounded(4.0)
                .fill(p.ink.alpha(0.08))
                .child(el().h(4.0).grow(fraction).min_w(0.0).fill(ink))
                .child(el().h(4.0).grow(1.0 - fraction).min_w(0.0)),
        );
    if skipped > 0 {
        meter = meter.child(
            label(
                format!(
                    "{skipped} essential {} past the limit and left out of context.",
                    if skipped == 1 {
                        "memory is"
                    } else {
                        "memories are"
                    }
                ),
                META,
                p.danger,
            )
            .wrap(),
        );
    }
    meter
}

/// One quiet line about learned memory (Honcho). Nothing shows while it is disabled.
fn memory_backend_line(s: &State) -> Option<String> {
    use crate::model::MemoryBackendStatus as M;
    let queued = |n: usize| {
        if n == 0 {
            String::new()
        } else {
            format!(", {n} waiting to sync")
        }
    };
    match s.memory_backend_status() {
        M::Disabled => None,
        M::Connected {
            queued_count,
            deriver_pending,
        } => Some(format!(
            "Learned memory: Honcho connected{}{}",
            match deriver_pending {
                Some(n) if n > 0 => format!(", learning from {n} new messages"),
                _ => String::new(),
            },
            queued(queued_count)
        )),
        M::Offline {
            last_error,
            queued_count,
        } => Some(format!(
            "Learned memory offline{}. {last_error}",
            queued(queued_count)
        )),
    }
}

fn memory_input(p: Palette, edit: &rune::TextEdit, key: String, title: &str) -> El {
    text_input(edit, ROW_TEXT)
        .key(key)
        .label(title)
        .placeholder("Something this captain should always know", p.muted)
        .color(p.ink)
        .w_full()
        .min_h(56.0)
        .max_h(140.0)
        .px(10.0)
        .py(8.0)
        .radius(12.0)
        .fill(p.card)
        .field_focus(edit, p)
}

fn memory_editor(s: &State, p: Palette, captain: usize, id: Option<String>) -> El {
    let (edit, key, title) = match &id {
        Some(id) => (&s.brief.edit, format!("memory-input-{id}"), "Edit memory"),
        None => (
            &s.brief.draft,
            "brief-memory-input".to_string(),
            "New memory",
        ),
    };
    let editor_key = format!("{key}-editor");
    let count = edit.text.chars().count();
    let empty = edit.text.trim().is_empty();
    let save_id = id.clone();
    let save = move |s: &mut State| {
        let ok = match &save_id {
            Some(id) => {
                let text = s.brief.edit.text.clone();
                let result = s.edit_memory(captain, id, text);
                report(s, result)
            }
            None => {
                let text = s.brief.draft.text.clone();
                let result = s.add_memory(captain, text, false);
                report(s, result)
            }
        };
        if ok {
            s.brief.editing = None;
            s.brief.adding = false;
            s.brief.edit = rune::TextEdit::default();
            s.brief.draft = rune::TextEdit::default();
        }
    };
    let submit = save.clone();
    let field = memory_input(p, edit, key, title)
        .on_edit(move |s: &mut State| {
            if id.is_some() {
                &mut s.brief.edit
            } else {
                &mut s.brief.draft
            }
        })
        .on_submit(move |s: &mut State, _| submit(s));
    let cancel = |s: &mut State| {
        s.brief.editing = None;
        s.brief.adding = false;
    };
    col()
        .key(editor_key)
        .px(2.0)
        .py(4.0)
        .gap(6.0)
        .child(field)
        .child(
            row()
                .gap(6.0)
                .center_y()
                .child(label(
                    format!("{count} / 500"),
                    META.features(TABULAR),
                    if count > 500 { p.danger } else { p.muted },
                ))
                .child(spacer())
                .child(ghost_button(s, "cancel-memory".into(), "Cancel", p, cancel))
                .child(
                    solid_button("save-memory".into(), "Save", p, save)
                        .disabled(empty || count > 500)
                        .opacity(if empty || count > 500 { 0.5 } else { 1.0 }),
                ),
        )
}

/// One memory, as KSG's details rows: an essential pin, the text, a lock badge for entries the
/// user controls, then edit and delete.
fn memory_row(s: &State, p: Palette, captain: usize, memory: &MemoryView) -> El {
    let record =
        s.brief.record.is_some() || s.captain_access(captain) != crate::model::CaptainAccess::Full;
    if !record && s.brief.editing.as_deref() == Some(memory.id.as_str()) {
        return memory_editor(s, p, captain, Some(memory.id.clone()));
    }
    let id = memory.id.clone();
    let essential = memory.essential;
    let pin = {
        let id = id.clone();
        el().key(format!("memory-pin-{id}"))
            .role("switch")
            .checked(essential)
            .label(if essential {
                "Essential, always in context. Press to keep it searchable only."
            } else {
                "Mark essential, always in context"
            })
            .cursor(rune::Cursor::Pointer)
            .focusable(true)
            .size(24.0, 24.0)
            .center()
            .radius(6.0)
            .hover(hover_wash(s, p))
            .press(active_wash(s, p))
            .on_press(move |s: &mut State| {
                let result = s.set_memory_essential(captain, &id, !essential);
                report(s, result);
            })
            .child(glyph_icon(
                if essential {
                    &super::icons::PIN_ON
                } else {
                    &super::icons::PIN
                },
                if essential { p.sea } else { p.faint },
            ))
    };
    let mut text = col()
        .key(format!("memory-text-{id}"))
        .grow(1.0)
        .min_w(0.0)
        .gap(3.0)
        .pt(2.0)
        .child(label(&memory.text, ROW_TEXT, p.ink).wrap().selectable(true));
    if memory.user_controlled {
        text = text.child(
            row().child(
                row()
                    .key(format!("memory-lock-{id}"))
                    .label("You control this memory. The captain cannot change or forget it.")
                    .h(18.0)
                    .px(6.0)
                    .gap(4.0)
                    .center_y()
                    .radius(6.0)
                    .fill(p.ink.alpha(if s.dark { 0.06 } else { 0.05 }))
                    .child(
                        rune::icon(&super::icons::LOCK, super::ICON)
                            .color(p.muted)
                            .a11y_hidden(),
                    )
                    .child(meta_label("Yours", p.muted)),
            ),
        );
    }
    let mut row_ = row()
        .key(format!("memory-{id}"))
        .px(4.0)
        .py(4.0)
        .gap(6.0)
        .radius(ROW_RADIUS)
        .hover(hover_wash(s, p))
        .hover_motion(motion(), motion())
        // The edit and delete icons show on hover and when keyboard focus is in the row.
        .hover_group_focus();
    if !record {
        row_ = row_.child(pin);
    }
    row_ = row_.child(text);
    if record {
        return row_;
    }
    let edit_id = id.clone();
    let text_value = memory.text.clone();
    let wash = hover_wash(s, p);
    let backing = p.panel.mix(wash, wash.a);
    row_.child(
        row()
            .key(format!("memory-actions-{id}"))
            .pin(None, Some(4.0), Some(4.0), None)
            .gap(2.0)
            .px(2.0)
            .radius(ROW_RADIUS)
            .fill(backing)
            .child(
                icon_action(
                    s,
                    format!("memory-edit-{id}"),
                    "Edit memory",
                    icon("edit", p.muted),
                    p,
                    |_| {},
                )
                .on_press_tx(move |s: &mut State, tx| {
                    s.brief.adding = false;
                    s.brief.editing = Some(edit_id.clone());
                    s.brief.edit = rune::TextEdit::default();
                    s.brief.edit.replace_text(&text_value);
                    s.brief.edit.focused = true;
                    tx.scroll_into_view(format!("memory-input-{edit_id}-editor"));
                }),
            )
            .child(icon_action(
                s,
                format!("memory-delete-{id}"),
                "Delete memory",
                glyph_icon(&super::icons::TRASH, p.muted),
                p,
                move |s| {
                    let result = s.delete_memory(captain, &id);
                    report(s, result);
                },
            ))
            .group_opacity(0.0, 1.0),
    )
}

fn memory_section(s: &State, p: Palette, captain: usize) -> El {
    let memories = s.memories(captain);
    let record =
        s.brief.record.is_some() || s.captain_access(captain) != crate::model::CaptainAccess::Full;
    let add = icon_action(
        s,
        "brief-memory-add".into(),
        "Add memory",
        icon("plus", p.muted),
        p,
        |_| {},
    )
    .on_press_tx(|s: &mut State, tx| {
        s.brief.editing = None;
        s.brief.adding = true;
        s.brief.draft = rune::TextEdit::default();
        s.brief.draft.focused = true;
        tx.scroll_into_view("brief-memory-input-editor");
    });
    let mut section = section(
        "brief-memory",
        "Memory",
        (!memories.is_empty()).then(|| memories.len().to_string()),
        if record { vec![] } else { vec![add] },
        p,
    );
    if let Some(status) = memory_backend_line(s) {
        section = section.child(quiet_note(status, p));
    }
    section = section.child(budget_meter(&memories, p));
    if s.brief.adding && !record {
        section = section.child(memory_editor(s, p, captain, None));
    }
    if memories.is_empty() && (record || !s.brief.adding) {
        section = section.child(super::empty_list(
            s,
            p,
            "brief-memory-empty",
            "Nothing yet. Add what this captain should remember, or let it remember as it works.",
        ));
    }
    // Essential memories first, each group in stored order, as they enter the context.
    for memory in memories
        .iter()
        .filter(|m| m.essential)
        .chain(memories.iter().filter(|m| !m.essential))
    {
        section = section.child(memory_row(s, p, captain, memory));
    }
    section
}

/// "Weekdays at 9:00", "Every 30 min", as KSG's `scheduleLabel`.
pub(super) fn schedule_words(schedule: &Schedule) -> String {
    let time = |at: &str| {
        let (hour, minute) = at.split_once(':').unwrap_or((at, "00"));
        format!("{}:{minute}", hour.parse::<u32>().unwrap_or(0))
    };
    match schedule {
        Schedule::Every { minutes: 60 } => "Every hour".into(),
        Schedule::Every { minutes } if minutes % 60 == 0 => format!("Every {} hours", minutes / 60),
        Schedule::Every { minutes } => format!("Every {minutes} min"),
        Schedule::Daily { at } => format!("Daily at {}", time(at)),
        Schedule::Weekdays { at } => format!("Weekdays at {}", time(at)),
    }
}

/// An upcoming run as KSG's `relativeLabel`: "in 12m", "in 3h", "in 2d".
pub(super) fn next_run_words(next_at: i64, now: i64) -> String {
    let seconds = next_at - now;
    if seconds <= 0 {
        return "due now".into();
    }
    let minutes = (seconds + 59) / 60;
    if minutes < 2 {
        return "in under a minute".into();
    }
    if minutes < 60 {
        return format!("in {minutes}m");
    }
    let hours = (minutes as f64 / 60.0).round() as i64;
    if hours < 24 {
        return format!("in {hours}h");
    }
    format!("in {}d", (hours as f64 / 24.0).round() as i64)
}

/// One routine, ported from KSG Code's `ThreadAutomationsPanel` row: a calendar mark with a
/// status dot, the name over its schedule line, then delete and a pause switch.
fn routine_row(s: &State, p: Palette, routine: &RoutineView, now: i64) -> El {
    let id = routine.id;
    let paused = routine.paused;
    let failed = {
        let status = routine.last_status.to_lowercase();
        status.contains("fail") || status.contains("interrupted") || status.contains("error")
    };
    // The corner dot says something only while the routine runs (sea) or its last run failed.
    let dot = if routine.running {
        Some(p.sea)
    } else if failed {
        Some(p.danger)
    } else {
        None
    };
    let schedule = schedule_words(&routine.schedule);
    // The schedule is Inter; its state is a pixel status word beside it, 8 px apart.
    let (state, state_ink) = if routine.running {
        ("Running now".to_string(), p.sea)
    } else if paused {
        ("Paused".to_string(), p.muted)
    } else {
        (
            format!("Next {}", next_run_words(routine.next_at, now)),
            p.muted,
        )
    };
    let line = format!("{schedule}, {state}");
    let mut calendar = el()
        .size(16.0, 16.0)
        .mt(2.0)
        .center()
        .child(glyph_icon(&super::icons::CALENDAR, p.muted));
    if let Some(dot) = dot {
        // A panel-coloured square under the dot keeps it from smearing the glyph's lines.
        calendar = calendar.child(
            el().size(8.0, 8.0)
                .fill(p.panel)
                .center()
                .pin(None, Some(-2.0), Some(-2.0), None)
                .a11y_hidden()
                .child(el().size(6.0, 6.0).fill(dot)),
        );
    }
    let wash = hover_wash(s, p);
    let backing = p.panel.mix(wash, wash.a);
    // The switch sits on the title row. The schedule and the next run take the full width of the
    // line under it, and delete shows over that line's end on hover or focus, as a memory's
    // controls do.
    row()
        .key(format!("routine-{id}"))
        .label(format!(
            "{}, {line}. Last run: {}",
            routine.name, routine.last_status
        ))
        .px(10.0)
        .py(6.0)
        .gap(10.0)
        .radius(ROW_RADIUS)
        .hover(hover_wash(s, p))
        .hover_motion(motion(), motion())
        .hover_group_focus()
        .child(calendar)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(
                            label(
                                &routine.name,
                                ROW_TEXT,
                                if paused { p.muted } else { p.ink },
                            )
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                        )
                        .child(switch(
                            s,
                            &format!("routine-pause-{id}"),
                            &if paused {
                                format!("Resume {}", routine.name)
                            } else {
                                format!("Pause {}", routine.name)
                            },
                            !paused,
                            p,
                            move |s| {
                                let result = s.set_routine_paused(id, !paused);
                                report(s, result);
                            },
                        )),
                )
                .child(
                    row()
                        .key(format!("routine-meta-{id}"))
                        .gap(8.0)
                        .center_y()
                        .min_w(0.0)
                        // The schedule is never cut; the state beside it gives way.
                        .child(
                            label(schedule, META.features(TABULAR), p.muted)
                                .key(format!("routine-schedule-{id}")),
                        )
                        .child(
                            label(state, TIMER, state_ink)
                                .truncate()
                                .shrink()
                                .min_w(0.0),
                        ),
                ),
        )
        .child(
            row()
                .key(format!("routine-actions-{id}"))
                .pin(None, None, Some(8.0), Some(4.0))
                .px(2.0)
                .radius(ROW_RADIUS)
                .fill(backing)
                .child(icon_action(
                    s,
                    format!("routine-delete-{id}"),
                    &format!("Delete {}", routine.name),
                    glyph_icon(&super::icons::TRASH, p.muted),
                    p,
                    move |s| {
                        let result = s.delete_routine(id);
                        report(s, result);
                    },
                ))
                .group_opacity(0.0, 1.0),
        )
}

fn routines_section(s: &State, p: Palette, captain: usize) -> El {
    let routines = s.routines(captain);
    let mut section = section(
        "brief-routines",
        "Routines",
        (!routines.is_empty()).then(|| routines.len().to_string()),
        vec![],
        p,
    );
    if routines.is_empty() {
        return section.child(quiet_note(
            "No routines. Ask this captain to check in on a schedule, such as every weekday morning.",
            p,
        ));
    }
    let now = chrono::Utc::now().timestamp();
    for routine in &routines {
        section = section.child(routine_row(s, p, routine, now));
    }
    section
}

fn earlier_section(s: &State, p: Palette, captain: usize) -> El {
    let earlier = s.earlier_conversations(captain);
    let mut section = section(
        "brief-earlier",
        "Earlier conversations",
        (!earlier.is_empty()).then(|| earlier.len().to_string()),
        vec![],
        p,
    );
    if earlier.is_empty() {
        return section.child(quiet_note(
            "A reset keeps the current conversation here, read-only.",
            p,
        ));
    }
    for id in earlier {
        let Some(thread) = s.threads.iter().find(|t| t.id == id) else {
            continue;
        };
        let open = s.current().id == id;
        let mut item = row()
            .key(format!("earlier-{id}"))
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("Open {} read-only", thread.title))
            .focusable(true)
            .h(32.0)
            .px(10.0)
            .gap(10.0)
            .center_y()
            .radius(ROW_RADIUS)
            .fill(if open {
                active_wash(s, p)
            } else {
                Color::TRANSPARENT
            })
            .hover(hover_wash(s, p))
            .press(active_wash(s, p))
            .hover_motion(motion(), motion())
            .on_press(move |s: &mut State| {
                if let Some(index) = s.threads.iter().position(|t| t.id == id) {
                    s.select_thread(index);
                }
                // A sheet covers the conversation it just opened; step out of the way.
                if s.viewport_width < SHEET_BELOW {
                    close_brief(s);
                }
            })
            .child(icon("thread", p.muted))
            .child(
                label(&thread.title, ROW_TEXT, p.ink)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if let Some(age) = relative_time(thread.updated) {
            item = item.child(label(age, META.features(TABULAR), p.muted));
        }
        section = section.child(item);
    }
    section
}

// The old reset-only "Conversation" section is replaced by
// `captain_lifecycle_ui::section`, which keeps `brief-conversation`/`brief-reset` and adds
// archive, demote and delete rows.

/// The reset confirmation, as KSG's `ui/alert-dialog.tsx`: a title, what happens, and what is
/// kept, with Cancel before the action.
fn reset_dialog(s: &State, p: Palette) -> Option<El> {
    let captain = brief_captain(s).filter(|_| s.brief.confirm_reset)?;
    let name = s.captains[captain].name.clone();
    let close = |s: &mut State| s.brief.confirm_reset = false;
    let card = col()
        .key("reset-conversation-dialog")
        .role("alertdialog")
        .label("Reset conversation")
        .w(400.0)
        .max_w(s.viewport_width - 32.0)
        .padding(20.0)
        .gap(10.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(label(
            format!("Reset the conversation with {name}?"),
            super::DIALOG_TITLE,
            p.ink,
        ).wrap())
        .child(
            label(
                "Memory, scope, routines and tasks are kept. The current conversation moves to Earlier conversations, where you can still read it.",
                BODY,
                p.muted,
            )
            .wrap(),
        )
        .child(
            row()
                .pt(8.0)
                .gap(8.0)
                .child(spacer())
                .child(secondary_button(
                    "cancel-reset-conversation",
                    "Cancel",
                    p,
                    close,
                ))
                .child(primary_button(
                    "confirm-reset-conversation",
                    "Reset conversation",
                    p,
                    move |s| {
                        s.brief.confirm_reset = false;
                        let result = s.reset_captain_conversation(captain);
                        if report(s, result)
                            && let Some(index) = s
                                .captain_home(captain)
                                .and_then(|home| s.threads.iter().position(|t| t.id == home))
                        {
                            s.select_thread(index);
                        }
                    },
                )),
        );
    Some(
        el().key("reset-conversation-backdrop")
            .instant_changes()
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .padding(16.0)
            .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.28 }))
            .modal(false, close)
            .child(card),
    )
}

/// Where an earlier conversation's composer would be: it opens read-only, with a way back.
pub(super) fn earlier_bar(s: &State, p: Palette, wide: bool, pane_width: f32) -> El {
    let captain = brief_captain(s);
    let home = captain.and_then(|c| s.captain_home(c));
    let bar = row()
        .key("earlier-bar")
        .role("status")
        .label("Earlier conversation, read-only")
        .w(lane_width(pane_width, wide))
        .min_h(44.0)
        .px(14.0)
        .py(8.0)
        .gap(10.0)
        .center_y()
        .radius(CARD_RADIUS)
        .border(1.0, hairline(p))
        .child(
            rune::icon(&super::icons::LOCK, super::ICON)
                .color(p.muted)
                .a11y_hidden(),
        )
        .child(
            label(
                "Earlier conversation. Read-only.",
                TextStyle::new(13.0, 470.0),
                p.muted,
            )
            .truncate()
            .grow(1.0)
            .min_w(0.0),
        )
        .child(solid_button(
            "open-current-home".into(),
            "Open current",
            p,
            move |s| {
                if let Some(index) =
                    home.and_then(|home| s.threads.iter().position(|t| t.id == home))
                {
                    s.select_thread(index);
                }
            },
        ));
    col()
        .key("earlier-lane")
        .w_full()
        .px(lane_gutter(pane_width))
        .pb(16.0)
        .items_center()
        .child(bar)
}

/// A quiet line in a captain's header when background mode is hiding work or routines are
/// paused while the user is away. Nothing otherwise.
/// `available` is the width the header leaves the line, from the last layout. The count never
/// shrinks: the phrase gives way first, and when the whole phrase cannot fit beside the count it
/// reads "Background".
pub(super) fn away_status(s: &State, p: Palette, available: Option<f32>) -> Option<El> {
    let captain = brief_captain(s)?;
    let status: BackgroundStatus = s
        .brief
        .demo_status
        .clone()
        .unwrap_or_else(|| s.background_status());
    // Both texts are status words, so both are pixel type. Work running in the background is
    // `sea`, as it is in the crew tray; paused routines are muted.
    // The same count as the crew line and the band's sails: this captain's threads running now.
    let working = if s.brief.demo_status.is_some() {
        status.working
    } else {
        s.crew_working_now(s.captain_home(captain).unwrap_or(s.current().id))
    };
    let (word, detail, ink) = if status.hidden_to_tray && working > 0 {
        (
            "Working in the background",
            Some(plural(working, "task", "tasks")),
            p.sea,
        )
    } else if status.routines_paused_while_away {
        ("Routines paused while you're away", None, p.muted)
    } else {
        return None;
    };
    let text = match &detail {
        Some(detail) => format!("{word}, {detail}"),
        None => word.to_string(),
    };
    let word = match (&detail, available) {
        (Some(detail), Some(available)) => {
            let width = |t: &str| rune::text::measure(t, &[], super::META, None).0;
            // The line's 4 pt padding each side, the 18 pt icon and two 8 pt gaps.
            let full = 8.0 + super::ICON + 16.0 + width(word) + width(detail);
            if full > available + 0.5 {
                "Background"
            } else {
                word
            }
        }
        _ => word,
    };
    let mut line = row()
        .key("captain-away-status")
        .role("status")
        .label(text)
        .h(20.0)
        .px(4.0)
        .gap(8.0)
        .center_y()
        .min_w(0.0)
        .shrink()
        .opacity_motion(motion())
        // The moon is for paused routines only. Work that runs on in the background gets the bolt.
        .child(icon(
            if detail.is_some() { "bolt" } else { "crescent" },
            ink,
        ))
        .child(super::meta_text(word, ink).truncate().shrink().min_w(0.0));
    if let Some(detail) = detail {
        line = line.child(super::meta_text(detail, ink).key("captain-away-count"));
    }
    Some(line)
}

/// The header control that opens the Brief, labelled because it names a captain feature.
pub(super) fn brief_button(s: &State, p: Palette) -> El {
    let on = s.show_activity;
    let press = |s: &mut State| {
        if s.show_activity {
            close_brief(s);
        } else {
            s.show_activity = true;
            s.activity_full = false;
        }
    };
    if on {
        // With the Brief open the panel names itself, so the control is the same icon-only
        // active toggle as the other header buttons.
        return toggle_button(
            "open-brief",
            "Hide the captain's brief",
            "list",
            true,
            p,
            press,
        );
    }
    row()
        .key("open-brief")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label("Show the captain's brief")
        .focusable(true)
        .h(24.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(6.0)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .on_press(press)
        .child(icon("list", p.muted))
        .child(label("Brief", TextStyle::new(12.5, 500.0), p.muted))
}

/// Add or remove a workspace from the picker in its captain-scope mode.
pub(super) fn toggle_scope(s: &mut State, project: crate::projects::ProjectId) {
    s.project_picker = None;
    let Some(captain) = brief_captain(s) else {
        return;
    };
    let mut scope = s.captain_scope(captain);
    if scope.contains(&project) {
        scope.retain(|&id| id != project);
    } else {
        scope.push(project);
    }
    let result = s.set_captain_scope(captain, scope);
    report(s, result);
}

/// The open captain's scope holds this workspace.
pub(super) fn in_scope(s: &State, project: crate::projects::ProjectId) -> bool {
    brief_captain(s).is_some_and(|captain| s.captain_scope(captain).contains(&project))
}

pub(super) fn remote_in_scope(s: &State, server: &str, project: usize) -> bool {
    brief_captain(s).is_some_and(|captain| {
        s.remote_captain_scope(captain)
            .iter()
            .any(|entry| entry.server == server && entry.project_id == project)
    })
}

pub(super) fn toggle_remote_scope(s: &mut State, server: String, project: usize) {
    s.project_picker = None;
    let Some(captain) = brief_captain(s) else {
        return;
    };
    let entry = crate::model::RemoteProject {
        server,
        project_id: project,
    };
    let mut scope = s.remote_captain_scope(captain);
    if scope.contains(&entry) {
        scope.retain(|p| p != &entry);
    } else {
        scope.push(entry);
    }
    let result = s.set_remote_captain_scope(captain, scope);
    report(s, result);
}

/// Settings rows for background work, as KSG's `SettingsRow` (`settings/settingsLayout.tsx`):
/// a `text-sm font-medium` title over a one-line `text-xs` muted description, with the
/// control at the end, and Keep awake's three choices as KSG's segmented `ToggleGroup`.
pub(super) fn background_settings(s: &State, p: Palette) -> Vec<El> {
    let settings = s.background_settings();
    let keep_working = settings.keep_working;
    let supported = cfg!(windows);
    let row_title = |title: &str| label(title, TextStyle::new(14.0, 500.0), p.ink);
    let row_note = |note: &str| label(note, TextStyle::new(13.0, 400.0).lh(18.0), p.muted).wrap();
    let working = row()
        .key("background-row")
        .py(12.0)
        .gap(12.0)
        .items_start()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(row_title("Keep working in the background"))
                .child(row_note(if supported {
                    "Hide to the tray while tasks run."
                } else {
                    "Available on Windows only."
                })),
        )
        .child(
            switch(
                s,
                "background-toggle",
                "Keep working in the background",
                keep_working,
                p,
                move |s| s.set_keep_working(!keep_working),
            )
            .disabled(!supported && !keep_working)
            .opacity(if supported || keep_working { 1.0 } else { 0.5 }),
        );
    let policy = s.settings_keep_awake();
    let note = match policy {
        KeepAwake::Off => "This machine sleeps as usual.",
        KeepAwake::WhileRunning => "This machine stays awake while tasks run.",
        KeepAwake::OnAcPower => "Awake while tasks run and plugged in.",
    };
    let mut group = row()
        .key("keep-awake-toggle")
        .role("radiogroup")
        // Its own group inside the settings menu: Tab reaches it and Left and Right choose, so
        // Up and Down through the menu never change the setting on the way past.
        .focus_group()
        .label("Keep awake")
        .w_full()
        .padding(2.0)
        .gap(2.0)
        .radius(6.0)
        .fill(p.ink.alpha(if s.dark { 0.07 } else { 0.06 }))
        .border(1.0, p.line);
    for (key, title, choice) in [
        ("keep-awake-off", "Off", KeepAwake::Off),
        (
            "keep-awake-while-running",
            "While tasks run",
            KeepAwake::WhileRunning,
        ),
        ("keep-awake-on-ac", "On AC power only", KeepAwake::OnAcPower),
    ] {
        let on = policy == choice;
        group = group.child(
            row()
                .key(key)
                .role("radio")
                .checked(on)
                .label(format!("Keep awake: {title}"))
                .cursor(rune::Cursor::Pointer)
                .focusable(true)
                .h(24.0)
                .px(8.0)
                .grow(if choice == KeepAwake::Off { 0.0 } else { 1.0 })
                .center()
                .radius(6.0)
                .fill(if on { p.card } else { Color::TRANSPARENT })
                // The chosen segment has a 1 px frame. Idle ones keep a clear border so no
                // segment changes size when the choice moves.
                .border(1.0, if on { p.frame } else { Color::TRANSPARENT })
                .hover(if on { p.card } else { hover_wash(s, p) })
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| s.set_settings_keep_awake(choice))
                .child(label(
                    match choice {
                        KeepAwake::Off => "Off",
                        KeepAwake::WhileRunning => "While working",
                        KeepAwake::OnAcPower => "AC only",
                    },
                    TextStyle::new(12.0, 500.0),
                    if on { p.ink } else { p.muted },
                )),
        );
    }
    let awake = row()
        .key("keep-awake-row")
        .py(12.0)
        .gap(12.0)
        .items_start()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(row_title("Keep awake"))
                .child(row_note(note)),
        )
        .child(group.w(244.0).min_w(244.0));
    let rule = || el().w_full().h(1.0).fill(p.line);
    let mut rows = vec![
        col().child(working).child(rule()),
        col().child(awake).child(rule()),
    ];
    if let Some(error) = s.background_status().error {
        rows.push(label(error, SMALL, p.danger).wrap().px(10.0).pb(6.0));
    }
    rows
}

/// The settings menu's entry for the profile editor.
pub(super) fn profile_item(s: &State, p: Palette) -> El {
    let profile = if s.settings_menu {
        s.settings_profile()
    } else {
        s.user_profile()
    };
    let detail = if profile.name.trim().is_empty() {
        "Add your name and preferences".to_string()
    } else {
        profile.name.trim().to_string()
    };
    row()
        .key("open-profile")
        .role("menuitem")
        .cursor(rune::Cursor::Pointer)
        .label("You: profile shared with every captain")
        .focusable(true)
        .h(44.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(12.0)
        .focus_highlight(p.selected)
        .on_press(open_profile)
        .child(
            el().size(28.0, 28.0)
                .center()
                .radius(6.0)
                .fill(p.coral)
                .border(1.0, p.frame)
                .child(super::meta_text(
                    profile
                        .name
                        .trim()
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_else(|| "Y".into()),
                    p.on_coral,
                )),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(label("You", TextStyle::new(13.0, 500.0), p.ink))
                .child(label(detail, META, p.muted).truncate()),
        )
        .child(icon("open", p.muted))
}

pub(super) fn open_profile(s: &mut State) {
    let profile = if s.settings_menu {
        s.settings_profile()
    } else {
        s.user_profile()
    };
    s.settings.profile_target = s.settings_menu.then(|| s.settings.machine.clone());
    s.settings_menu = false;
    s.brief.profile_open = true;
    s.brief.error = None;
    for (edit, value) in [
        (&mut s.brief.profile_name, profile.name),
        (&mut s.brief.profile_zone, profile.time_zone),
        (&mut s.brief.profile_preferences, profile.preferences),
    ] {
        *edit = rune::TextEdit::default();
        edit.replace_text(&value);
    }
    s.brief.profile_name.focused = true;
}

fn draft_profile(s: &State) -> UserProfile {
    UserProfile {
        name: s.brief.profile_name.text.clone(),
        time_zone: s.brief.profile_zone.text.clone(),
        preferences: s.brief.profile_preferences.text.clone(),
    }
}

/// The profile's size as memory.rs measures it: its serialized JSON.
pub(super) fn profile_bytes(profile: &UserProfile) -> usize {
    serde_json::to_vec(profile).map_or(0, |bytes| bytes.len())
}

fn close_profile(s: &mut State) {
    s.brief.profile_open = false;
}

fn save_profile(s: &mut State) {
    let profile = draft_profile(s);
    let result = if s.settings.profile_target.is_some() {
        s.save_settings_profile(profile)
    } else {
        s.set_user_profile(profile)
    };
    if report(s, result) {
        s.brief.profile_open = false;
    }
}

/// One labelled field, as a KSG settings row with its control below the title.
fn profile_field(p: Palette, title: &str, note: &str, field: El) -> El {
    col()
        .gap(6.0)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(meta_label(title, p.muted))
                .child(
                    label(note, TextStyle::new(12.0, 420.0), p.muted)
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                ),
        )
        .child(field)
}

fn profile_input(s: &State, p: Palette, edit: &rune::TextEdit, key: &str, title: &str) -> El {
    text_input(edit, TextStyle::new(14.0, 420.0).lh(20.0))
        .key(key)
        .label(title)
        .color(p.ink)
        .w_full()
        .px(10.0)
        .py(7.0)
        .radius(6.0)
        .fill(field_fill(s, p))
        .field_focus(edit, p)
}

/// "You": the global profile every captain reads, ported from KSG's settings panels
/// (`settings/settingsLayout.tsx` rows inside a `SettingsGroup` card) as an editor dialog.
/// A live count holds it to 1 KiB.
fn profile_dialog(s: &State, p: Palette) -> Option<El> {
    if !s.brief.profile_open {
        return None;
    }
    let bytes = profile_bytes(&draft_profile(s));
    let over = bytes > PROFILE_BUDGET;
    let name = profile_field(
        p,
        "Name",
        "What captains call you",
        profile_input(s, p, &s.brief.profile_name, "profile-name", "Name")
            .single_line()
            .placeholder("Your name", p.muted)
            .on_edit(|s: &mut State| &mut s.brief.profile_name)
            .on_submit(|s: &mut State, _| save_profile(s)),
    );
    let zone = profile_field(
        p,
        "Time zone",
        "For schedules and dates",
        profile_input(s, p, &s.brief.profile_zone, "profile-zone", "Time zone")
            .single_line()
            .placeholder("Europe/Berlin", p.muted)
            .on_edit(|s: &mut State| &mut s.brief.profile_zone)
            .on_submit(|s: &mut State, _| save_profile(s)),
    );
    let preferences = profile_field(
        p,
        "Preferences",
        "How you like to work",
        profile_input(
            s,
            p,
            &s.brief.profile_preferences,
            "profile-preferences",
            "Preferences",
        )
        .placeholder("Short answers first. Ask before large refactors.", p.muted)
        .min_h(96.0)
        .max_h(180.0)
        .on_edit(|s: &mut State| &mut s.brief.profile_preferences),
    );
    let fraction = (bytes as f32 / PROFILE_BUDGET as f32).clamp(0.0, 1.0);
    let ink = if over {
        p.danger
    } else if fraction > 0.85 {
        p.brass
    } else {
        p.muted
    };
    let mut card = col()
        .key("profile-dialog")
        .role("dialog")
        .label("You")
        .w(460.0)
        .max_w(s.viewport_width - 32.0)
        .padding(20.0)
        .gap(16.0)
        .radius(12.0)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .gap(8.0)
                .child(
                    col()
                        .gap(4.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .child(label(
                            s.settings.profile_target.as_ref().map_or_else(
                                || "You".into(),
                                |machine| format!("You on {}", s.machine_label(machine)),
                            ),
                            super::DIALOG_TITLE,
                            p.ink,
                        ))
                        .child(
                            label(
                                "Every captain reads this with its memory. It is data about you, not instructions.",
                                TextStyle::new(12.5, 420.0).lh(18.0),
                                p.muted,
                            )
                            .wrap(),
                        ),
                )
                .child(icon_button("close-profile", "Close", "close", p, close_profile)),
        )
        .child(name)
        .child(zone)
        .child(preferences);
    if let Some(error) = &s.brief.error {
        card = card.child(label(error, SMALL, p.danger).wrap());
    }
    card = card.child(
        row()
            .gap(8.0)
            .center_y()
            .child(
                label(format!("{bytes} / {PROFILE_BUDGET} bytes"), TIMER, ink)
                    .key("profile-count")
                    .label(if over {
                        format!(
                            "{bytes} of {PROFILE_BUDGET} bytes. Shorten the profile to save it."
                        )
                    } else {
                        format!("{bytes} of {PROFILE_BUDGET} bytes used")
                    }),
            )
            .child(spacer())
            .child(secondary_button(
                "cancel-profile",
                "Cancel",
                p,
                close_profile,
            ))
            .child(
                primary_button("save-profile", "Save", p, save_profile)
                    .disabled(over)
                    .opacity(if over { 0.5 } else { 1.0 }),
            ),
    );
    Some(modal_frame("profile-backdrop", s, close_profile, card))
}

/// Dialogs this module owns, mounted above everything else in the window.
pub(super) fn dialogs(s: &State, p: Palette) -> Vec<El> {
    let mut out: Vec<El> = [
        reset_dialog(s, p),
        profile_dialog(s, p),
        super::captain_docs_ui::discard_dialog(s, p),
    ]
    .into_iter()
    .flatten()
    .collect();
    out.extend(super::captain_lifecycle_ui::dialogs(s, p));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    const WIDTHS: [f32; 4] = [620.0, 960.0, 1280.0, 1920.0];

    fn runtime(width: f32) -> Runtime<State> {
        Runtime::with_text(
            State::new(".".into(), false),
            view,
            (width, 840.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
        )
    }
    fn press(rt: &mut Runtime<State>, key: &str) {
        rt.frame(0.25);
        let (x, y) = rt
            .find(key)
            .unwrap_or_else(|| panic!("{key} is not on screen"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
    }
    fn tap(rt: &mut Runtime<State>, key: rune::Key) {
        rt.key(rune::KeyEvent {
            key,
            mods: Default::default(),
        });
        rt.frame(0.0);
        rt.frame(1.0);
    }
    /// Scroll the Brief to its end, as a wheel over the panel does. Wheels until the
    /// scroll-top identity stops moving, so taller bodies still reach the bottom.
    fn scroll_brief(rt: &mut Runtime<State>) {
        let (panel, _) = rt.presented("captain-brief").unwrap();
        let mut prev = f32::INFINITY;
        for _ in 0..24 {
            rt.wheel(panel[0] + panel[2] / 2.0, panel[1] + 200.0, -2000.0);
            rt.frame(1.0);
            let y = rt
                .presented("brief-pennant")
                .map(|(r, _)| r[1])
                .unwrap_or(f32::INFINITY);
            if (y - prev).abs() < 0.5 {
                return;
            }
            prev = y;
        }
    }
    /// Scroll `key` until its whole box is in the Brief's viewport. A visible
    /// centre alone does not rule out clipping at the edge.
    fn reveal_in_brief(rt: &mut Runtime<State>, key: &str) {
        for _ in 0..24 {
            if fully_visible(rt, key) {
                return;
            }
            rt.update(|_, tx| tx.scroll_into_view_in("captain-brief", key.to_string()));
            rt.frame(0.5);
        }
        panic!("{key} never revealed inside the brief's scroll viewport");
    }
    /// Every corner of `key`'s drawn box is inside its ancestors' clips, so the
    /// element is fully on screen rather than cut by the scroll viewport.
    fn fully_visible(rt: &Runtime<State>, key: &str) -> bool {
        let Some((r, _)) = rt.presented(key) else {
            return false;
        };
        [
            (r[0] + 1.0, r[1] + 1.0),
            (r[0] + r[2] - 1.0, r[1] + 1.0),
            (r[0] + 1.0, r[1] + r[3] - 1.0),
            (r[0] + r[2] - 1.0, r[1] + r[3] - 1.0),
        ]
        .into_iter()
        .all(|(x, y)| rt.contains(key, x, y))
    }
    /// A press on a Brief control that may sit outside the scroll viewport.
    fn press_in_brief(rt: &mut Runtime<State>, key: &str) {
        reveal_in_brief(rt, key);
        press(rt, key);
    }
    fn inside(outer: [f32; 4], inner: [f32; 4]) -> bool {
        inner[0] >= outer[0] - 0.5
            && inner[1] >= outer[1] - 0.5
            && inner[0] + inner[2] <= outer[0] + outer[2] + 0.5
            && inner[1] + inner[3] <= outer[1] + outer[3] + 0.5
    }

    /// A store-backed captain home on a temp dir, so documents and crew really save.
    fn store_runtime(width: f32) -> Runtime<State> {
        let dir = std::env::temp_dir().join(format!(
            "shina-brief-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::new(dir, false);
        state.load_global_for_test();
        state.captains = crate::captains::Captain::presets();
        state.open_captain(0);
        Runtime::with_text(
            state,
            view,
            (width, 840.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
        )
    }

    /// A demo captain home with scope, memory and one routine, and the Brief open.
    fn brief_runtime(width: f32) -> (Runtime<State>, u64) {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        rt.state.compact_sidebar = width < 760.0;
        rt.state.open_captain(0);
        rt.state.set_captain_scope(0, vec![1, 2]).unwrap();
        rt.state
            .add_memory(0, "Prefers small pull requests.".into(), true)
            .unwrap();
        rt.state
            .add_memory(0, "The release branch is cut on Thursdays.".into(), false)
            .unwrap();
        let home = rt.state.current().id;
        rt.state.background.routines = crate::routines::Routines::load(
            crate::store::directory().unwrap().join("routines.json"),
        );
        let routine = rt
            .state
            .background
            .routines
            .create(
                0,
                home,
                "Morning review".into(),
                "Summarize overnight work".into(),
                Schedule::Weekdays { at: "09:00".into() },
                crate::routines::MissedPolicy::Skip,
                chrono::Local::now(),
            )
            .unwrap();
        rt.frame(0.0);
        press(&mut rt, "open-brief");
        rt.frame(1.0);
        (rt, routine)
    }

    #[test]
    fn brief_sections_fit_beside_or_over_the_chat_at_supported_widths() {
        for width in WIDTHS {
            let (mut rt, routine) = brief_runtime(width);
            let (panel, _) = rt.presented("captain-brief").expect("brief is shown");
            assert!(
                panel[0] >= 0.0 && panel[0] + panel[2] <= width + 0.5,
                "{width}: {panel:?}"
            );
            // Narrow windows show it as a sheet over the chat; wide ones beside it.
            assert_eq!(
                rt.presented("brief-sheet").is_some(),
                width < 1050.0,
                "{width}"
            );
            assert!(rt.presented("session-panel").is_none());
            assert!(rt.presented("composer").is_some(), "{width}");
            // The header's close control is fixed above the scroll viewport: it
            // shows, fully on screen, without scrolling.
            let (close, _) = rt.presented("close-brief").expect("close-brief");
            assert!(inside(panel, close), "{width}: {close:?} outside {panel:?}");
            assert!(fully_visible(&rt, "close-brief"), "{width}");
            // Sections below the header scroll into the viewport fully, not clipped.
            for key in [
                "brief-scope-1",
                "brief-scope-remove-2",
                "brief-scope-add",
                "memory-budget",
                "brief-memory-add",
            ] {
                reveal_in_brief(&mut rt, key);
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    inside(panel, rect),
                    "{key} at {width}: {rect:?} outside {panel:?}"
                );
                assert!(
                    fully_visible(&rt, key),
                    "{key} at {width}: {rect:?} clipped by the scroll viewport"
                );
            }
            // Rows lower in the scroll still lay out inside the panel's width. The
            // delete overlay is transparent until hover, so it scrolls to without
            // the clip check the visible rows take.
            for key in [
                format!("routine-pause-{routine}"),
                format!("routine-delete-{routine}"),
                "brief-reset".into(),
            ] {
                rt.update(|_, tx| tx.scroll_into_view_in("captain-brief", key.clone()));
                rt.frame(1.0);
                let (rect, _) = rt
                    .presented(&key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    rect[0] >= panel[0] && rect[0] + rect[2] <= panel[0] + panel[2] + 0.5,
                    "{key} at {width}"
                );
            }
        }
    }

    #[test]
    fn memory_rows_pin_edit_and_delete_through_the_state_api() {
        for width in WIDTHS {
            let (mut rt, _) = brief_runtime(width);
            scroll_brief(&mut rt);
            let memories = rt.state.memories(0);
            let release = memories.iter().find(|m| !m.essential).unwrap().id.clone();
            press(&mut rt, &format!("memory-pin-{release}"));
            assert!(rt.state.memories(0).iter().all(|m| m.essential));
            assert!(rt.presented(&format!("memory-lock-{release}")).is_some());
            press(&mut rt, &format!("memory-edit-{release}"));
            rt.state
                .brief
                .edit
                .replace_text("The release branch is cut on Fridays.");
            rt.invalidate();
            rt.frame(1.0);
            assert!(
                rt.presented("save-memory").is_some(),
                "{width}: edited memory remains visible"
            );
            press(&mut rt, "save-memory");
            assert!(
                rt.state
                    .memories(0)
                    .iter()
                    .any(|m| m.text == "The release branch is cut on Fridays.")
            );
            press(&mut rt, &format!("memory-delete-{release}"));
            assert_eq!(rt.state.memories(0).len(), 1, "{width}");
            rt.update(|_, tx| tx.scroll_into_view("brief-memory-add"));
            rt.frame(1.0);
            press(&mut rt, "brief-memory-add");
            rt.state.brief.draft.replace_text("Use British spelling.");
            rt.invalidate();
            rt.frame(1.0);
            assert!(
                rt.presented("save-memory").is_some(),
                "{width}: new memory editor remains visible"
            );
            press(&mut rt, "save-memory");
            assert_eq!(rt.state.memories(0).len(), 2, "{width}");
            assert!(rt.state.brief.error.is_none());
            assert!(rt.state.current().pi.is_none(), "no prompt starts");
        }
    }

    #[test]
    fn essential_budget_counts_records_and_reports_those_left_out() {
        let memory = |n: usize, essential| MemoryView {
            id: format!("memory-{n}"),
            text: "x".repeat(480),
            essential,
            user_controlled: true,
        };
        let few: Vec<_> = (0..3).map(|n| memory(n, true)).collect();
        let (used, skipped) = essential_usage(&few);
        assert!(used > 3 * 480 && used < ESSENTIAL_BUDGET && skipped == 0);
        let many: Vec<_> = (0..20).map(|n| memory(n, n % 5 != 0)).collect();
        let (used, skipped) = essential_usage(&many);
        assert!(used <= ESSENTIAL_BUDGET && skipped > 0);
    }

    #[test]
    fn routines_read_in_words_and_pause_resume_and_delete() {
        assert_eq!(
            schedule_words(&Schedule::Weekdays { at: "09:00".into() }),
            "Weekdays at 9:00"
        );
        assert_eq!(
            schedule_words(&Schedule::Daily { at: "00:30".into() }),
            "Daily at 0:30"
        );
        assert_eq!(
            schedule_words(&Schedule::Every { minutes: 60 }),
            "Every hour"
        );
        assert_eq!(
            schedule_words(&Schedule::Every { minutes: 120 }),
            "Every 2 hours"
        );
        assert_eq!(
            schedule_words(&Schedule::Every { minutes: 45 }),
            "Every 45 min"
        );
        assert_eq!(next_run_words(1_000 + 30, 1_000), "in under a minute");
        assert_eq!(next_run_words(1_000 + 12 * 60, 1_000), "in 12m");
        assert_eq!(next_run_words(1_000 + 3 * 3600, 1_000), "in 3h");
        assert_eq!(next_run_words(1_000 + 50 * 3600, 1_000), "in 2d");
        for width in WIDTHS {
            let (mut rt, routine) = brief_runtime(width);
            scroll_brief(&mut rt);
            press(&mut rt, &format!("routine-pause-{routine}"));
            assert!(rt.state.routines(0)[0].paused, "{width}");
            press(&mut rt, &format!("routine-pause-{routine}"));
            assert!(!rt.state.routines(0)[0].paused, "{width}");
            press(&mut rt, &format!("routine-delete-{routine}"));
            assert!(rt.state.routines(0).is_empty(), "{width}");
        }
    }

    #[test]
    fn a_switch_is_on_at_the_right_in_coral_and_off_at_the_left_in_both_themes() {
        for dark in [false, true] {
            let (mut rt, routine) = brief_runtime(1280.0);
            scroll_brief(&mut rt);
            rt.state.dark = dark;
            rt.invalidate();
            rt.frame(1.0);
            let key = format!("routine-pause-{routine}");
            let thumb_key = format!("{key}-thumb");
            let offset = |rt: &Runtime<State>| {
                let (track, _) = rt.presented(&key).unwrap();
                let (thumb, _) = rt.presented(&thumb_key).unwrap();
                thumb[0] - track[0]
            };
            // On: the routine runs, so the thumb sits at the right of its 34 px track.
            let on = offset(&rt);
            press(&mut rt, &key);
            rt.frame(1.0);
            rt.frame(2.0);
            let off = offset(&rt);
            assert!(on > off + 10.0, "dark {dark}: on {on}, off {off}");
            assert!(
                off < 8.0,
                "dark {dark}: the off thumb is at the left ({off})"
            );
        }
    }

    #[test]
    fn a_routine_row_puts_the_switch_on_the_title_row_and_the_schedule_below() {
        for width in WIDTHS {
            let (rt, routine) = brief_runtime(width);
            let (switch, _) = rt.presented(&format!("routine-pause-{routine}")).unwrap();
            let (meta, _) = rt.presented(&format!("routine-meta-{routine}")).unwrap();
            let (schedule, _) = rt
                .presented(&format!("routine-schedule-{routine}"))
                .unwrap();
            assert!(
                meta[1] >= switch[1] + switch[3] - 0.5,
                "{width}: the schedule {meta:?} shares the switch's row {switch:?}"
            );
            assert!(inside(meta, schedule), "{width}: the schedule is cut");
            // The delete button is an overlay: it takes no room in the row and shows on hover.
            let (actions, _) = rt.presented(&format!("routine-actions-{routine}")).unwrap();
            assert!(
                actions[0] + actions[2] <= switch[0] + switch[2] + 10.0,
                "{width}: {actions:?}"
            );
            assert!(rt.find(&format!("routine-delete-{routine}")).is_some());
        }
    }

    #[test]
    fn the_brief_header_has_the_same_rule_under_it_as_the_chat_header() {
        for width in [960.0, 1280.0, 1920.0] {
            let (rt, _) = brief_runtime(width);
            let (rule, _) = rt.presented("brief-header-rule").unwrap();
            let (panel, _) = rt.presented("captain-brief").unwrap();
            assert!((rule[3] - 1.0).abs() < 0.1, "{width}: {rule:?}");
            assert!(
                (rule[2] - panel[2]).abs() < 1.0 && (rule[0] - panel[0]).abs() < 1.0,
                "{width}: the rule {rule:?} spans the panel {panel:?}"
            );
            // It lines up with the chat header's rule, and the identity row sits below it.
            let (chat, _) = rt.presented("chat-header-rule").unwrap();
            assert!(
                (rule[1] - chat[1]).abs() < 1.0,
                "{width}: {rule:?} {chat:?}"
            );
            let (flag, _) = rt.presented("brief-pennant").unwrap();
            assert!(flag[1] >= rule[1] + rule[3], "{width}: {flag:?} {rule:?}");
        }
    }

    #[test]
    fn remote_scope_chips_and_picker_preserve_machine_and_project_ids() {
        for width in WIDTHS {
            let (mut rt, _) = brief_runtime(width);
            let server = uuid::Uuid::new_v4().to_string();
            rt.state.peers.push(crate::peer::Peer {
                server_id: server.clone(), host:"127.0.0.1:1".into(), label:"Studio".into(),
                linked_at:0, last_reached_at:Some(0), last_error:None,
                capabilities:json!({"projects":[{"id":9,"name":"Scratch","path":"C:\\scratch","machine":"local"}]}),
            });
            press_in_brief(&mut rt, "brief-scope-add");
            let key = format!("choose-project-{server}-9");
            assert!(rt.presented(&key).is_some(), "{width}");
            press(&mut rt, &key);
            assert_eq!(
                rt.state.remote_captain_scope(0),
                vec![crate::model::RemoteProject {
                    server: server.clone(),
                    project_id: 9,
                }]
            );
            reveal_in_brief(&mut rt, &format!("brief-scope-{server}-9"));
            assert!(
                rt.presented(&format!("brief-scope-{server}-9")).is_some(),
                "{width}"
            );
            press_in_brief(&mut rt, &format!("brief-scope-remove-{server}-9"));
            assert!(rt.state.remote_captain_scope(0).is_empty());
        }
    }

    #[test]
    fn scope_chips_remove_and_the_picker_adds_a_workspace() {
        for width in WIDTHS {
            let (mut rt, _) = brief_runtime(width);
            press_in_brief(&mut rt, "brief-scope-remove-2");
            assert_eq!(rt.state.captain_scope(0), vec![1], "{width}");
            press_in_brief(&mut rt, "brief-scope-add");
            assert_eq!(rt.state.project_picker, Some(ProjectPicker::CaptainScope));
            assert!(rt.presented("choose-no-project").is_none());
            press(&mut rt, "choose-project-3");
            assert_eq!(rt.state.captain_scope(0), vec![1, 3], "{width}");
            assert!(rt.state.project_picker.is_none());
        }
    }

    #[test]
    fn reset_confirms_then_keeps_memory_scope_and_lists_the_old_conversation() {
        for width in WIDTHS {
            let (mut rt, _) = brief_runtime(width);
            let old = rt.state.current().id;
            scroll_brief(&mut rt);
            press(&mut rt, "brief-reset");
            let (dialog, _) = rt.presented("reset-conversation-dialog").unwrap();
            assert!(
                dialog[0] >= 0.0 && dialog[0] + dialog[2] <= width + 0.5,
                "{width}"
            );
            press(&mut rt, "cancel-reset-conversation");
            assert_eq!(rt.state.current().id, old);
            press(&mut rt, "brief-reset");
            press(&mut rt, "confirm-reset-conversation");
            let fresh = rt.state.current().id;
            assert_ne!(fresh, old, "{width}");
            assert_eq!(rt.state.captain_home(0), Some(fresh));
            assert_eq!(rt.state.earlier_conversations(0), vec![old]);
            assert_eq!(rt.state.memories(0).len(), 2);
            assert_eq!(rt.state.captain_scope(0), vec![1, 2]);
            scroll_brief(&mut rt);
            // The earlier conversation opens read-only, with a way back.
            press(&mut rt, &format!("earlier-{old}"));
            assert_eq!(rt.state.current().id, old);
            rt.frame(3.0);
            assert!(rt.presented("composer").is_none(), "{width}");
            let (bar, _) = rt.presented("earlier-bar").unwrap();
            assert!(bar[0] >= 0.0 && bar[0] + bar[2] <= width + 0.5, "{width}");
            press(&mut rt, "open-current-home");
            assert_eq!(rt.state.current().id, fresh);
        }
    }

    #[test]
    fn away_status_is_quiet_and_only_shows_for_hidden_work_or_paused_routines() {
        for width in WIDTHS {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.compact_sidebar = width < 760.0;
            rt.state.open_captain(0);
            rt.frame(0.0);
            assert!(rt.presented("captain-away-status").is_none());
            rt.state.brief.demo_status = Some(BackgroundStatus {
                routines_paused_while_away: true,
                ..Default::default()
            });
            rt.frame(1.0);
            let (header, _) = rt.presented("chat-header").unwrap();
            let (line, _) = rt.presented("captain-away-status").unwrap();
            assert!(inside(header, line), "{width}: {line:?} outside {header:?}");
            let (brief, _) = rt.presented("open-brief").unwrap();
            assert!(line[0] + line[2] <= brief[0], "{width}");
        }
    }

    #[test]
    fn profile_editor_counts_bytes_and_saves_within_one_kib() {
        for width in WIDTHS {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.compact_sidebar = width < 760.0;
            rt.state.settings_menu = true;
            rt.frame(0.0);
            press(&mut rt, "open-profile");
            assert!(rt.state.brief.profile_open && !rt.state.settings_menu);
            let (dialog, _) = rt.presented("profile-dialog").unwrap();
            assert!(
                dialog[0] >= 0.0 && dialog[0] + dialog[2] <= width + 0.5,
                "{width}"
            );
            for key in [
                "profile-name",
                "profile-zone",
                "profile-preferences",
                "save-profile",
            ] {
                let (rect, _) = rt.presented(key).unwrap();
                assert!(inside(dialog, rect), "{key} at {width}");
            }
            rt.state
                .brief
                .profile_preferences
                .replace_text("x".repeat(1100));
            rt.frame(1.0);
            press(&mut rt, "save-profile");
            assert!(rt.state.brief.profile_open, "over the limit does not save");
            assert_eq!(rt.state.user_profile(), UserProfile::default());
            rt.state.brief.profile_name.replace_text("Sam");
            rt.state.brief.profile_zone.replace_text("Europe/Berlin");
            rt.state
                .brief
                .profile_preferences
                .replace_text("Short answers first.");
            press(&mut rt, "save-profile");
            assert!(!rt.state.brief.profile_open);
            assert_eq!(rt.state.user_profile().name, "Sam");
        }
    }

    #[test]
    fn background_rows_are_switches_and_a_segmented_keep_awake() {
        for width in WIDTHS {
            let mut rt = runtime(width);
            rt.state.compact_sidebar = width < 760.0;
            rt.state.settings_menu = true;
            rt.frame(0.0);
            let (menu, _) = rt.presented("settings-menu").unwrap();
            for key in [
                "open-profile",
                "background-toggle",
                "keep-awake-off",
                "keep-awake-while-running",
                "keep-awake-on-ac",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    inside(menu, rect),
                    "{key} at {width}: {rect:?} outside {menu:?}"
                );
            }
            press(&mut rt, "keep-awake-on-ac");
            assert_eq!(
                rt.state.background_settings().keep_awake,
                KeepAwake::OnAcPower
            );
            press(&mut rt, "keep-awake-while-running");
            assert_eq!(
                rt.state.background_settings().keep_awake,
                KeepAwake::WhileRunning
            );
            if cfg!(windows) {
                press(&mut rt, "background-toggle");
                assert!(rt.state.background_settings().keep_working, "{width}");
            }
        }
    }
    #[test]
    fn digest_crew_and_document_rows_stay_inside_the_brief() {
        for width in WIDTHS {
            let (mut rt, _) = brief_runtime(width);
            let (panel, _) = rt.presented("captain-brief").expect("brief is shown");
            for key in [
                "brief-digest",
                "brief-digest-open",
                "brief-crew",
                "brief-crew-recruit",
                "brief-documents",
                "brief-document-NOTES.md",
                "brief-document-AGENTS.md",
                "brief-document-CREW.md",
            ] {
                // Rows below the fold mount as the panel scrolls; every mounted row
                // must sit fully inside the Brief.
                let mut rect = None;
                for _ in 0..12 {
                    if let Some((found, _)) = rt.presented(key) {
                        rect = Some(found);
                        break;
                    }
                    scroll_brief(&mut rt);
                }
                let rect = rect.unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    inside(panel, rect),
                    "{key} at {width}: {rect:?} outside {panel:?}"
                );
            }
        }
    }

    #[test]
    fn a_document_panel_covers_the_chat_up_to_1280_and_sits_inline_at_1920() {
        // The panel is a sheet over the conversation wherever the chat column cannot
        // spare 720 px for both (620 outright, 1280 with the Brief docked).
        for width in [620.0, 1280.0] {
            let mut rt = store_runtime(width);
            rt.state.show_activity = true;
            press(&mut rt, "brief-document-NOTES.md");
            rt.frame(0.0);
            let (doc, _) = rt
                .presented("captain-doc-panel")
                .unwrap_or_else(|| panic!("document open at {width}"));
            let (main, _) = rt
                .presented("chat-main")
                .unwrap_or_else(|| panic!("chat at {width}"));
            assert!(
                doc[0] <= main[0] + 2.0 && doc[0] + doc[2] >= main[0] + main[2] - 2.0,
                "{width}: doc {doc:?} does not cover chat {main:?}"
            );
        }
        // At 960 and 1920 the column fits an inline panel and keeps its 360 px floor.
        for width in [960.0, 1920.0] {
            let mut rt = store_runtime(width);
            rt.state.show_activity = true;
            press(&mut rt, "brief-document-NOTES.md");
            rt.frame(0.0);
            let (doc, _) = rt
                .presented("captain-doc-panel")
                .unwrap_or_else(|| panic!("document open at {width}"));
            let (main, _) = rt
                .presented("chat-main")
                .unwrap_or_else(|| panic!("chat at {width}"));
            assert!(
                doc[0] >= main[0] + main[2] - 1.0,
                "{width}: doc {doc:?} should sit beside chat {main:?}"
            );
            assert!(
                main[2] >= crate::right_panel::MIN_WIDTH - 0.5,
                "{width}: chat main {main:?} narrower than the floor"
            );
        }
    }

    #[test]
    fn a_crew_member_thread_shows_the_crew_bar_with_open_and_leave() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        rt.state.set_captain_scope(0, vec![1]).unwrap();
        let thread_id = rt.state.next_id;
        rt.state.next_id += 1;
        let mut member = Thread::new(thread_id);
        member.title = "Crew member".into();
        member.project = Some(1);
        member.crew_captain = Some(0);
        rt.state.threads.push(member);
        rt.state.select_thread(rt.state.threads.len() - 1);
        rt.frame(0.25);
        let (bar, _) = rt.presented("crew-bar").expect("crew bar");
        assert!(rt.presented("crew-bar-open-captain").is_some());
        press(&mut rt, "crew-bar-more");
        rt.frame(0.25);
        for key in ["crew-menu-move", "crew-bar-leave"] {
            let (rect, _) = rt.presented(key).unwrap_or_else(|| panic!("{key}"));
            assert!(rect[1] >= bar[1], "{key} above the bar");
        }
        // Leave calls the same release path the Brief's menu uses.
        press(&mut rt, "crew-bar-leave");
        rt.frame(0.25);
        assert_eq!(rt.state.threads.last().unwrap().crew_captain, None);
        assert!(rt.presented("crew-bar").is_none());
    }

    #[test]
    fn the_recruit_palette_groups_eligible_rows_and_shows_full_reasons() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        rt.state.show_activity = true;
        rt.state.set_captain_scope(0, vec![1]).unwrap();
        let next = rt.state.next_id;
        let joinable = {
            let mut t = Thread::new(next);
            t.title = "Joinable thread".into();
            t.project = Some(1);
            t.session = Some("session".into());
            t.never_sent = false;
            rt.state.threads.push(t);
            next
        };
        let out_of_scope = {
            let mut t = Thread::new(next + 1);
            t.title = "Outside thread".into();
            t.project = Some(99);
            t.session = Some("session".into());
            t.never_sent = false;
            rt.state.threads.push(t);
            next + 1
        };
        rt.state.next_id += 2;
        press(&mut rt, "brief-crew-recruit");
        rt.frame(0.25);
        assert!(rt.presented("crew-recruit-palette").is_some());
        assert!(rt.presented("crew-recruit-can").is_some());
        assert!(rt.presented("crew-recruit-cannot").is_some());
        // The unavailable row keeps its whole reason instead of truncating to 34 px.
        let (reason, _) = rt
            .presented(&format!("crew-recruit-reason-{out_of_scope}"))
            .expect("reason line");
        assert!(reason[3] > 14.0, "reason wraps: {reason:?}");
        assert!(rt.presented(&format!("crew-recruit-{joinable}")).is_some());
        std::fs::remove_dir_all(&rt.state.data_dir).ok();
    }

    #[test]
    fn a_saving_document_does_not_render_conflict_actions_for_its_new_revision() {
        let mut rt = store_runtime(1280.0);
        rt.state.show_activity = true;
        press(&mut rt, "brief-document-NOTES.md");
        press(&mut rt, "captain-doc-edit");
        let key = crate::model::DocumentDraftKey {
            server: "local".into(),
            captain: 0,
            name: "NOTES.md".into(),
        };
        let command_id = uuid::Uuid::new_v4();
        let draft = rt.state.captain_drafts.get_mut(&key).unwrap();
        draft.text.replace_text("Pending plan");
        draft.state = crate::model::DraftState::Saving {
            command: command_id,
        };
        rt.state.seed_captain_document(
            0,
            "NOTES.md",
            crate::model::CaptainDocument {
                text: "Pending plan".into(),
                revision: 1,
                updated_at: crate::lifecycle::now(),
            },
        );
        rt.frame(1.0);
        assert!(rt.presented("captain-doc-status").is_some());
        assert!(rt.presented("captain-doc-text").is_some());
        for action in [
            "captain-doc-conflict",
            "captain-doc-conflict-theirs",
            "captain-doc-conflict-replace",
            "captain-doc-conflict-discard",
        ] {
            assert!(rt.presented(action).is_none(), "{action} while Saving");
        }
        press(&mut rt, "captain-doc-cancel");
        assert!(rt.state.doc_discard.is_none());
        assert!(matches!(
            rt.state.captain_drafts[&key].state,
            crate::model::DraftState::Saving { command } if command == command_id
        ));
    }

    #[test]
    fn editing_a_document_saves_through_the_panel_and_conflicts_keep_the_draft() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        press(&mut rt, "open-brief");
        rt.frame(1.0);
        press(&mut rt, "brief-document-NOTES.md");
        rt.frame(0.5);
        press(&mut rt, "captain-doc-edit");
        rt.frame(0.5);
        let key = crate::model::DocumentDraftKey {
            server: "local".into(),
            captain: 0,
            name: "NOTES.md".into(),
        };
        assert!(rt.state.captain_drafts.contains_key(&key));
        assert!(rt.presented("captain-doc-text").is_some());
        assert!(rt.presented("captain-doc-save").is_some());
        rt.state
            .captain_drafts
            .get_mut(&key)
            .unwrap()
            .text
            .replace_text("Ship .20 on Friday.\n");
        press(&mut rt, "captain-doc-save");
        rt.frame(0.0);
        assert!(
            !rt.state.captain_drafts.contains_key(&key),
            "local save closes"
        );
        let doc = rt.state.captain_document_value(0, "NOTES.md").unwrap();
        assert_eq!(doc["text"], "Ship .20 on Friday.\n");
        assert_eq!(doc["revision"], 1);
        press(&mut rt, "captain-doc-edit");
        rt.state
            .captain_drafts
            .get_mut(&key)
            .unwrap()
            .text
            .replace_text("Mine.");
        rt.state
            .captain_document_write(0, "NOTES.md", "Theirs.".into(), 1)
            .unwrap();
        press(&mut rt, "captain-doc-save");
        assert!(matches!(
            rt.state.captain_drafts[&key].state,
            crate::model::DraftState::Conflict { .. }
        ));
        assert!(rt.presented("captain-doc-conflict").is_some());
        press(&mut rt, "captain-doc-conflict-theirs");
        press(&mut rt, "captain-doc-conflict-replace");
        assert_eq!(
            rt.state.captain_document_value(0, "NOTES.md").unwrap()["text"],
            "Mine."
        );
    }

    #[test]
    fn the_recruit_search_return_only_joins_the_matching_row() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        rt.state.show_activity = true;
        rt.state.set_captain_scope(0, vec![1]).unwrap();
        let base = rt.state.next_id;
        // The out-of-scope row sits first in thread order; it must stay disabled.
        {
            let mut t = Thread::new(base);
            t.title = "Outside thread".into();
            t.project = Some(99);
            t.session = Some("session".into());
            t.never_sent = false;
            rt.state.threads.push(t);
        }
        for (offset, title) in [(1usize, "First thread"), (2usize, "Second thread")] {
            let mut t = Thread::new(base + offset);
            t.title = title.into();
            t.project = Some(1);
            t.session = Some("session".into());
            t.never_sent = false;
            rt.state.threads.push(t);
        }
        rt.state.next_id += 3;
        press(&mut rt, "brief-crew-recruit");
        rt.frame(0.25);
        // Typing "Second" then Return joins only the matching row.
        press(&mut rt, "crew-recruit-search");
        assert!(
            rt.state.brief.recruit.as_ref().unwrap().query.focused,
            "search field focused"
        );
        rt.state.brief.recruit.as_mut().unwrap().query.text = "Second".into();
        tap(&mut rt, rune::Key::Enter);
        assert_eq!(
            rt.state
                .threads
                .iter()
                .find(|t| t.id == base + 1)
                .unwrap()
                .crew_captain,
            None
        );
        assert_eq!(
            rt.state
                .threads
                .iter()
                .find(|t| t.id == base + 2)
                .unwrap()
                .crew_captain,
            Some(0),
            "Return must recruit the matching thread"
        );
        // A query that matches nothing queues nothing. The palette is still open
        // from the successful join's replacement; focus its field and search.
        rt.frame(0.5);
        press(&mut rt, "brief-crew-recruit");
        rt.frame(0.25);
        press(&mut rt, "crew-recruit-search");
        rt.state.brief.recruit.as_mut().unwrap().query.text = "zzz".into();
        tap(&mut rt, rune::Key::Enter);
        assert_eq!(
            rt.state
                .threads
                .iter()
                .find(|t| t.id == base + 1)
                .unwrap()
                .crew_captain,
            None
        );
        // Down hands focus to the list and Return presses the first enabled row,
        // skipping the disabled out-of-scope entry that sorts earlier. Escape
        // closes the still-open palette first.
        tap(&mut rt, rune::Key::Escape);
        rt.frame(0.25);
        press(&mut rt, "brief-crew-recruit");
        rt.frame(0.25);
        press(&mut rt, "crew-recruit-search");
        tap(&mut rt, rune::Key::Down);
        let focused = rt
            .elements()
            .into_iter()
            .find(|e| e.focused)
            .and_then(|e| e.key);
        assert_eq!(
            focused.as_deref(),
            Some(format!("crew-recruit-{}", base + 1).as_str()),
            "Down lands on the first enabled row, skipping the disabled one"
        );
        tap(&mut rt, rune::Key::Enter);
        assert_eq!(
            rt.state
                .threads
                .iter()
                .find(|t| t.id == base + 1)
                .unwrap()
                .crew_captain,
            Some(0),
            "Return must press the first enabled row, not the disabled one"
        );
        assert_eq!(
            rt.state
                .threads
                .iter()
                .find(|t| t.id == base)
                .unwrap()
                .crew_captain,
            None
        );
        std::fs::remove_dir_all(&rt.state.data_dir).ok();
    }

    #[test]
    fn a_pending_join_survives_its_source_thread_disappearing() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        rt.state.show_activity = true;
        // The remote accepted the join command, then a projection removed the
        // thread before the receipt; rendering must not panic on the gap.
        let missing = rt.state.next_id + 50;
        rt.state.crew_pending.insert(
            ("local".into(), uuid::Uuid::new_v4()),
            crate::model::CrewChange {
                captain: 0,
                thread: missing,
                kind: crate::model::CrewKind::Join,
            },
        );
        rt.frame(0.5);
        scroll_brief(&mut rt);
        rt.frame(0.5);
        assert!(
            rt.presented(&format!("brief-crew-{missing}")).is_some(),
            "the pending join keeps a quiet row"
        );
        std::fs::remove_dir_all(&rt.state.data_dir).ok();
    }

    #[test]
    fn an_earlier_conversation_reads_documents_on_its_own_panel() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        rt.state.show_activity = true;
        // An earlier conversation of captain 0: same captain_home identity, not the
        // current primary home.
        let earlier_id = rt.state.next_id;
        rt.state.next_id += 1;
        {
            let mut t = Thread::new(earlier_id);
            t.title = "Earlier conversation".into();
            t.captain_home = true;
            t.captain = Some(0);
            rt.state.threads.push(t);
        }
        let index = rt
            .state
            .threads
            .iter()
            .position(|t| t.id == earlier_id)
            .unwrap();
        rt.state.select_thread(index);
        rt.frame(0.5);
        assert_eq!(
            rt.state.captain_access(0),
            crate::model::CaptainAccess::Earlier
        );
        scroll_brief(&mut rt);
        press(&mut rt, "brief-document-NOTES.md");
        rt.frame(0.5);
        assert!(rt.presented("captain-doc-panel").is_some());
        assert!(
            rt.presented("captain-doc-edit").is_none(),
            "an earlier view has no Edit"
        );
        // The tab lives on the earlier thread's panel, not the primary home's.
        let panel = rt.state.panel_key();
        assert!(
            rt.state
                .right_panels
                .get(&panel)
                .surfaces
                .iter()
                .any(|surface| matches!(
                    surface,
                    crate::right_panel::Surface::CaptainDocument { captain: 0, .. }
                )),
            "document surface opened on the earlier panel"
        );
        std::fs::remove_dir_all(&rt.state.data_dir).ok();
    }

    #[test]
    fn the_move_picker_offers_in_scope_targets_and_blocks_the_rest() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        // Captain 1 gets a home and a scope the member fits; captain 2 stays scopeless.
        rt.state.open_captain(1);
        rt.state.set_captain_scope(1, vec![1]).unwrap();
        rt.state.open_captain(0);
        rt.state.set_captain_scope(0, vec![1]).unwrap();
        let member_id = rt.state.next_id;
        rt.state.next_id += 1;
        let mut member = Thread::new(member_id);
        member.title = "Worker".into();
        member.project = Some(1);
        member.crew_captain = Some(0);
        member.session = Some("session".into());
        member.never_sent = false;
        rt.state.threads.push(member);
        rt.frame(0.25);
        rt.state.show_activity = true;
        press(&mut rt, &format!("brief-crew-more-{member_id}"));
        rt.frame(0.25);
        press(&mut rt, "crew-menu-move");
        rt.frame(0.25);
        assert!(rt.presented("crew-move-picker").is_some());
        assert!(
            rt.presented("crew-move-0").is_none(),
            "source is not a target"
        );
        // Captain 2 has no workspace that accepts the thread: its row is disabled.
        let (row, _) = rt.presented("crew-move-2").expect("captain 2 row");
        let _ = row;
        press(&mut rt, "crew-move-1");
        rt.frame(0.25);
        assert_eq!(
            rt.state
                .threads
                .iter()
                .find(|t| t.id == member_id)
                .unwrap()
                .crew_captain,
            Some(1)
        );
        std::fs::remove_dir_all(&rt.state.data_dir).ok();
    }

    #[test]
    fn crew_rows_release_and_recruit_through_the_brief() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        press(&mut rt, "open-brief");
        rt.frame(1.0);
        let thread_id = rt.state.next_id;
        rt.state.next_id += 1;
        rt.state.set_captain_scope(0, vec![1]).unwrap();
        let mut recruit = Thread::new(thread_id);
        recruit.title = "Recruitable thread".into();
        recruit.project = Some(1);
        recruit.session = Some("session".into());
        recruit.never_sent = false;
        rt.state.threads.push(recruit);
        let ordinary = rt.state.threads.len() - 1;
        press(&mut rt, "brief-crew-recruit");
        rt.frame(0.25);
        press(&mut rt, &format!("crew-recruit-{thread_id}"));
        rt.frame(0.25);
        assert_eq!(rt.state.threads[ordinary].crew_captain, Some(0));
        assert!(rt.state.captain_crew(0).contains(&thread_id));
        assert!(rt.presented(&format!("brief-crew-{thread_id}")).is_some());
        press(&mut rt, &format!("brief-crew-more-{thread_id}"));
        rt.frame(0.25);
        press(&mut rt, "crew-menu-release");
        rt.frame(0.25);
        assert_eq!(rt.state.threads[ordinary].crew_captain, None);
    }
}
