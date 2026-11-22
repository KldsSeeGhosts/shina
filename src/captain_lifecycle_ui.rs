//! Captain lifecycle UI: the Brief's bottom "Captain" section (reset, archive, demote,
//! delete), the four lifecycle confirmation dialogs, and the bar that replaces the composer
//! while a captain's home is archived.
//!
//! The dialogs adapt captain_ui's `reset_dialog`, itself ported from KSG's
//! `ui/alert-dialog.tsx`. Captain-specific group actions have no KSG equivalent: KSG's
//! `Sidebar.tsx` only confirms one thread's archive or delete at a time.
use super::captain_ui::{quiet_note, switch};
use super::crew_ui::{active_wash, ghost_button, hover_wash, mark_slot, motion};
use super::*;
use crate::model::{CaptainLifecycleAction, CaptainLifecycleDialog, CaptainPhase, CrewMutation};
use serde_json::Value;

/// Brief row text, as captain_ui's `ROW_TEXT`.
const ROW_TEXT: TextStyle = TextStyle::new(13.0, 500.0).lh(19.0).tracking(-0.08);
/// Row corners, shared with the Brief.
const ROW_RADIUS: f32 = 4.0;
/// Card corners, shared with the Brief.
const CARD_RADIUS: f32 = 6.0;
/// The backend's stale-version rejection prefix, after which a fresh preview is required.
const CONFLICT_PREFIX: &str = "Captain changed since version";

/// The machine a dialog or row was bound to at render; a stale callback on another selection
/// does nothing.
fn expected_server(s: &State, server: &str) -> bool {
    s.machines.selected == server
}

/// Threads in the captain's group that would block demotion or deletion: the primary home
/// and the canonical crew that are still working. The backend's preview blockers stay
/// authoritative; this only disables the entry early with its reason.
fn working_threads(s: &State, captain: usize) -> usize {
    let mut ids = s.captain_crew(captain);
    if let Some(home) = s.captain_home(captain) {
        ids.push(home);
    }
    s.threads
        .iter()
        .filter(|thread| {
            ids.contains(&thread.id)
                && (thread.status.busy() || thread.configuring || thread.queued())
        })
        .count()
}

fn pluralize(n: usize, one: &str, many: &str) -> String {
    if n == 1 { one.into() } else { many.into() }
}

/// A ghost button with danger ink, for "Delete captain...". KSG's destructive variant.
fn danger_ghost(
    s: &State,
    key: String,
    title: &str,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .center_y()
        .h(28.0)
        .px(10.0)
        .radius(6.0)
        .border(1.0, p.frame)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .on_press(action)
        .child(label(title, TextStyle::new(13.0, 500.0), p.danger))
}

/// The filled danger confirm, as KSG's `variant: "destructive"` button.
fn danger_button(key: &str, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    framed(
        row()
            .key(key)
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(title)
            .focusable(true)
            .center_y()
            .h(32.0)
            .px(14.0)
            .radius(6.0)
            .fill(p.danger)
            .press(p.danger.alpha(0.8))
            .on_press(action)
            .child(label(title, TextStyle::new(13.0, 600.0), p.on_brass)),
        p,
    )
}

/// Whether the Captain section draws at all: never for a deleted captain or while an
/// earlier conversation of a live captain is open. Archived and retired captains keep it
/// so Delete stays reachable.
pub(super) fn section_visible(s: &State, captain: usize) -> bool {
    match s.captain_phase(captain) {
        CaptainPhase::Deleted => false,
        CaptainPhase::Active => s.captain_access(captain) != crate::model::CaptainAccess::Earlier,
        CaptainPhase::Archived | CaptainPhase::Retired => true,
    }
}

/// The Brief's bottom section, retitled "Captain": reset, archive, demote and delete as a
/// ghost button with a note under each. Replaces the old reset-only "Conversation" section;
/// keys `brief-conversation` and `brief-reset` are kept. No KSG equivalent.
pub(super) fn section(s: &State, p: Palette, captain: usize) -> El {
    use crate::model::CaptainLifecycleAction as A;
    let mut sec = super::captain_ui::section("brief-conversation", "Captain", None, vec![], p);
    if !section_visible(s, captain) {
        return sec;
    }
    let phase = s.captain_phase(captain);
    let active = phase == CaptainPhase::Active;
    let record = s.brief.record.is_some();
    let full = s.captain_access(captain) == crate::model::CaptainAccess::Full;
    let server = s.machines.selected.clone();
    let name = s
        .captains
        .get(captain)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| "This captain".into());
    let mut reasons: Vec<String> = Vec::new();
    let mut row_for =
        |sec: El, button: El, note: String, reason: Option<String>, disabled: bool| {
            let button = button.mx(10.0).mt(4.0).disabled(disabled);
            if let (Some(reason), true) = (reason, disabled) {
                reasons.push(reason);
            }
            sec.child(row().child(button)).child(quiet_note(note, p))
        };
    if active && !record {
        let busy_home = s
            .captain_home(captain)
            .and_then(|home| s.threads.iter().find(|t| t.id == home))
            .is_some_and(|t| t.status.busy() || t.configuring || t.queued());
        let reset = ghost_button(s, "brief-reset".into(), "Reset conversation…", p, |s| {
            s.brief.confirm_reset = true
        });
        sec = row_for(
            sec,
            reset,
            "Start over with an empty conversation. Memory, scope, routines, crew and tasks stay."
                .into(),
            busy_home.then(|| "Stop the current turn first.".into()),
            busy_home || !full,
        );
        let expected = server.clone();
        let archive = ghost_button(
            s,
            "brief-archive-captain".into(),
            "Archive captain…",
            p,
            move |s| {
                if !expected_server(s, &expected) {
                    return;
                }
                if let Err(error) = s.open_captain_lifecycle(&expected, captain, A::Archive) {
                    s.brief.error = Some(error);
                }
            },
        );
        let archive_reason = s.captain_lifecycle_unavailable(&server, captain, A::Archive);
        sec = row_for(
            sec,
            archive,
            format!("Moves {name} and its crew to the Archive. Undo any time."),
            archive_reason.clone(),
            archive_reason.is_some(),
        );
        let expected = server.clone();
        let demote = ghost_button(
            s,
            "brief-demote".into(),
            "Make it a thread…",
            p,
            move |s| {
                if !expected_server(s, &expected) {
                    return;
                }
                if let Err(error) = s.open_captain_lifecycle(&expected, captain, A::Demote) {
                    s.brief.error = Some(error);
                }
            },
        );
        let working = working_threads(s, captain);
        let demote_reason = s
            .captain_lifecycle_unavailable(&server, captain, A::Demote)
            .or_else(|| {
                (working > 0).then(|| {
                    format!(
                        "Stop {working} working {} first.",
                        pluralize(working, "thread", "threads")
                    )
                })
            });
        sec = row_for(
            sec,
            demote,
            "Keeps this conversation as an ordinary thread and releases the crew.".into(),
            demote_reason.clone(),
            demote_reason.is_some(),
        );
    }
    let expected = server.clone();
    let delete = danger_ghost(
        s,
        "brief-delete-captain".into(),
        "Delete captain…",
        p,
        move |s| {
            if !expected_server(s, &expected) {
                return;
            }
            if let Err(error) = s.open_captain_lifecycle(&expected, captain, A::Delete) {
                s.brief.error = Some(error);
            }
        },
    );
    let working = if active {
        working_threads(s, captain)
    } else {
        0
    };
    let delete_reason = s
        .captain_lifecycle_unavailable(&server, captain, A::Delete)
        .or_else(|| {
            (working > 0).then(|| {
                format!(
                    "Stop {working} working {} first.",
                    pluralize(working, "thread", "threads")
                )
            })
        });
    sec = row_for(
        sec,
        delete,
        "Removes the profile, plan, rules and memory. Crew threads stay.".into(),
        delete_reason.clone(),
        delete_reason.is_some(),
    );
    if let Some(reason) = reasons.into_iter().next() {
        sec = sec.child(quiet_note(reason, p).key("brief-captain-reason"));
    }
    sec
}

fn preview_ids(preview: &Option<Value>, field: &str) -> Vec<usize> {
    preview
        .as_ref()
        .and_then(|value| value[field].as_array())
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_u64().map(|id| id as usize))
                .collect()
        })
        .unwrap_or_default()
}

fn preview_blockers(preview: &Option<Value>) -> Vec<String> {
    preview
        .as_ref()
        .and_then(|value| value["blockers"].as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn pending_word(action: CaptainLifecycleAction) -> &'static str {
    use crate::model::CaptainLifecycleAction as A;
    match action {
        A::Archive => "Archiving",
        A::Unarchive => "Unarchiving",
        A::Demote => "Demoting",
        A::Delete => "Deleting",
    }
}

fn dialog_key(action: CaptainLifecycleAction) -> &'static str {
    use crate::model::CaptainLifecycleAction as A;
    match action {
        A::Archive => "captain-archive-dialog",
        A::Unarchive => "captain-unarchive-dialog",
        A::Demote => "captain-demote-dialog",
        A::Delete => "captain-delete-dialog",
    }
}

fn cancel_key(action: CaptainLifecycleAction) -> &'static str {
    use crate::model::CaptainLifecycleAction as A;
    match action {
        A::Archive => "cancel-archive-captain",
        A::Unarchive => "cancel-unarchive-captain",
        A::Demote => "cancel-demote-captain",
        A::Delete => "cancel-delete-captain",
    }
}

fn confirm_key(action: CaptainLifecycleAction) -> &'static str {
    use crate::model::CaptainLifecycleAction as A;
    match action {
        A::Archive => "confirm-archive-captain",
        A::Unarchive => "confirm-unarchive-captain",
        A::Demote => "confirm-demote-captain",
        A::Delete => "confirm-delete-captain",
    }
}

fn confirm_label(action: CaptainLifecycleAction) -> &'static str {
    use crate::model::CaptainLifecycleAction as A;
    match action {
        A::Archive => "Archive captain",
        A::Unarchive => "Unarchive",
        A::Demote => "Make it a thread",
        A::Delete => "Delete captain",
    }
}

/// A crew row inside the archive dialog: status mark, title and status word, with a
/// trailing "Blocks" on a member that is still working. Informational only, never a button.
fn archive_crew_row(s: &State, p: Palette, id: usize) -> El {
    let thread = s.threads.iter().find(|thread| thread.id == id);
    let title = thread
        .map(|thread| {
            if thread.title.is_empty() {
                "Untitled".to_string()
            } else {
                thread.title.clone()
            }
        })
        .unwrap_or_else(|| format!("Thread {id}"));
    let blocking =
        thread.is_some_and(|thread| thread.status.busy() || thread.configuring || thread.queued());
    let mut row_ = row()
        .key(format!("captain-archive-crew-{id}"))
        .label(if blocking {
            format!("{title}, blocks archiving")
        } else {
            title.clone()
        })
        .px(8.0)
        .min_h(28.0)
        .center_y()
        .gap(8.0)
        .radius(ROW_RADIUS);
    if let Some(thread) = thread {
        row_ = row_
            .child(mark_slot(thread.status, p))
            .child(
                label(title, ROW_TEXT, p.ink)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(label(
                thread.status.label(),
                META_LABEL,
                status_ink(thread.status, p),
            ));
    } else {
        row_ = row_.child(icon("thread", p.muted)).child(
            label(title, ROW_TEXT, p.muted)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        );
    }
    if blocking {
        row_ = row_.child(meta_text("Blocks", p.danger));
    }
    row_
}

/// The conflict notice every lifecycle dialog shares: a brass 2 px left bar like the
/// approval cards, keyed `{dialog}-conflict`, telling the user to recheck before confirming.
fn conflict_row(p: Palette, dialog: &CaptainLifecycleDialog, name: &str) -> Option<El> {
    let error = dialog.error.as_ref()?;
    if !error.starts_with(CONFLICT_PREFIX) {
        return None;
    }
    Some(
        row()
            .key(format!("{}-conflict", dialog_key(dialog.action)))
            .role("alert")
            .label(format!("{name} changed while this was open"))
            .w_full()
            .center_y()
            .child(el().w(2.0).h_full().min_h(20.0).fill(p.brass))
            .child(
                label(
                    format!(
                        "{name} changed while this was open. Check the list again before you confirm."
                    ),
                    META,
                    p.brass,
                )
                .wrap()
                .grow(1.0)
                .min_w(0.0)
                .px(8.0)
                .py(4.0),
            ),
    )
}

/// A plain danger line for a rejection that is not a version conflict.
fn error_row(p: Palette, dialog: &CaptainLifecycleDialog) -> Option<El> {
    let error = dialog.error.as_ref()?;
    if error.starts_with(CONFLICT_PREFIX) {
        return None;
    }
    Some(
        label(error.clone(), META, p.danger)
            .key(format!("{}-error", dialog_key(dialog.action)))
            .wrap(),
    )
}

/// Whether the confirm may be pressed now: no pending command, no machine-state or preview
/// blocker, and (for delete) the exact typed name.
fn confirm_ready(
    s: &State,
    dialog: &CaptainLifecycleDialog,
    blockers: &[String],
) -> Result<(), String> {
    use crate::model::CaptainLifecycleAction as A;
    if dialog.pending.is_some() {
        return Err(format!(
            "Waiting for {} to confirm.",
            s.machine_label(&dialog.server)
        ));
    }
    if let Some(reason) =
        s.captain_lifecycle_unavailable(&dialog.server, dialog.captain, dialog.action)
    {
        return Err(reason);
    }
    if dialog.preview.is_none() {
        return Err(format!("Loading from {}.", s.machine_label(&dialog.server)));
    }
    if let Some(blocker) = blockers.first() {
        return Err(blocker.clone());
    }
    if dialog.action == A::Delete {
        if let Some(required) = dialog
            .preview
            .as_ref()
            .and_then(|preview| preview["keepConversationsRequiredReason"].as_str())
            && !dialog.keep_conversations
            && !required.is_empty()
        {
            return Err(required.to_string());
        }
        let name = s
            .captains
            .get(dialog.captain)
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        if dialog.confirm_name.text.trim() != name {
            return Err(format!("Type {name} to confirm."));
        }
    }
    Ok(())
}

/// Press the lifecycle dialog's confirm through the state seam. A stale callback whose
/// machine, captain or action no longer matches the open dialog sends nothing. Errors stay
/// on the dialog (`dialog.error`); a stale preview refreshes and asks for a fresh
/// confirmation.
fn confirm(s: &mut State, server: &str, captain: usize, action: CaptainLifecycleAction) {
    let Some(dialog) = &s.brief.lifecycle else {
        return;
    };
    if s.machines.selected != server
        || dialog.server != server
        || dialog.captain != captain
        || dialog.action != action
    {
        return;
    }
    match s.confirm_captain_lifecycle() {
        Ok(CrewMutation::Saved) => s.brief.lifecycle = None,
        Ok(CrewMutation::Pending(_)) => {}
        Err(error) => {
            if let Some(dialog) = s.brief.lifecycle.as_mut() {
                dialog.error = Some(error);
            }
        }
    }
}

/// The shared dialog scaffold: a dimmed backdrop and a centred `reset_dialog` card with a
/// bounded, vertically scrolling body so it fits at every width.
fn lifecycle_dialog(s: &State, p: Palette, dialog: &CaptainLifecycleDialog, content: El) -> El {
    let close = |s: &mut State| s.brief.lifecycle = None;
    let server = dialog.server.clone();
    let captain = dialog.captain;
    let action = dialog.action;
    let blockers = preview_blockers(&dialog.preview);
    let pending = dialog.pending.is_some();
    let ready = confirm_ready(s, dialog, &blockers);
    let confirm_text = if pending {
        pending_word(action)
    } else {
        confirm_label(action)
    };
    let press = {
        let server = server.clone();
        move |s: &mut State| confirm(s, &server, captain, action)
    };
    let confirm_button = if action == CaptainLifecycleAction::Delete {
        danger_button(confirm_key(action), confirm_text, p, press)
    } else {
        primary_button(confirm_key(action), confirm_text, p, press)
    };
    let reason = ready.clone().err();
    let confirm_button = confirm_button
        .disabled(ready.is_err())
        .help(reason.clone().unwrap_or_else(|| confirm_text.to_string()));
    let mut card = col()
        .key(dialog_key(action))
        .role("alertdialog")
        .label(dialog_title(s, dialog))
        .w(if action == CaptainLifecycleAction::Delete {
            460.0
        } else {
            440.0
        })
        .max_w(s.viewport_width - 32.0)
        .padding(20.0)
        .gap(10.0)
        .radius(CARD_RADIUS)
        .fill(p.card)
        .border(1.0, p.frame)
        .box_shadow(soft_shadow(p))
        .on_press(|_: &mut State| {})
        .child(
            col()
                .min_h(0.0)
                .max_h((s.viewport_height - 180.0).max(160.0))
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(content),
        );
    let name = s
        .captains
        .get(captain)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| "The captain".into());
    if let Some(conflict) = conflict_row(p, dialog, &name) {
        card = card.child(conflict);
    } else if let Some(error) = error_row(p, dialog) {
        card = card.child(error);
    }
    if let Some(reason) = reason {
        card = card.child(
            label(reason, META, p.danger)
                .key(format!("{}-reason", dialog_key(action)))
                .wrap(),
        );
    }
    let mut cancel = secondary_button(cancel_key(action), "Cancel", p, close);
    // Delete focuses its name field instead: typing is the required step.
    if action != CaptainLifecycleAction::Delete {
        cancel = cancel.autofocus();
    }
    card = card.child(
        row()
            .pt(8.0)
            .gap(8.0)
            .child(spacer())
            .child(cancel)
            .child(confirm_button),
    );
    el().key(format!("{}-backdrop", dialog_key(action)))
        .instant_changes()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .padding(16.0)
        .fill(Color::BLACK.alpha(if s.dark { 0.45 } else { 0.28 }))
        .modal(false, close)
        .child(card)
}

fn dialog_title(s: &State, dialog: &CaptainLifecycleDialog) -> String {
    use crate::model::CaptainLifecycleAction as A;
    let name = s
        .captains
        .get(dialog.captain)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| "This captain".into());
    let crew = preview_ids(&dialog.preview, "crewSeen").len();
    match dialog.action {
        A::Archive if crew > 0 => format!("Archive {name} and its crew?"),
        A::Archive => format!("Archive {name}?"),
        A::Unarchive => format!("Unarchive {name}?"),
        A::Demote => format!("Make {name} an ordinary thread?"),
        A::Delete => format!("Delete {name}?"),
    }
}

/// A "Loading from {machine}." line while a linked machine's preview is still in flight.
fn loading_note(s: &State, p: Palette, dialog: &CaptainLifecycleDialog) -> Option<El> {
    dialog.preview.is_none().then(|| {
        quiet_note(
            format!("Loading from {}.", s.machine_label(&dialog.server)),
            p,
        )
    })
}

/// The body copy of each dialog: exact lines from the design, with counts read from the
/// preview so they never lie about what will change.
fn dialog_body(s: &State, p: Palette, dialog: &CaptainLifecycleDialog) -> El {
    use crate::model::CaptainLifecycleAction as A;
    let name = s
        .captains
        .get(dialog.captain)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| "This captain".into());
    let crew = preview_ids(&dialog.preview, "crewSeen");
    let routines = s.routines(dialog.captain).len();
    let mut body = col().gap(10.0);
    match dialog.action {
        A::Archive => {
            let intro = if crew.is_empty() {
                format!("{name} moves to the Archive.")
            } else {
                format!(
                    "{name} and these {} crew {} move to the Archive:",
                    crew.len(),
                    pluralize(crew.len(), "thread", "threads")
                )
            };
            body = body.child(label(intro, BODY, p.muted).wrap());
            if let Some(loading) = loading_note(s, p, dialog) {
                body = body.child(loading);
            }
            if !crew.is_empty() {
                let mut list = col().key("captain-archive-crew").gap(2.0);
                for id in crew.iter().take(6) {
                    list = list.child(archive_crew_row(s, p, *id));
                }
                if crew.len() > 6 {
                    list = list.child(
                        label(format!("and {} more", crew.len() - 6), META, p.muted)
                            .px(8.0)
                            .py(2.0),
                    );
                }
                body = body.child(list);
            }
            let pause = if routines > 0 {
                ", its routines pause,"
            } else {
                ""
            };
            body = body
                .child(
                    label(
                        format!(
                            "While archived, {name} doesn't run{pause} and crew reports wait. Nothing wakes {name} until you unarchive it."
                        ),
                        BODY,
                        p.muted,
                    )
                    .wrap(),
                )
                .child(
                    label(
                        "Kept: conversations, plan and rules, memory, scope, tasks and pull requests.",
                        BODY,
                        p.muted,
                    )
                    .wrap(),
                );
        }
        A::Unarchive => {
            let affected = preview_ids(&dialog.preview, "affectedThreads").len();
            let line = if affected > 0 {
                format!(
                    "{name} comes back, along with the {} archived with it. Paused routines resume; missed runs are skipped.",
                    pluralize(affected, "thread that was", "threads that were")
                )
            } else {
                format!("{name} comes back. Paused routines resume; missed runs are skipped.")
            };
            body = body.child(label(line, BODY, p.muted).wrap());
            if let Some(loading) = loading_note(s, p, dialog) {
                body = body.child(loading);
            }
        }
        A::Demote => {
            let workspace = s
                .captain_home(dialog.captain)
                .and_then(|id| s.threads.iter().find(|t| t.id == id))
                .and_then(|t| t.project)
                .and_then(|id| s.project(Some(id)))
                .map(|project| project.name.clone())
                .unwrap_or_else(|| "its workspace".into());
            let mut lines = vec![format!(
                "The current conversation stays where it is, as an ordinary thread in {workspace}."
            )];
            if !crew.is_empty() {
                lines.push(format!(
                    "Its {} crew {} released. They keep their conversations, tasks and pull requests and appear in your thread list.",
                    crew.len(),
                    pluralize(crew.len(), "thread is", "threads are")
                ));
            }
            lines.push(format!(
                "{name}'s profile, plan, rules, digest, memory and crew reports are kept as a record in the Archive under Former captains."
            ));
            if !s.earlier_conversations(dialog.captain).is_empty() {
                lines
                    .push("Earlier conversations move to the Archive as read-only threads.".into());
            }
            if routines > 0 {
                lines.push("Routines stop.".into());
            }
            for line in lines {
                body = body.child(label(line, BODY, p.muted).wrap());
            }
            if let Some(loading) = loading_note(s, p, dialog) {
                body = body.child(loading);
            }
        }
        A::Delete => {
            let homes = preview_ids(&dialog.preview, "homeThreads").len();
            body = body
                .child(meta_label("Removed for good", p.muted))
                .child(
                    label(
                        format!(
                            "{name}'s profile, plan, captain rules, crew rules, digest, memory and routines."
                        ),
                        BODY,
                        p.ink,
                    )
                    .wrap()
                    .pl(8.0),
                );
            if homes > 0 && !dialog.keep_conversations {
                body = body.child(
                    label(
                        format!(
                            "{} with {name}.",
                            pluralize(homes, "1 conversation", &format!("{homes} conversations"))
                        ),
                        BODY,
                        p.ink,
                    )
                    .wrap()
                    .pl(8.0),
                );
            }
            body = body.child(meta_label("Kept", p.muted));
            if !crew.is_empty() {
                body = body
                    .child(
                        label(
                            format!(
                                "{} crew {}, released into your thread list with their conversations, tasks and pull requests.",
                                crew.len(),
                                pluralize(crew.len(), "thread", "threads")
                            ),
                            BODY,
                            p.ink,
                        )
                        .wrap()
                        .pl(8.0),
                    )
                    .child(
                        label("Crew reports in those threads' history.", BODY, p.ink)
                            .wrap()
                            .pl(8.0),
                    );
            }
            if dialog
                .preview
                .as_ref()
                .and_then(|preview| preview["externalMemoryManagedSeparately"].as_bool())
                .unwrap_or(false)
            {
                body = body.child(
                    label(
                        "Learned memory on the memory service is managed there separately.",
                        META,
                        p.muted,
                    )
                    .wrap(),
                );
            }
            if let Some(loading) = loading_note(s, p, dialog) {
                body = body.child(loading);
            }
            let required_reason = dialog
                .preview
                .as_ref()
                .and_then(|preview| preview["keepConversationsRequiredReason"].as_str())
                .map(str::to_string);
            let keep = required_reason.is_some() || dialog.keep_conversations;
            let keep_label = format!(
                "Keep {} with {name} in the Archive",
                pluralize(homes, "1 conversation", &format!("{homes} conversations"))
            );
            let expected_server = dialog.server.clone();
            let expected_captain = dialog.captain;
            body = body.child(meta_label("Conversations", p.muted)).child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        switch(
                            s,
                            "captain-delete-keep-conversations",
                            &keep_label,
                            keep,
                            p,
                            move |s| {
                                if s.machines.selected == expected_server
                                    && let Some(dialog) = s.brief.lifecycle.as_mut()
                                    && dialog.server == expected_server
                                    && dialog.captain == expected_captain
                                    && dialog.pending.is_none()
                                {
                                    dialog.keep_conversations = !dialog.keep_conversations;
                                }
                            },
                        )
                        .disabled(required_reason.is_some() || dialog.pending.is_some())
                        .help(required_reason.clone().unwrap_or_else(|| {
                            format!("Keep {name}'s conversations as archived threads")
                        })),
                    )
                    .child(label(keep_label, BODY, p.ink).wrap().grow(1.0).min_w(0.0)),
            );
            if let Some(reason) = required_reason {
                body = body.child(label(reason, META, p.danger).wrap());
            }
            body = body
                .child(meta_label(&format!("Type \"{name}\" to confirm"), p.muted))
                .child(
                    text_input(&dialog.confirm_name, TextStyle::new(14.0, 400.0).lh(20.0))
                        .key("captain-delete-name")
                        .label("Captain name")
                        .color(p.ink)
                        .w_full()
                        .px(10.0)
                        .py(7.0)
                        .radius(6.0)
                        .fill(field_fill(s, p))
                        .field_focus(&dialog.confirm_name, p)
                        .single_line()
                        .autofocus()
                        .on_edit(|s: &mut State| {
                            &mut s
                                .brief
                                .lifecycle
                                .as_mut()
                                .expect("lifecycle dialog open")
                                .confirm_name
                        })
                        .on_submit(|s: &mut State, _| {
                            let ready = s.brief.lifecycle.as_ref().is_some_and(|dialog| {
                                let blockers = preview_blockers(&dialog.preview);
                                confirm_ready(s, dialog, &blockers).is_ok()
                            });
                            let Some(dialog) = &s.brief.lifecycle else {
                                return;
                            };
                            let (server, captain, action) =
                                (dialog.server.clone(), dialog.captain, dialog.action);
                            if ready {
                                confirm(s, &server, captain, action);
                            }
                        }),
                );
        }
    }
    body
}

/// The lifecycle dialog above the window, mounted from `captain_ui::dialogs`.
pub(super) fn dialogs(s: &State, p: Palette) -> Vec<El> {
    let Some(dialog) = s.brief.lifecycle.as_ref() else {
        return vec![];
    };
    let content = col()
        .gap(10.0)
        .child(label(dialog_title(s, dialog), DIALOG_TITLE, p.ink).wrap())
        .child(dialog_body(s, p, dialog));
    vec![lifecycle_dialog(s, p, dialog, content)]
}

/// Where an archived captain home's composer would be: it is read-only, with a way back.
/// `earlier_bar` geometry. KSG's archived thread shows a disabled composer; the group
/// archive has no KSG equivalent.
pub(super) fn archived_bar(s: &State, p: Palette, wide: bool, pane_width: f32) -> El {
    let captain = s.current().captain.filter(|_| s.current().captain_home);
    let name = captain
        .and_then(|captain| s.captains.get(captain))
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| "This captain".into());
    let server = s.machines.selected.clone();
    let reason = captain.and_then(|captain| {
        s.captain_lifecycle_unavailable(&server, captain, CaptainLifecycleAction::Unarchive)
    });
    let expected = server.clone();
    let unarchive = secondary_button("captain-archived-unarchive", "Unarchive", p, move |s| {
        if !expected_server(s, &expected) {
            return;
        }
        if let Some(captain) = captain
            && s.current().captain_home
            && s.current().captain == Some(captain)
            && let Err(error) =
                s.open_captain_lifecycle(&expected, captain, CaptainLifecycleAction::Unarchive)
        {
            s.brief.error = Some(error);
        }
    })
    .disabled(reason.is_some());
    let unarchive = match reason {
        Some(reason) => unarchive.help(reason),
        None => unarchive,
    };
    let bar = row()
        .key("captain-archived-bar")
        .role("status")
        .label(format!("{name} is archived, read-only"))
        .w(lane_width(pane_width, wide))
        .min_h(44.0)
        .px(14.0)
        .py(8.0)
        .gap(10.0)
        .center_y()
        .radius(CARD_RADIUS)
        .border(1.0, p.line)
        .child(icon("archive", p.muted))
        .child(
            label(
                format!("{name} is archived. It doesn't run or receive crew reports."),
                TextStyle::new(13.0, 500.0),
                p.muted,
            )
            .truncate()
            .grow(1.0)
            .min_w(0.0),
        )
        .child(unarchive);
    col()
        .key("captain-archived-lane")
        .w_full()
        .px(lane_gutter(pane_width))
        .pb(16.0)
        .items_center()
        .child(bar)
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
    fn inside(outer: [f32; 4], inner: [f32; 4]) -> bool {
        inner[0] >= outer[0] - 0.5
            && inner[1] >= outer[1] - 0.5
            && inner[0] + inner[2] <= outer[0] + outer[2] + 0.5
            && inner[1] + inner[3] <= outer[1] + outer[3] + 0.5
    }
    fn focused(rt: &Runtime<State>) -> rune::ElementInfo {
        rt.elements()
            .into_iter()
            .find(|e| e.focused)
            .expect("something has focus")
    }

    /// A demo captain home with its lifecycle dialog open and a seeded preview, so the
    /// dialog never touches a user profile or the network.
    fn lifecycle_runtime(
        width: f32,
        action: CaptainLifecycleAction,
        preview: Option<Value>,
    ) -> Runtime<State> {
        let mut rt = runtime(width);
        rt.state.populate_demo();
        rt.state.compact_sidebar = width < 760.0;
        rt.state.open_captain(0);
        let preview =
            preview.or_else(|| rt.state.captain_lifecycle_preview(0, action.action()).ok());
        rt.state.brief.lifecycle = Some(CaptainLifecycleDialog {
            server: "local".into(),
            captain: 0,
            action,
            preview,
            pending: None,
            error: None,
            confirm_name: rune::TextEdit::default(),
            keep_conversations: true,
        });
        rt.frame(0.5);
        rt
    }

    /// A linked test machine "studio" negotiating `capabilities`, and the receiver its
    /// commands land in. Mirrors the state tests' `linked`.
    fn linked(
        state: &mut State,
        capabilities: &[&str],
    ) -> tokio::sync::mpsc::Receiver<crate::protocol::Command> {
        let (mut client, _local) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, commands) = crate::client::Client::test_connected(state.data_dir.clone());
        *remote.info.lock().unwrap() = serde_json::json!({"capabilities": capabilities});
        client.linked.insert("studio".into(), remote);
        state.client = Some(client);
        commands
    }

    #[test]
    fn the_lifecycle_dialogs_fit_and_focus_cancel_at_supported_widths() {
        for action in [
            CaptainLifecycleAction::Archive,
            CaptainLifecycleAction::Unarchive,
            CaptainLifecycleAction::Demote,
        ] {
            for width in WIDTHS {
                let rt = lifecycle_runtime(width, action, None);
                let key = dialog_key(action);
                let (dialog, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    dialog[0] >= 0.0 && dialog[0] + dialog[2] <= width + 0.5,
                    "{key} at {width}: {dialog:?}"
                );
                for key in [cancel_key(action), confirm_key(action)] {
                    let (rect, _) = rt
                        .presented(key)
                        .unwrap_or_else(|| panic!("{key} at {width}"));
                    assert!(inside(dialog, rect), "{key} at {width}");
                }
                assert_eq!(
                    focused(&rt).key.as_deref(),
                    Some(cancel_key(action)),
                    "{key} at {width}: Cancel opens focused"
                );
            }
        }
    }

    #[test]
    fn the_archive_dialog_lists_the_preview_crew() {
        for width in WIDTHS {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.compact_sidebar = width < 760.0;
            rt.state.open_captain(0);
            let crew: Vec<usize> = rt.state.threads.iter().take(3).map(|t| t.id).collect();
            rt.state.brief.lifecycle = Some(CaptainLifecycleDialog {
                server: "local".into(),
                captain: 0,
                action: CaptainLifecycleAction::Archive,
                preview: Some(serde_json::json!({
                    "captainVersion": "v1",
                    "crewSeen": crew,
                    "affectedThreads": [],
                    "blockers": [],
                })),
                pending: None,
                error: None,
                confirm_name: rune::TextEdit::default(),
                keep_conversations: true,
            });
            rt.frame(0.5);
            let (dialog, _) = rt.presented("captain-archive-dialog").unwrap();
            assert!(
                rt.presented("captain-archive-crew").is_some(),
                "{width}: the crewSeen list is presented"
            );
            let (rect, _) = rt.presented("captain-archive-crew").unwrap();
            assert!(inside(dialog, rect), "{width}");
            // No private document text may appear in any dialog.
            for element in rt.elements() {
                assert!(
                    !element.text.as_deref().unwrap_or("").contains("AGENTS.md")
                        && !element.label.as_deref().unwrap_or("").contains("AGENTS.md"),
                    "{width}: private document text must not render"
                );
            }
        }
    }

    #[test]
    fn the_delete_dialog_needs_the_exact_typed_name() {
        for width in WIDTHS {
            let mut rt = lifecycle_runtime(
                width,
                CaptainLifecycleAction::Delete,
                Some(serde_json::json!({
                    "captainVersion": "v1",
                    "crewSeen": [],
                    "affectedThreads": [],
                    "blockers": [],
                    "homeThreads": [],
                    "canDeleteConversations": true,
                })),
            );
            let mut _commands = linked(&mut rt.state, &[crate::protocol::CAPTAIN_DELETE]);
            rt.state.machines.selected = "studio".into();
            rt.state.brief.lifecycle.as_mut().unwrap().server = "studio".into();
            rt.frame(0.5);
            let (dialog, _) = rt.presented("captain-delete-dialog").unwrap();
            for key in [
                "captain-delete-keep-conversations",
                "captain-delete-name",
                "cancel-delete-captain",
                "confirm-delete-captain",
            ] {
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(inside(dialog, rect), "{key} at {width}");
            }
            assert_eq!(
                focused(&rt).key.as_deref(),
                Some("captain-delete-name"),
                "{width}: the name field opens focused"
            );
            let name = rt.state.captains[0].name.clone();
            let lower = name.to_lowercase();
            rt.state
                .brief
                .lifecycle
                .as_mut()
                .unwrap()
                .confirm_name
                .replace_text(&lower);
            rt.frame(0.25);
            press(&mut rt, "confirm-delete-captain");
            assert_eq!(
                rt.state.captain_phase(0),
                CaptainPhase::Active,
                "{width}: a wrong-case name does not delete"
            );
            rt.state
                .brief
                .lifecycle
                .as_mut()
                .unwrap()
                .confirm_name
                .replace_text(&name);
            rt.frame(0.25);
            tap(&mut rt, rune::Key::Enter);
            let dialog =
                rt.state.brief.lifecycle.as_ref().unwrap_or_else(|| {
                    panic!("{width}: Return with the exact name sends the command")
                });
            assert!(
                dialog.pending.is_some(),
                "{width}: the exact name sends captain.delete and waits on its receipt"
            );
            let sent = _commands.try_recv().expect("captain.delete was sent");
            assert_eq!(sent.method, "captain.delete");
            assert_eq!(sent.params["expectedVersion"], "v1");
        }
    }

    #[test]
    fn the_captain_section_keeps_reset_and_adds_lifecycle_rows() {
        for width in WIDTHS {
            let mut rt = runtime(width);
            rt.state.populate_demo();
            rt.state.compact_sidebar = width < 760.0;
            rt.state.open_captain(0);
            rt.frame(0.0);
            let key = rt.state.panel_key();
            rt.state
                .right_panels
                .open(&key, crate::right_panel::SingletonKind::Overview);
            rt.invalidate();
            rt.frame(1.0);
            let (panel, _) = rt.presented("captain-brief").unwrap();
            for key in [
                "brief-conversation",
                "brief-reset",
                "brief-archive-captain",
                "brief-demote",
                "brief-delete-captain",
            ] {
                rt.update(|_, tx| tx.scroll_into_view(key.to_string()));
                rt.frame(0.5);
                let (rect, _) = rt
                    .presented(key)
                    .unwrap_or_else(|| panic!("{key} at {width}"));
                assert!(
                    rect[0] >= panel[0] && rect[0] + rect[2] <= panel[0] + panel[2] + 0.5,
                    "{key} at {width}"
                );
            }
        }
    }
}
