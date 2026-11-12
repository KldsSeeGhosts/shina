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
use crate::model::{BackgroundStatus, MemoryView, UserProfile};
use crate::power::KeepAwake;
use crate::routines::{RoutineView, Schedule};
use rune::Transition;

/// The Brief's width beside the chat, as KSG's right panel.
const PANEL_WIDTH: f32 = 320.0;
/// Below this window width (ui.rs's `narrow`) the Brief is a sheet over the chat.
const SHEET_BELOW: f32 = 1050.0;
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

/// The captain whose Brief the open thread shows: its primary home or an earlier one.
pub(super) fn brief_captain(s: &State) -> Option<usize> {
    let t = s.current();
    s.is_captain_home(t)
        .then_some(t.captain)
        .flatten()
        .filter(|&captain| captain < s.captains.len())
}

/// The open thread is a captain's earlier conversation, which opens read-only.
pub(super) fn is_earlier(s: &State) -> bool {
    brief_captain(s).is_some_and(|captain| s.captain_home(captain) != Some(s.current().id))
}

fn close_brief(s: &mut State) {
    s.show_activity = false;
    s.activity_full = false;
    s.brief.confirm_reset = false;
}

/// A 24 px icon action, as KSG's `THREAD_DETAILS_PANEL_ICON_ACTION_CLASS` (`size-6 rounded-md`).
fn icon_action(
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
        .radius(4.0)
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
    _: &State,
    key: &str,
    title: &str,
    on: bool,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    let travel = rune::Curve::tween(0.15, rune::Easing::cubic(0.23, 1.0, 0.32, 1.0));
    // A square, framed switch: coral with a card-coloured thumb and a 1 px frame when on, a card
    // with a muted thumb off. Near-black on coral read as a hole.
    let thumb = if on { p.card } else { p.muted };
    el().key(key)
        .role("switch")
        .checked(on)
        .label(title)
        .cursor(rune::Cursor::Pointer)
        .focusable(true)
        .size(34.0, 20.0)
        .min_w(34.0)
        .radius(2.0)
        .padding(4.0)
        .center_y()
        .border(2.0, p.frame)
        .fill(if on { p.coral } else { p.card })
        .motion(rune::Spring::new(0.2, 0.0))
        .on_press(action)
        .child(
            el().size(12.0, 12.0)
                .fill(thumb)
                .border(if on { 1.0 } else { 0.0 }, p.frame)
                .offset(if on { 14.0 } else { 0.0 }, 0.0)
                .offset_motion(travel),
        )
}

/// One section, as KSG's `ThreadDetailsSection`: `px-2 pt-2 pb-2.5`, a 32 px heading row in
/// `text-2xs font-medium text-muted-foreground`, actions at its end, and a hairline above.
fn section(key: &str, title: &str, meta: Option<String>, actions: Vec<El>, p: Palette) -> El {
    let mut heading = row()
        .h(32.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .child(pixel_label(title, p.muted));
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

fn quiet_note(text: impl Into<String>, p: Palette) -> El {
    label(text, META, p.muted).wrap().px(10.0).py(4.0)
}

/// The Brief: a captain's standing instructions to Shina, beside its home conversation.
pub(super) fn brief_panel(s: &State, p: Palette) -> El {
    let Some(captain) = brief_captain(s) else {
        return el();
    };
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
                .radius(4.0)
                .fill(p.selected)
                .child(icon("list", p.muted))
                .child(label("Brief", SMALL, p.ink)),
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
        .child(avatar(s, p, captain, 36.0, shown_status(s)))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(label(&profile.name, CAPTAIN_SERIF, p.ink).truncate())
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
    let sections = [
        scope_section(s, p, captain),
        memory_section(s, p, captain),
        routines_section(s, p, captain),
        earlier_section(s, p, captain),
        reset_section(s, p, captain),
    ];
    for section in sections {
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

/// The Brief beside the chat on a wide pane.
pub(super) fn brief_side(s: &State, p: Palette) -> El {
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
        .child(
            brief_panel(s, p)
                .w(width)
                .fill(p.panel)
                .radius(6.0)
                .border(2.0, p.frame)
                .box_shadow(hard_shadow(p))
                .on_press(|_: &mut State| {})
                .transition(Transition::offset(16.0, 0.0).combined(Transition::opacity())),
        )
}

/// Workspace chips, added and removed through the workspace picker.
fn scope_section(s: &State, p: Palette, captain: usize) -> El {
    let scope = s.captain_scope(captain);
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
        (!scope.is_empty()).then(|| scope.len().to_string()),
        vec![add],
        p,
    );
    if scope.is_empty() {
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
                .radius(4.0)
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
    section = section.child(chips);
    if scope.len() > 1 {
        section = section.child(quiet_note(
            "The first workspace hosts the conversation. Tasks name the one they work in.",
            p,
        ));
    }
    section
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
                .radius(2.0)
                .clip_rounded(2.0)
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

/// Reserved for the memory backend's one-line status (another change adds
/// `State::memory_backend_status`). Nothing is shown until it exists.
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
        .radius(6.0)
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
    if s.brief.editing.as_deref() == Some(memory.id.as_str()) {
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
            .radius(4.0)
            .hover(hover_wash(s, p))
            .press(active_wash(s, p))
            .on_press(move |s: &mut State| {
                let result = s.set_memory_essential(captain, &id, !essential);
                report(s, result);
            })
            .child(glyph_icon(
                if essential {
                    &super::pixel_icons::PIN_ON
                } else {
                    &super::pixel_icons::PIN
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
                    .radius(4.0)
                    .fill(p.ink.alpha(if s.dark { 0.06 } else { 0.05 }))
                    .child(
                        rune::icon(&super::pixel_icons::LOCK, super::ICON)
                            .color(p.muted)
                            .a11y_hidden(),
                    )
                    .child(pixel_label("Yours", p.muted)),
            ),
        );
    }
    let edit_id = id.clone();
    let text_value = memory.text.clone();
    let wash = hover_wash(s, p);
    let backing = p.panel.mix(wash, wash.a);
    row()
        .key(format!("memory-{id}"))
        .px(4.0)
        .py(4.0)
        .gap(6.0)
        .radius(ROW_RADIUS)
        .hover(hover_wash(s, p))
        .hover_motion(motion(), motion())
        // The edit and delete icons show on hover and when keyboard focus is in the row.
        .hover_group_focus()
        .child(pin)
        .child(text)
        // The controls lie over the row's trailing edge on hover and focus, on a backing the
        // colour of the hovered row, so the text uses the row's full width at rest.
        .child(
            row()
                .key(format!("memory-actions-{id}"))
                .pin(None, Some(4.0), Some(4.0), None)
                .gap(2.0)
                .px(2.0)
                .radius(ROW_RADIUS)
                .fill(backing)
                .child(icon_action(
                    s,
                    format!("memory-edit-{id}"),
                    "Edit memory",
                    icon("edit", p.muted),
                    p,
                    move |s| {
                        s.brief.adding = false;
                        s.brief.editing = Some(edit_id.clone());
                        s.brief.edit = rune::TextEdit::default();
                        s.brief.edit.replace_text(&text_value);
                        s.brief.edit.focused = true;
                    },
                ))
                .child(icon_action(
                    s,
                    format!("memory-delete-{id}"),
                    "Delete memory",
                    glyph_icon(&super::pixel_icons::TRASH, p.muted),
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
    let add = icon_action(
        s,
        "brief-memory-add".into(),
        "Add memory",
        icon("plus", p.muted),
        p,
        |s| {
            s.brief.editing = None;
            s.brief.adding = true;
            s.brief.draft = rune::TextEdit::default();
            s.brief.draft.focused = true;
        },
    );
    let mut section = section(
        "brief-memory",
        "Memory",
        (!memories.is_empty()).then(|| memories.len().to_string()),
        vec![add],
        p,
    );
    if let Some(status) = memory_backend_line(s) {
        section = section.child(quiet_note(status, p));
    }
    section = section.child(budget_meter(&memories, p));
    if s.brief.adding {
        section = section.child(memory_editor(s, p, captain, None));
    }
    if memories.is_empty() && !s.brief.adding {
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
    let dot = if routine.running {
        p.coral
    } else if failed {
        p.danger
    } else {
        p.muted.alpha(0.45)
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
    row()
        .key(format!("routine-{id}"))
        .label(format!(
            "{}, {line}. Last run: {}",
            routine.name, routine.last_status
        ))
        .px(10.0)
        .py(6.0)
        .gap(10.0)
        .center_y()
        .radius(ROW_RADIUS)
        .hover(hover_wash(s, p))
        .hover_motion(motion(), motion())
        .child(
            el().size(16.0, 16.0)
                .center()
                .child(glyph_icon(&super::pixel_icons::CALENDAR, p.muted))
                .child(
                    el().size(6.0, 6.0)
                        .fill(dot)
                        .pin(None, Some(-1.0), Some(-1.0), None)
                        .a11y_hidden(),
                ),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(
                    label(
                        &routine.name,
                        ROW_TEXT,
                        if paused { p.muted } else { p.ink },
                    )
                    .truncate(),
                )
                .child(
                    row()
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
        .child(icon_action(
            s,
            format!("routine-delete-{id}"),
            &format!("Delete {}", routine.name),
            glyph_icon(&super::pixel_icons::TRASH, p.muted),
            p,
            move |s| {
                let result = s.delete_routine(id);
                report(s, result);
            },
        ))
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
        ))
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

fn reset_section(s: &State, p: Palette, captain: usize) -> El {
    let busy = s
        .captain_home(captain)
        .and_then(|home| s.threads.iter().find(|t| t.id == home))
        .is_some_and(|t| t.status.busy() || t.configuring || t.queued());
    let mut section =
        section("brief-conversation", "Conversation", None, vec![], p).child(quiet_note(
            "Start over with an empty conversation. Memory, scope, routines and tasks stay.",
            p,
        ));
    let button = ghost_button(s, "brief-reset".into(), "Reset conversation…", p, |s| {
        s.brief.confirm_reset = true
    })
    .mx(10.0)
    .mt(4.0)
    .disabled(busy)
    .opacity(if busy { 0.5 } else { 1.0 });
    section = section.child(row().child(button));
    if busy {
        section = section.child(quiet_note("Stop the current turn first.", p));
    }
    section
}

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
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
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
            rune::icon(&super::pixel_icons::LOCK, super::ICON)
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
pub(super) fn away_status(s: &State, p: Palette) -> Option<El> {
    brief_captain(s)?;
    let status: BackgroundStatus = s
        .brief
        .demo_status
        .clone()
        .unwrap_or_else(|| s.background_status());
    // Both texts are status words, so both are pixel type. Work running in the background is
    // `sea`, as it is in the crew tray; paused routines are muted.
    let (word, detail, ink) = if status.hidden_to_tray && status.working > 0 {
        (
            "Working in the background",
            Some(plural(status.working, "task", "tasks")),
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
        .child(icon("crescent", ink))
        .child(super::pixel_text(word, ink).truncate().shrink().min_w(0.0));
    if let Some(detail) = detail {
        line = line.child(
            super::pixel_text(detail, ink)
                .truncate()
                .shrink()
                .min_w(0.0),
        );
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
        .radius(4.0)
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

/// Settings rows for background work, as KSG's `SettingsRow` (`settings/settingsLayout.tsx`):
/// a `text-sm font-medium` title over a one-line `text-xs` muted description, with the
/// control at the end, and Keep awake's three choices as KSG's segmented `ToggleGroup`.
pub(super) fn background_settings(s: &State, p: Palette) -> Vec<El> {
    let settings = s.background_settings();
    let keep_working = settings.keep_working;
    let supported = cfg!(windows);
    let row_title = |title: &str| label(title, TextStyle::new(13.0, 500.0), p.ink);
    let row_note = |note: &str| label(note, TextStyle::new(12.0, 420.0).lh(16.0), p.muted).wrap();
    let working = row()
        .key("background-row")
        .px(10.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
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
    let policy = settings.keep_awake;
    let note = match policy {
        KeepAwake::Off => "This PC sleeps as usual.",
        KeepAwake::WhileRunning => "This PC stays awake while tasks run.",
        KeepAwake::OnAcPower => "Awake while tasks run and plugged in.",
    };
    let mut group = row()
        .key("keep-awake-toggle")
        .role("radiogroup")
        .label("Keep awake")
        .w_full()
        .padding(2.0)
        .gap(2.0)
        .radius(4.0)
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
                .radius(4.0)
                .fill(if on { p.card } else { Color::TRANSPARENT })
                // The chosen segment has a 1 px frame. Idle ones keep a clear border so no
                // segment changes size when the choice moves.
                .border(1.0, if on { p.frame } else { Color::TRANSPARENT })
                .hover(if on { p.card } else { hover_wash(s, p) })
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| s.set_keep_awake(choice))
                .child(label(
                    title,
                    TextStyle::new(12.0, 500.0),
                    if on { p.ink } else { p.muted },
                )),
        );
    }
    let awake = col()
        .key("keep-awake-row")
        .px(10.0)
        .py(8.0)
        .gap(8.0)
        .child(
            col()
                .gap(2.0)
                .child(row_title("Keep awake"))
                .child(row_note(note)),
        )
        .child(group);
    let mut rows = vec![working, awake];
    if let Some(error) = s.background_status().error {
        rows.push(label(error, SMALL, p.danger).wrap().px(10.0).pb(6.0));
    }
    rows
}

/// The settings menu's entry for the profile editor.
pub(super) fn profile_item(s: &State, p: Palette) -> El {
    let profile = s.user_profile();
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
        .radius(6.0)
        .hover(p.selected)
        .on_press(open_profile)
        .child(
            el().size(28.0, 28.0)
                .center()
                .radius(4.0)
                .fill(p.coral)
                .border(1.0, p.frame)
                .child(super::pixel_text(
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
    let profile = s.user_profile();
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
    let result = s.set_user_profile(profile);
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
                .child(pixel_label(title, p.muted))
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
        .radius(4.0)
        .fill(field_fill(s, p))
        .field_focus(edit, p)
}

/// "You": the global profile every captain reads, ported from KSG's settings panels
/// (`settings/settingsLayout.tsx` rows inside a `SettingsGroup` card) as a dialog, since Shina's
/// settings live in a menu. A live count holds it to 1 KiB.
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
        .radius(6.0)
        .fill(p.card)
        .border(2.0, p.frame)
        .box_shadow(hard_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            row()
                .gap(8.0)
                .child(
                    col()
                        .gap(4.0)
                        .grow(1.0)
                        .min_w(0.0)
                        .child(label("You", super::DIALOG_TITLE, p.ink))
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
    [reset_dialog(s, p), profile_dialog(s, p)]
        .into_iter()
        .flatten()
        .collect()
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
    /// Scroll the Brief to its end, as a wheel over the panel does.
    fn scroll_brief(rt: &mut Runtime<State>) {
        let (panel, _) = rt.presented("captain-brief").unwrap();
        rt.wheel(panel[0] + panel[2] / 2.0, panel[1] + 200.0, -2000.0);
        rt.frame(1.0);
    }
    fn inside(outer: [f32; 4], inner: [f32; 4]) -> bool {
        inner[0] >= outer[0] - 0.5
            && inner[1] >= outer[1] - 0.5
            && inner[0] + inner[2] <= outer[0] + outer[2] + 0.5
            && inner[1] + inner[3] <= outer[1] + outer[3] + 0.5
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
            let (rt, routine) = brief_runtime(width);
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
            for key in [
                "close-brief",
                "brief-scope-1",
                "brief-scope-remove-2",
                "brief-scope-add",
                "memory-budget",
                "brief-memory-add",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    inside(panel, rect),
                    "{key} at {width}: {rect:?} outside {panel:?}"
                );
            }
            // Rows below the fold still lay out inside the panel's width.
            for key in [
                format!("routine-pause-{routine}"),
                format!("routine-delete-{routine}"),
                "brief-reset".into(),
            ] {
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
            press(&mut rt, "save-memory");
            assert!(
                rt.state
                    .memories(0)
                    .iter()
                    .any(|m| m.text == "The release branch is cut on Fridays.")
            );
            press(&mut rt, &format!("memory-delete-{release}"));
            assert_eq!(rt.state.memories(0).len(), 1, "{width}");
            press(&mut rt, "brief-memory-add");
            rt.state.brief.draft.replace_text("Use British spelling.");
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
            press(&mut rt, "brief-scope-remove-2");
            assert_eq!(rt.state.captain_scope(0), vec![1], "{width}");
            press(&mut rt, "brief-scope-add");
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
}
