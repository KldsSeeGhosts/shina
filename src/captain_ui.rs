//! A captain's Brief content (scope, memory, routines, earlier conversations and reset),
//! shown in the Inspector's Overview and Context tabs, the "You" profile editor, the
//! background rows in the settings menu, and the quiet away line in a captain's header.
//!
//! The Brief is ported from KSG Code's `chat/ThreadDetailsPanel.tsx` and
//! `chat/ThreadDetailsSection.tsx`: sections with a small muted heading and a hairline between
//! them, 32 px rows at a 10 px radius. It shares crew_ui's design language: hover is ink at 6 %,
//! active 8 %, opaque hairlines in `line`, motion of 0.18 s on `cubic-bezier(.23, 1, .32, 1)`.
use super::META_COMPACT as META;
use super::crew_ui::{active_wash, ghost_button, hover_wash, motion, solid_button};
use super::*;
use crate::model::BackgroundStatus;
use crate::power::KeepAwake;
use crate::routines::{RoutineView, Schedule};
use rune::Transition;

/// Below this window width (ui.rs's `narrow`) the Brief is a sheet over the chat.
pub(super) const SHEET_BELOW: f32 = 1050.0;
/// The width rows and chips assume for the Inspector's panels.
pub(super) const PANEL_WIDTH: f32 = 420.0;
/// Rows, chips and inputs share this corner.
/// Rows, buttons and chips.
const ROW_RADIUS: f32 = 4.0;
/// Cards.
const CARD_RADIUS: f32 = 6.0;
/// KSG's `text-[13px] font-medium` detail rows.
const ROW_TEXT: TextStyle = TextStyle::new(13.0, 500.0).lh(19.0).tracking(-0.08);

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

/// The retired captain record currently open in the archive sheet, if any.
pub(super) fn retired_record_captain(s: &State) -> Option<usize> {
    if let Some((server, captain)) = &s.brief.record
        && *server == s.machines.selected
        && *captain < s.captains.len()
        && s.captain_phase(*captain) == crate::model::CaptainPhase::Retired
    {
        return Some(*captain);
    }
    None
}

/// The captain whose Brief lives in the current thread's Inspector (primary home or an
/// earlier conversation of that captain; never a retired record sheet).
pub(super) fn home_brief_captain(s: &State) -> Option<usize> {
    let t = s.current();
    s.is_captain_home(t)
        .then_some(t.captain)
        .flatten()
        .filter(|&captain| captain < s.captains.len())
}

/// The captain whose Brief is shown: a retired record opened from the Archive, else the
/// primary home or an earlier conversation of the open thread.
pub(super) fn brief_captain(s: &State) -> Option<usize> {
    retired_record_captain(s).or_else(|| home_brief_captain(s))
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
pub(super) fn brief_error_banner(s: &State, p: Palette) -> Option<El> {
    let error = s.brief.error.as_ref()?;
    Some(
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
    )
}

pub(super) fn brief_panel_for(s: &State, p: Palette, captain: usize, record: bool) -> El {
    if captain >= s.captains.len() {
        return el();
    }
    let access = s.captain_access(captain);
    let profile = &s.captains[captain];
    let mut role = profile.role.trim().to_string();
    if role.is_empty() {
        role = "Captain".into();
    }
    let identity = row()
        .key("brief-identity")
        .px(16.0)
        .pt(14.0)
        .pb(12.0)
        .gap(12.0)
        .center_y()
        .child(
            el().size(48.0, 48.0)
                .center()
                .child(captain_wash(s.dark, profile.portrait, 72.0, 64.0).absolute(-12.0, -8.0))
                .child(avatar(
                    s,
                    p,
                    captain,
                    48.0,
                    if record {
                        crate::model::Status::Ready
                    } else {
                        shown_status(s)
                    },
                )),
        )
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
                            label(&profile.name, IDENTITY, p.ink)
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
    if !record && let Some(error) = brief_error_banner(s, p) {
        body = body.child(error);
    }
    // A host without captain records or the protocol cannot know the new sections, so
    // they stay hidden rather than drawing empty state.
    let knowable = !matches!(
        access,
        crate::model::CaptainAccess::UpdateRequired | crate::model::CaptainAccess::NoRecords
    );
    // A retired record has no Inspector tabs (spec §3.9), so its sheet stacks the read-only
    // Overview and Context sections together. Active/archived homes keep Context in its own
    // tab, and the Crew tab owns the roster and the manage/new/recruit controls.
    let sections: Vec<Option<El>> = if record {
        vec![
            knowable.then(|| digest_section(s, p, captain)),
            knowable.then(|| {
                super::context_ui::documents_section(
                    s,
                    p,
                    captain,
                    super::context_ui::Audience::Captain,
                )
            }),
            Some(super::context_ui::memory_section(s, p, captain)),
            Some(super::context_ui::conversation_section_with(
                s, p, captain, false,
            )),
            super::captain_lifecycle_ui::section_visible(s, captain)
                .then(|| super::captain_lifecycle_ui::section(s, p, captain)),
        ]
    } else {
        vec![
            knowable.then(|| digest_section(s, p, captain)),
            Some(
                routines_section(s, p, captain)
                    .disabled(access != crate::model::CaptainAccess::Full),
            ),
            Some(
                behaviour_section(s, p, captain)
                    .disabled(access != crate::model::CaptainAccess::Full),
            ),
            super::captain_lifecycle_ui::section_visible(s, captain)
                .then(|| super::captain_lifecycle_ui::section(s, p, captain)),
        ]
    };
    for section in sections.into_iter().flatten() {
        body = body
            .child(el().h(1.0).mx(16.0).fill(p.line).hairline())
            .child(section);
    }
    col()
        .key(if record {
            "retired-captain-brief"
        } else {
            "captain-brief"
        })
        .role("complementary")
        .label(format!("{}'s brief", profile.name))
        .min_h(0.0)
        .h_full()
        .w_full()
        .children(brief_notice(s, p, captain, access, record))
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
    record: bool,
) -> Vec<El> {
    let server = s.machines.selected.clone();
    if record {
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

/// A retired record's Brief opens as a sheet over the chat, as KSG's `ui/sheet.tsx` on the
/// right: a dimmed backdrop that closes it, and the panel inset 8 px with a 12 px radius.
/// A record has no home thread, so it has no Inspector to sit in; this is the only place
/// `brief_panel` draws outside the Inspector. `top` keeps the title strip and its window
/// buttons clear.
pub(super) fn brief_sheet(s: &State, p: Palette, top: f32) -> El {
    let Some(captain) = retired_record_captain(s) else {
        return el();
    };
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
            brief_panel_for(s, p, captain, true)
                .w(width)
                .fill(p.panel)
                .radius(12.0)
                .border(1.0, p.frame)
                .box_shadow(soft_shadow(p))
                .on_press(|_: &mut State| {})
                .transition(Transition::offset(16.0, 0.0).combined(Transition::opacity())),
        )
}

/// The Digest section: the automatic crew-activity digest, a rule, then the
/// saved DIGEST.md under its own sub-heading so the document's age never reads
/// as the age of the activity above it. The preview bounds itself to six
/// lines when activity sits above it, twelve when it stands alone. The
/// document is only read on demand and the section never writes it.
pub(super) fn digest_section(s: &State, p: Palette, captain: usize) -> El {
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
            super::context_ui::open_captain_document(s, captain, "DIGEST.md");
        }
    });
    let has_activity = super::captain_activity_ui::activity_has_entries(s, captain);
    let mut section = section("brief-digest", "Digest", None, vec![open], p);
    // Only "What's new" lives here; "Needs you" and "In progress" belong to the
    // request bar and the Crew tab (spec §3.4).
    section = section.child(super::captain_activity_ui::activity_whats_new(
        s, p, captain,
    ));
    // The only divider inside the section: activity above, the saved digest below.
    section = section.child(
        el().key("brief-digest-rule")
            .h(1.0)
            .mx(10.0)
            .fill(p.line)
            .hairline(),
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

/// The Inspector's Crew tab (spec §3.5): the grouped roster — Needs you, Working,
/// Queued, Offline, Idle — one `crew_ui::roster_row` per member, plus a pinned 44 pt
/// footer with New crew thread and Recruit on a Full-access captain. Crew membership
/// has no KSG equivalent; the grouping follows the work rows' visual language.
pub(super) fn crew_section(s: &State, p: Palette, captain: usize) -> El {
    let access = s.captain_access(captain);
    if access == crate::model::CaptainAccess::NoRecords {
        return col()
            .key("crew-roster-rows")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(quiet_note(format!("Loading from {}.", machine_label(s)), p));
    }
    let home = s.captain_home(captain).unwrap_or_else(|| s.current().id);
    let members = super::crew_ui::roster_members(s, home, captain);
    let pending: Vec<(usize, crate::model::CrewKind)> = s
        .crew_pending
        .iter()
        .filter(|((server, _), change)| *server == s.machines.selected && change.captain == captain)
        .map(|(_, change)| (change.thread, change.kind))
        .collect();
    let current = s.current().id;
    let mut rows = col().gap(2.0);
    // On a worker thread, put a pressable captain row at the top of the Crew tab so the
    // user always has a direct return path to the captain's home (spec §3.5).
    if !s.is_captain_home(s.current()) {
        let profile = &s.captains[captain];
        rows = rows.child(
            row()
                .key("crew-tab-captain")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!("Open {}", profile.name))
                .focusable(true)
                .h(32.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| s.open_captain(captain))
                .child(avatar(s, p, captain, 20.0, s.captain_status(captain)))
                .child(
                    label(&profile.name, ROW_TEXT, p.ink)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(meta_text("Captain", p.faint)),
        );
    }
    // Pending joins show a quiet in-flight row; vanished threads keep the receipt.
    for (thread, kind) in &pending {
        if s.threads.iter().any(|t| t.id == *thread) {
            continue;
        }
        let note = match kind {
            crate::model::CrewKind::Join => "Joining, but the thread is no longer here.",
            crate::model::CrewKind::Release => "Leaving, but the thread is no longer here.",
            crate::model::CrewKind::Move { .. } => "Moving, but the thread is no longer here.",
        };
        rows = rows.child(
            label(note, META, p.faint)
                .key(format!("brief-crew-{thread}"))
                .px(10.0)
                .min_h(28.0),
        );
    }
    if let Some(error) = s.crew_errors.get(&s.machines.selected) {
        rows = rows.child(
            quiet_note(error.clone(), p)
                .key("brief-crew-error")
                .color(p.danger),
        );
    }
    // Grouped roster (spec §3.5): Needs you, Working, Queued, Offline, Idle. Each member
    // lands in the first group that claims it, so a queued row that also waits on you is
    // filed under Needs you, and Idle is whatever is left.
    use crate::model::Status;
    let groups: [(&str, fn(&super::crew_ui::RosterMember) -> bool); 5] = [
        ("Needs you", |m| m.status == Status::Waiting),
        ("Working", |m| m.status == Status::Running),
        ("Queued", |m| m.queued),
        ("Offline", |m| m.status == Status::Paused),
        ("Idle", |_| true),
    ];
    let mut any_group = false;
    let mut placed: std::collections::HashSet<usize> = Default::default();
    for (title, matches) in groups {
        let group_members: Vec<&super::crew_ui::RosterMember> = members
            .iter()
            .filter(|m| !placed.contains(&m.thread_id) && matches(m))
            .collect();
        if group_members.is_empty() {
            continue;
        }
        placed.extend(group_members.iter().map(|m| m.thread_id));
        any_group = true;
        rows = rows.child(
            row()
                .h(32.0)
                .px(10.0)
                .center_y()
                .child(meta_label(title, p.muted))
                .child(spacer())
                .child(
                    label(group_members.len().to_string(), TIMER, p.faint).key(format!(
                        "crew-group-{}",
                        title.to_lowercase().replace(' ', "-")
                    )),
                ),
        );
        for member in group_members {
            let key = format!("crew-chip-{}", member.thread_id);
            let mut member_row = super::crew_ui::roster_row(s, p, member, key, true)
                .h(32.0)
                .px(10.0);
            if member.thread_id == current {
                member_row = member_row.fill(super::crew_ui::active_wash(s, p));
            }
            rows = rows.child(member_row);
        }
    }
    if !any_group {
        rows = rows.child(
            quiet_note(
                "No crew yet. This captain starts crew threads as it delegates work.",
                p,
            )
            .key("crew-roster-empty"),
        );
    }
    let mut tab = col().key("crew-tab").grow(1.0).min_h(0.0).w_full();
    tab = tab.child(
        col()
            .key("crew-roster-rows")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(rows),
    );
    if access == crate::model::CaptainAccess::Full {
        tab = tab.child(crew_roster_footer(s, p, captain));
    }
    tab
}

/// The pinned footer of the Crew tab: a rule above a 44 pt row with New crew thread and
/// Recruit. Full access only, per spec §3.5.
fn crew_roster_footer(s: &State, p: Palette, captain: usize) -> El {
    let server = s.machines.selected.clone();
    let draft_reason = s.crew_creation_unavailable(&server, captain);
    let new_server = server.clone();
    let mut new_thread = ghost_button(
        s,
        "crew-roster-new".into(),
        "New crew thread",
        p,
        move |s: &mut State| {
            if s.machines.selected != new_server {
                return;
            }
            if let Err(error) = s.begin_crew_draft(&new_server, captain) {
                s.brief.error = Some(error);
            }
        },
    )
    .disabled(draft_reason.is_some());
    if let Some(reason) = draft_reason {
        new_thread = new_thread.help(reason);
    }
    let recruit_server = server.clone();
    col()
        .key("crew-roster-footer")
        .child(el().h(1.0).fill(p.line).hairline())
        .child(
            row()
                .h(44.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .child(new_thread)
                .child(ghost_button(
                    s,
                    "crew-roster-recruit".into(),
                    "Recruit...",
                    p,
                    move |s: &mut State| {
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
                    },
                )),
        )
}

/// The Overview's Behaviour section (spec §3.4): the Reports policy control and the
/// Restart recovery block. These moved here from the old Crew section, which now shows
/// only the roster.
pub(super) fn behaviour_section(s: &State, p: Palette, captain: usize) -> El {
    section("brief-behaviour", "Behaviour", None, vec![], p)
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

pub(super) fn routines_section(s: &State, p: Palette, captain: usize) -> El {
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

/// Where an earlier conversation's composer would be: it opens read-only, with a way back.
pub(super) fn earlier_bar(s: &State, p: Palette, wide: bool, pane_width: f32) -> El {
    let captain = brief_captain(s);
    let home = captain.and_then(|c| s.captain_home(c));
    // The lane the conversation and composer resolve (`chatCanvasLayout.ts`); `None` until
    // C5 mounts the Details card.
    let lane = chat_lane(pane_width, wide, None);
    let bar = row()
        .key("earlier-bar")
        .role("status")
        .label("Earlier conversation, read-only")
        .w(lane.width)
        .min_h(44.0)
        .px(14.0)
        .py(8.0)
        .gap(10.0)
        .center_y()
        .radius(CARD_RADIUS)
        .border(1.0, p.line)
        .child(
            rune::icon(&super::icons::LOCK, super::ICON)
                .color(p.muted)
                .a11y_hidden(),
        )
        .child(
            label(
                "Earlier conversation. Read-only.",
                TextStyle::new(13.0, 500.0),
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
                    s.show_thread(index);
                }
            },
        ));
    col()
        .key("earlier-lane")
        .w_full()
        .pl(lane_gutter(pane_width) + lane.inset_start)
        .pr(lane_gutter(pane_width) + lane.inset_end)
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
    let mut rows = vec![
        col().child(working).child(rule(p)),
        col().child(awake).child(rule(p)),
    ];
    if let Some(error) = s.background_status().error {
        rows.push(label(error, SMALL, p.danger).wrap().px(10.0).pb(6.0));
    }
    rows
}

/// Dialogs this module owns, mounted above everything else in the window.
pub(super) fn dialogs(s: &State, p: Palette) -> Vec<El> {
    let mut out: Vec<El> = [
        super::context_ui::reset_dialog(s, p),
        super::context_ui::profile_dialog(s, p),
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

    const WIDTHS: [f32; 2] = [620.0, 1280.0];

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
    /// Scroll the Brief's body to its end, as a wheel over the panel does. Wheels
    /// until the scroll-top identity stops moving, so taller bodies still reach
    /// the bottom. `container` is the scroll region's key: "captain-brief" for
    /// Overview, "context-tab" for Context.
    fn scroll_panel_body(rt: &mut Runtime<State>, container: &str) {
        let (panel, _) = rt.presented(container).unwrap();
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
    fn scroll_brief(rt: &mut Runtime<State>) {
        scroll_panel_body(rt, "captain-brief");
    }
    /// Scroll `key` until its whole box is in `container`'s viewport. A visible
    /// centre alone does not rule out clipping at the edge.
    fn reveal_in(rt: &mut Runtime<State>, container: &str, key: &str) {
        for _ in 0..24 {
            if fully_visible(rt, key) {
                return;
            }
            rt.update(|_, tx| tx.scroll_into_view_in(container, key.to_string()));
            rt.frame(0.5);
        }
        panic!("{key} never revealed inside {container}'s scroll viewport");
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
    /// The same in the Context tab: switches the Inspector to Context first.
    fn press_in_context(rt: &mut Runtime<State>, key: &str) {
        press(rt, "right-panel-segment-context");
        reveal_in(rt, "context-tab", key);
        press(rt, key);
    }
    fn inside(outer: [f32; 4], inner: [f32; 4]) -> bool {
        inner[0] >= outer[0] - 0.5
            && inner[1] >= outer[1] - 0.5
            && inner[0] + inner[2] <= outer[0] + outer[2] + 0.5
            && inner[1] + inner[3] <= outer[1] + outer[3] + 0.5
    }

    /// Open the Inspector on `kind`, as pressing its strip segment does.
    fn open_tab(rt: &mut Runtime<State>, kind: crate::right_panel::SingletonKind) {
        let key = rt.state.panel_key();
        rt.state.right_panels.open(&key, kind);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.5);
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
        let key = rt.state.panel_key();
        rt.state
            .right_panels
            .open(&key, crate::right_panel::SingletonKind::Overview);
        rt.frame(0.0);
        rt.frame(1.0);
        (rt, routine)
    }

    #[test]
    fn brief_sections_fit_inside_the_inspector_at_supported_widths() {
        for width in WIDTHS {
            let (mut rt, routine) = brief_runtime(width);
            let (panel, _) = rt.presented("captain-brief").expect("brief is shown");
            assert!(
                panel[0] >= 0.0 && panel[0] + panel[2] <= width + 0.5,
                "{width}: {panel:?}"
            );
            // The Inspector docks where it fits and covers the chat where it does not.
            assert_eq!(rt.presented("right-panel").is_some(), true, "{width}");
            assert!(rt.presented("composer").is_some(), "{width}");
            let (inspector, _) = rt.presented("right-panel").expect("right panel");
            // Sections below the strip scroll into the viewport fully, not clipped.
            for key in ["brief-identity", "brief-digest", "brief-routines"] {
                rt.update(|_, tx| tx.scroll_into_view(key.to_string()));
                rt.frame(0.5);
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    rect[0] >= inspector[0]
                        && rect[0] + rect[2] <= inspector[0] + inspector[2] + 0.5,
                    "{key} at {width}: {rect:?} outside {inspector:?}"
                );
            }
            // Scope, memory and reset live in the Context tab.
            press(&mut rt, "right-panel-segment-context");
            rt.frame(0.5);
            for key in [
                "brief-scope-1",
                "brief-scope-remove-2",
                "brief-scope-add",
                "memory-budget",
                "brief-memory-add",
            ] {
                reveal_in(&mut rt, "context-tab", key);
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    inside(inspector, rect),
                    "{key} at {width}: {rect:?} outside {inspector:?}"
                );
                assert!(
                    fully_visible(&rt, key),
                    "{key} at {width}: {rect:?} clipped by the scroll viewport"
                );
            }
            // Routine rows sit in the Overview tab.
            press(&mut rt, "right-panel-segment-overview");
            rt.frame(0.5);
            for key in [
                format!("routine-pause-{routine}"),
                format!("routine-delete-{routine}"),
                "brief-reset".into(),
            ] {
                rt.update(|_, tx| tx.scroll_into_view(key.clone()));
                rt.frame(1.0);
                let (rect, _) = rt
                    .presented(&key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    rect[0] >= inspector[0]
                        && rect[0] + rect[2] <= inspector[0] + inspector[2] + 0.5,
                    "{key} at {width}"
                );
            }
            let _ = panel;
        }
    }

    #[test]
    fn memory_rows_pin_edit_and_delete_through_the_state_api() {
        for width in WIDTHS {
            let (mut rt, _) = brief_runtime(width);
            press(&mut rt, "right-panel-segment-context");
            rt.frame(0.5);
            scroll_panel_body(&mut rt, "context-tab");
            let memories = rt.state.memories(0);
            let release = memories.iter().find(|m| !m.essential).unwrap().id.clone();
            press(&mut rt, &format!("memory-pin-{release}"));
            assert!(rt.state.memories(0).iter().all(|m| m.essential));
            assert!(rt.presented(&format!("memory-lock-{release}")).is_some());
            let (lock_rect, _) = rt
                .presented(&format!("memory-lock-{release}"))
                .unwrap_or_else(|| panic!("{width}: memory-lock-{release} not presented"));
            rt.pointer_move(lock_rect[0] + 4.0, lock_rect[1] + lock_rect[3] / 2.0);
            rt.frame(1.0);
            let edit = format!("memory-edit-{release}");
            let presented = rt.presented(&edit);
            assert!(
                presented.is_some(),
                "{width}: {edit} not presented; actions {:?} row {:?}",
                rt.presented(&format!("memory-actions-{release}")),
                rt.presented(&format!("memory-{release}")),
            );
            press(&mut rt, &edit);
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
    fn scope_chips_remove_and_the_picker_adds_a_workspace() {
        for width in WIDTHS {
            let (mut rt, _) = brief_runtime(width);
            press_in_context(&mut rt, "brief-scope-remove-2");
            assert_eq!(rt.state.captain_scope(0), vec![1], "{width}");
            press_in_context(&mut rt, "brief-scope-add");
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
            // The earlier conversation opens read-only, with a way back.
            // The reset selects a fresh home, so open the Inspector on Context for it.
            open_tab(&mut rt, crate::right_panel::SingletonKind::Context);
            rt.frame(0.5);
            reveal_in(&mut rt, "context-tab", "context-earlier-toggle");
            press(&mut rt, "context-earlier-toggle");
            rt.frame(0.5);
            reveal_in(&mut rt, "context-tab", &format!("earlier-{old}"));
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
    fn a_document_panel_covers_the_chat_below_1050_and_sits_inline_from_1280() {
        // Spec §0.1 / §3.3: Inspector covers the conversation below 1050 pt (620 and 960)
        // and docks beside it at 1280 and 1920.
        for width in [620.0, 960.0] {
            let mut rt = store_runtime(width);
            open_tab(&mut rt, crate::right_panel::SingletonKind::Context);
            press(&mut rt, "context-doc-NOTES.md");
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
        // At 1280 and 1920 the column docks inline and keeps its 360 px floor.
        for width in [1280.0, 1920.0] {
            let mut rt = store_runtime(width);
            open_tab(&mut rt, crate::right_panel::SingletonKind::Context);
            press(&mut rt, "context-doc-NOTES.md");
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
    fn editing_a_document_saves_through_the_panel_and_conflicts_keep_the_draft() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        open_tab(&mut rt, crate::right_panel::SingletonKind::Context);
        press(&mut rt, "context-doc-NOTES.md");
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
    fn crew_rows_release_and_recruit_through_the_brief() {
        let mut rt = store_runtime(1280.0);
        rt.frame(0.25);
        open_tab(&mut rt, crate::right_panel::SingletonKind::Overview);
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
        press(&mut rt, "crew-roster-toggle");
        press(&mut rt, "crew-roster-recruit");
        rt.frame(0.25);
        press(&mut rt, &format!("crew-recruit-{thread_id}"));
        rt.frame(0.25);
        assert_eq!(rt.state.threads[ordinary].crew_captain, Some(0));
        assert!(rt.state.captain_crew(0).contains(&thread_id));
        assert!(rt.presented(&format!("crew-chip-{thread_id}")).is_some());
        // A secondary press on the roster row opens the member's More menu.
        rt.state.brief.more = Some(thread_id);
        rt.invalidate();
        rt.frame(0.25);
        press(&mut rt, "crew-menu-release");
        rt.frame(0.25);
        assert_eq!(rt.state.threads[ordinary].crew_captain, None);
    }
}
