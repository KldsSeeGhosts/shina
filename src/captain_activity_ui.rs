//! The Brief's automatic activity digest and the compact report-policy block.
//!
//! KSG Code has no Captain equivalent for either, so these adapt its
//! `ThreadDetailsSection` structure (the shared `section`/`quiet_note`/`icon_action`
//! helpers), its `ThreadDetailsControl` rows and its segmented `ToggleGroup`
//! (already ported as the Keep awake control) rather than porting a component.
//! The digest is a read-only, source-backed view: it never writes DIGEST.md,
//! never acknowledges a delivery and never runs a local path or artifact.
//! Entries open their source thread on the pinned machine only; a needs-you
//! crew question goes to the question card on the Captain's home, not the
//! worker's composer, and Ignore policy never hides a genuine human question.
use super::captain_ui::{SHEET_BELOW, close_brief, icon_action, quiet_note};
use super::crew_questions_ui::{delivery_words, outcome_status, outcome_word, policy_badge};
use super::crew_ui::{active_wash, hover_wash, motion};
use super::*;
use crate::model::crew_reports::QuestionRecipient;
use crate::model::{RecoveryMode, ReportPolicy};
use serde_json::Value;

/// KSG's `text-[13px] font-medium` detail rows, matching `captain_ui::ROW_TEXT`.
const ROW_TEXT: TextStyle = TextStyle::new(13.0, 470.0).lh(19.0).tracking(-0.08);
/// The quiet meta line under a row's title.
const META: TextStyle = TextStyle::new(11.5, 430.0).lh(16.0);

/// How many entries each group shows before its Show more button takes over:
/// Needs you 3, What's new 2, In progress 3.
fn bound(group: &str) -> usize {
    match group {
        "whats-new" => 2,
        _ => 3,
    }
}

/// The report's captured policy as the row needs it; anything missing or
/// unrecognised renders as React, the durable default an old host always holds.
fn entry_policy(entry: &Value) -> ReportPolicy {
    match entry["reportPolicy"].as_str() {
        Some("context") => ReportPolicy::Context,
        Some("ignore") => ReportPolicy::Ignore,
        _ => ReportPolicy::React,
    }
}

/// The status word for one entry and the Status that colours it, in the
/// vocabulary `crew_ui` and `crew_questions_ui` already use for the same facts.
fn status_word(entry: &Value) -> (String, Status) {
    let status = entry["status"].as_str().unwrap_or("");
    match entry["kind"].as_str().unwrap_or("") {
        "crew_question" => {
            if status == "answer_pending" {
                // A saved-but-not-accepted answer is not an open question
                // asking again; the source still has to confirm it.
                ("Answer saved".into(), Status::Ready)
            } else {
                ("Asked you".into(), Status::Waiting)
            }
        }
        "provider_approval" => ("Approval".into(), Status::Waiting),
        "provider_question" => ("Question".into(), Status::Waiting),
        "crew_report" => {
            let outcome = match status {
                "completed" => crate::model::crew_reports::CrewOutcome::Completed,
                "failed" => crate::model::crew_reports::CrewOutcome::Failed,
                "blocked" => crate::model::crew_reports::CrewOutcome::Blocked,
                "cancelled" => crate::model::crew_reports::CrewOutcome::Cancelled,
                _ => crate::model::crew_reports::CrewOutcome::Progress,
            };
            (outcome_word(outcome).to_string(), outcome_status(outcome))
        }
        "crew_thread" => {
            if status == "queued" {
                ("Queued".into(), Status::Ready)
            } else {
                ("Working".into(), Status::Running)
            }
        }
        "delegated_task" => match status {
            "queued" => ("Queued".into(), Status::Ready),
            "starting" => ("Starting".into(), Status::Running),
            _ => ("Working".into(), Status::Running),
        },
        _ => (status.to_string(), Status::Ready),
    }
}

/// The detail that follows the status word: what the record proves about
/// delivery and answers, or where the request waits. Never an inference about
/// hold state; a Context report only ever reads "Waiting to reach".
fn status_detail(s: &State, entry: &Value, captain_name: &str) -> String {
    let status = entry["status"].as_str().unwrap_or("");
    match entry["kind"].as_str().unwrap_or("") {
        "crew_question" => {
            if status == "answer_pending" {
                "Not yet accepted".into()
            } else {
                delivery_words(
                    entry["reportDelivered"].as_bool().unwrap_or(false),
                    entry_policy(entry),
                    captain_name,
                )
            }
        }
        "provider_approval" | "provider_question" => {
            if status == "remote" {
                let label = entry["threadId"]
                    .as_u64()
                    .and_then(|thread| s.remote_task(thread as usize))
                    .map(|remote| remote.label.clone())
                    .unwrap_or_else(|| {
                        entry["summary"]
                            .as_str()
                            .unwrap_or("")
                            .trim_start_matches("Needs you on ")
                            .to_string()
                    });
                format!("On {label}")
            } else {
                "Open the thread to answer".into()
            }
        }
        "crew_report" => {
            let mut detail = delivery_words(
                entry["reportDelivered"].as_bool().unwrap_or(false),
                entry_policy(entry),
                captain_name,
            );
            if entry["answerAccepted"].as_bool() == Some(true) {
                detail.push_str(", answer accepted");
            } else if entry["answerSaved"].as_bool() == Some(true) {
                detail.push_str(", answer saved, not yet accepted");
            }
            detail
        }
        _ => String::new(),
    }
}

/// The status line under a row: the pixel word in `status_ink`, the Inter
/// detail that truncates first, the age that never shrinks, and the policy
/// badge the captured snapshot earned. No middle-dot joins.
fn status_line(s: &State, p: Palette, entry: &Value, tag_key: String, captain_name: &str) -> El {
    let (word, status) = status_word(entry);
    let detail = status_detail(s, entry, captain_name);
    let mut line = row()
        .gap(8.0)
        .center_y()
        .h(16.0)
        .child(meta_text(word, status_ink(status, p)));
    if !detail.is_empty() {
        line = line.child(label(detail, META, p.muted).truncate().shrink().min_w(0.0));
    }
    line = line.child(spacer());
    if let Some(age) = entry["createdAt"]
        .as_u64()
        .filter(|at| *at > 0)
        .and_then(|at| relative_time(at / 1000))
    {
        line = line.child(meta_text(age, p.faint));
    }
    if let Some(badge) = policy_badge(tag_key, entry_policy(entry), p) {
        line = line.child(badge);
    }
    line
}

/// Whether the entry's source thread still exists on the pinned machine:
/// either in the selected view's threads or routable through a linked remote
/// task, exactly like the needs-you cards' Open button.
fn source_known(s: &State, server: &str, thread: usize) -> bool {
    if s.machines.selected != server || s.client.is_some() && !s.machine_connected(server) {
        return false;
    }
    if s.threads.iter().any(|t| t.id == thread) {
        return true;
    }
    s.remote_task(thread).is_some_and(|remote| {
        remote.thread_id.is_some_and(|id| {
            s.client
                .as_ref()
                .is_some_and(|c| c.linked.contains_key(&remote.server_id))
                && s.machine_connected(&remote.server_id)
                && s.machines.views.get(&remote.server_id).is_some_and(|view| {
                    view["index"]["threads"]
                        .as_array()
                        .is_some_and(|threads| threads.iter().any(|t| t["id"] == id))
                })
        })
    })
}

/// Open the entry's source thread, routing a remote worker through its machine
/// link. `focus` drops the caret in the composer for the provider requests
/// that wait on a typed answer. Local paths and artifacts never execute.
fn open_thread_source(s: &mut State, server: String, thread: usize, focus: bool) {
    if s.machines.selected != server {
        return;
    }
    if let Some(remote) = s.remote_task(thread).cloned()
        && let Some(id) = remote.thread_id
        && s.client
            .as_ref()
            .is_some_and(|c| c.linked.contains_key(&remote.server_id))
        && s.machines.views.get(&remote.server_id).is_some_and(|view| {
            view["index"]["threads"]
                .as_array()
                .is_some_and(|threads| threads.iter().any(|t| t["id"] == id))
        })
    {
        s.open_machine_thread(&remote.server_id, id);
        return;
    }
    if let Some(index) = s.threads.iter().position(|t| t.id == thread) {
        s.show_worker_ghost(index);
        if focus {
            s.current_mut().reply.focused = true;
        }
    }
}

/// Open the entry's source. The sheet closes when it covers the chat or a
/// retired record sheet is open. A crew question goes to its question card on
/// the Captain's primary home — an answer typed in the worker's composer is
/// not an answer record — and falls back to the worker thread when the card is
/// not drawn (answer saved, or the Captain is archived). Provider requests go
/// to the worker with composer focus; reports and busy workers just open.
fn open_activity_source(s: &mut State, tx: &mut Tx, server: String, captain: usize, entry: Value) {
    let thread = entry["threadId"].as_u64().unwrap_or(0) as usize;
    if !source_known(s, &server, thread) {
        return;
    }
    if s.brief.record.is_some() || s.viewport_width < SHEET_BELOW {
        close_brief(s);
    }
    let kind = entry["kind"].as_str().unwrap_or("");
    if kind == "crew_question" {
        let rid = entry["reportId"].as_str().unwrap_or("").to_string();
        let card_drawn = !rid.is_empty()
            && s.captain_phase(captain) == crate::model::CaptainPhase::Active
            && s.captain_home(captain).is_some()
            && s.open_crew_questions(captain, QuestionRecipient::User)
                .iter()
                .any(|view| view.report_id == rid);
        if card_drawn {
            let home = s.captain_home(captain).expect("home checked above");
            if s.current().id != home
                && let Some(index) = s.threads.iter().position(|t| t.id == home)
            {
                s.show_thread(index);
            }
            tx.scroll_into_view(format!("needs-you-report-{rid}"));
            tx.focus(format!("needs-you-report-{rid}-text"), true);
            return;
        }
    }
    let focus = matches!(kind, "provider_approval" | "provider_question");
    open_thread_source(s, server, thread, focus);
}

/// One needs-you or what's-new row: title, one summary line when the record
/// carries one, then the status line. Pressable only while its source exists
/// on the pinned machine.
fn entry_row(
    s: &State,
    p: Palette,
    group: &str,
    entry: &Value,
    server: &str,
    captain: usize,
    captain_name: &str,
) -> El {
    let id = entry["id"].as_str().unwrap_or_default();
    let title = entry["title"].as_str().unwrap_or("Untitled");
    let thread = entry["threadId"].as_u64().unwrap_or(0) as usize;
    let known = source_known(s, server, thread);
    let key = format!("brief-activity-{group}-{id}");
    let summary = entry["summary"].as_str().unwrap_or("").replace('\n', " ");
    let summary = summary.trim();
    let (word, _) = status_word(entry);
    let detail = status_detail(s, entry, captain_name);
    let age = entry["createdAt"]
        .as_u64()
        .filter(|at| *at > 0)
        .and_then(|at| relative_time(at / 1000));
    let mut a11y = format!("{title}, {word}");
    if !detail.is_empty() {
        a11y.push_str(&format!(", {detail}"));
    }
    if let Some(age) = &age {
        a11y.push_str(&format!(", {age}"));
    }
    let mut row = col()
        .key(key)
        .px(10.0)
        .py(5.0)
        .gap(2.0)
        .radius(6.0)
        .child(label(title, ROW_TEXT, p.ink).truncate().min_w(0.0).w_full());
    if !summary.is_empty() {
        row = row.child(
            label(summary.to_string(), META, p.muted)
                .truncate()
                .w_full(),
        );
    }
    row = row.child(status_line(
        s,
        p,
        entry,
        format!("brief-activity-{group}-{id}-tag"),
        captain_name,
    ));
    if known {
        let machine = server.to_string();
        let entry = entry.clone();
        row = row
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("{a11y}. Open"))
            .focusable(true)
            .hover(hover_wash(s, p))
            .press(active_wash(s, p))
            .hover_motion(motion(), motion())
            .on_press_tx(move |s: &mut State, tx| {
                open_activity_source(s, tx, machine.clone(), captain, entry.clone())
            });
    } else {
        row = row
            .role("group")
            .label(format!("{a11y}, source not on this machine"));
    }
    row
}

/// One in-progress row: a single 28 px line, the title truncating and the
/// pixel status word at its end. The Crew section carries the detail.
fn in_progress_row(
    s: &State,
    p: Palette,
    group: &str,
    entry: &Value,
    server: &str,
    captain: usize,
) -> El {
    let id = entry["id"].as_str().unwrap_or_default();
    let title = entry["title"].as_str().unwrap_or("Untitled");
    let thread = entry["threadId"].as_u64().unwrap_or(0) as usize;
    let known = source_known(s, server, thread);
    let (word, status) = status_word(entry);
    let mut row = row()
        .key(format!("brief-activity-{group}-{id}"))
        .px(10.0)
        .h(28.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .child(label(title, ROW_TEXT, p.ink).truncate().shrink().min_w(0.0))
        .child(spacer())
        .child(meta_text(word.clone(), status_ink(status, p)));
    if known {
        let machine = server.to_string();
        let entry = entry.clone();
        row = row
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("{title}, {word}. Open"))
            .focusable(true)
            .hover(hover_wash(s, p))
            .press(active_wash(s, p))
            .hover_motion(motion(), motion())
            .on_press_tx(move |s: &mut State, tx| {
                open_activity_source(s, tx, machine.clone(), captain, entry.clone())
            });
    } else {
        row = row
            .role("group")
            .label(format!("{title}, {word}, source not on this machine"));
    }
    row
}

/// One bounded group: its sub-heading with the exact total, up to `bound`
/// rows, a Show more/fewer button, and the digest's own truncation note once
/// nothing the digest returned stays hidden.
fn group(
    s: &State,
    p: Palette,
    key: &'static str,
    title: &str,
    entries: &[Value],
    truncated: usize,
    server: &str,
    captain: usize,
    captain_name: &str,
) -> El {
    if entries.is_empty() {
        return el().key(format!("brief-activity-{key}"));
    }
    let total = entries.len() + truncated;
    let open = s
        .brief
        .activity_open
        .contains(&(server.to_string(), captain, key));
    let max = bound(key);
    let shown = if open {
        entries.len()
    } else {
        entries.len().min(max)
    };
    let mut group = col()
        .key(format!("brief-activity-{key}"))
        .role("group")
        .label(title)
        .gap(1.0)
        .child(
            row()
                .h(20.0)
                .px(10.0)
                .center_y()
                .child(meta_label(title, p.muted))
                .child(spacer())
                .child(meta_text(total.to_string(), p.faint)),
        );
    for entry in entries.iter().take(shown) {
        group = group.child(if key == "in-progress" {
            in_progress_row(s, p, key, entry, server, captain)
        } else {
            entry_row(s, p, key, entry, server, captain, captain_name)
        });
    }
    let hidden = entries.len() - shown;
    let expandable = open || hidden > 0;
    if expandable {
        let more_key = format!("brief-activity-{key}-more");
        let machine = server.to_string();
        let word = if open {
            "Show fewer".to_string()
        } else {
            format!("Show {hidden} more")
        };
        group = group.child(
            row()
                .key(more_key.clone())
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(word.clone())
                .focusable(true)
                .px(10.0)
                .min_h(28.0)
                .center_y()
                .radius(6.0)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press_tx(move |s: &mut State, tx| {
                    let key_at = (machine.clone(), captain, key);
                    if s.brief.activity_open.contains(&key_at) {
                        s.brief.activity_open.remove(&key_at);
                        // Focus inside a row the collapse hides lands on the button.
                        let prefix = format!("brief-activity-{key}-");
                        let focused_is_row = tx.focused().is_some_and(|focused| {
                            focused.starts_with(&prefix)
                                && focused != more_key
                                && !focused.ends_with("-tag")
                        });
                        if focused_is_row {
                            tx.focus(more_key.clone(), true);
                        }
                    } else {
                        s.brief.activity_open.insert(key_at);
                    }
                })
                .child(label(word, META, p.muted)),
        );
    }
    if truncated > 0 && (open || hidden == 0) {
        group = group.child(
            label(
                format!("{truncated} older not listed. Open a crew thread for its full history."),
                META,
                p.faint,
            )
            .key(format!("brief-activity-{key}-truncated"))
            .px(10.0)
            .py(2.0),
        );
    }
    group
}

/// The automatic activity digest for `captain`, mounted inside the Digest
/// section above the manual DIGEST.md preview. The typed digest is converted
/// to a `Value` at the seam so this view depends on the snapshot's fields, not
/// on the record types. Needs-you, what's-new and in-progress groups render
/// only while they hold entries.
pub(super) fn activity(s: &State, p: Palette, captain: usize) -> El {
    let digest = s
        .captain_activity_digest(captain)
        .ok()
        .and_then(|digest| serde_json::to_value(digest).ok())
        .unwrap_or_default();
    let server = digest["server"].as_str().unwrap_or_default().to_string();
    let captain_name = s
        .captains
        .get(captain)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "the captain".into());
    let entries = |name: &str| digest[name].as_array().cloned().unwrap_or_default();
    let needs_you = entries("needsYou");
    let whats_new = entries("whatsNew");
    let in_progress = entries("inProgress");
    if needs_you.is_empty() && whats_new.is_empty() && in_progress.is_empty() {
        return quiet_note(
            format!("No crew activity yet. Reports and questions from {captain_name}'s crew appear here."),
            p,
        )
        .key("brief-activity-empty");
    }
    col()
        .key("brief-activity")
        .role("group")
        .label("Crew activity")
        .gap(4.0)
        .pb(4.0)
        .child(group(
            s,
            p,
            "needs-you",
            "Needs you",
            &needs_you,
            digest["truncated"]["needsYou"].as_u64().unwrap_or(0) as usize,
            &server,
            captain,
            &captain_name,
        ))
        .child(group(
            s,
            p,
            "whats-new",
            "What's new",
            &whats_new,
            digest["truncated"]["whatsNew"].as_u64().unwrap_or(0) as usize,
            &server,
            captain,
            &captain_name,
        ))
        .child(group(
            s,
            p,
            "in-progress",
            "In progress",
            &in_progress,
            digest["truncated"]["inProgress"].as_u64().unwrap_or(0) as usize,
            &server,
            captain,
            &captain_name,
        ))
}

/// Whether the digest has any activity at all, so the saved preview can take
/// its taller twelve-line bound only when nothing sits above it.
pub(super) fn activity_has_entries(s: &State, captain: usize) -> bool {
    s.captain_activity_digest(captain)
        .map(|digest| {
            !(digest.needs_you.is_empty()
                && digest.whats_new.is_empty()
                && digest.in_progress.is_empty())
        })
        .unwrap_or(false)
}

/// The policy segments, in order.
const POLICY_SEGMENTS: [(&str, ReportPolicy); 3] = [
    ("captain-report-policy-react", ReportPolicy::React),
    ("captain-report-policy-context", ReportPolicy::Context),
    ("captain-report-policy-ignore", ReportPolicy::Ignore),
];

/// Send one policy change on the pinned machine; the refusal lands on the
/// block's own error row, never a toast.
fn choose(s: &mut State, expected: &str, captain: usize, choice: ReportPolicy) {
    if s.machines.selected != expected || s.captain_report_policy(captain) == choice {
        return;
    }
    if let Err(error) = s.change_captain_report_policy(expected, captain, choice) {
        s.captain_policy_errors
            .insert((expected.to_string(), captain), error);
    }
}

/// The report-policy block: one segmented row inside the Crew section choosing
/// how future crew reports reach this captain's home. React wakes it on
/// arrival, Context holds reports until the next explicit turn and Ignore
/// keeps them without ever injecting. The choice never rewrites queued or
/// outstanding reports and never touches open questions. Unlike the Keep
/// awake control, arrows move focus only: each selection is a remote command,
/// so Left and Right must not queue several CAS writes on the way past.
pub(super) fn policy_block(s: &State, p: Palette, captain: usize) -> El {
    let server = s.machines.selected.clone();
    let name = s
        .captains
        .get(captain)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "the captain".into());
    let current = s.captain_report_policy(captain);
    let pending = s
        .captain_policy_pending
        .iter()
        .find(|((host, _), change)| *host == server && change.captain == captain)
        .map(|(_, change)| *change);
    let reason = s.captain_policy_unavailable(&server, captain);
    // A peer that cannot hold the capability gets the note only: drawing three
    // dead segments would offer a choice the host cannot take.
    let old_peer = reason
        .as_deref()
        .is_some_and(|reason| reason.starts_with("Update Shina on "));
    let shown = pending.map(|change| change.policy).unwrap_or(current);
    let machine = s.machine_label(&server);
    let mut block = col()
        .key("brief-report-policy")
        .role("group")
        .label("Crew reports")
        .px(10.0)
        .pt(6.0)
        .gap(4.0);
    if let Some(error) = s.captain_policy_errors.get(&(server.clone(), captain)) {
        let server = server.clone();
        block = block.child(
            row()
                .key("captain-report-policy-error")
                .role("alert")
                .px(10.0)
                .py(6.0)
                .gap(8.0)
                .radius(12.0)
                .fill(p.danger.alpha(0.08))
                .child(icon("alert", p.danger))
                .child(label(error, META, p.danger).wrap().grow(1.0).min_w(0.0))
                .child(icon_action(
                    s,
                    "dismiss-policy-error".into(),
                    "Dismiss",
                    icon("close", p.danger),
                    p,
                    move |s| {
                        s.captain_policy_errors.remove(&(server.clone(), captain));
                    },
                )),
        );
    }
    if old_peer {
        return block.child(
            quiet_note(
                format!("Update Shina on {machine} to choose how crew reports arrive."),
                p,
            )
            .key("captain-report-policy-reason"),
        );
    }
    // Below a 300 px panel the label drops and the group takes the row.
    let panel = if s.viewport_width < SHEET_BELOW {
        (s.viewport_width - 48.0).clamp(280.0, 360.0)
    } else {
        super::captain_ui::PANEL_WIDTH
    };
    let narrow = panel < 300.0;
    let disabled = reason.is_some() || pending.is_some();
    let expected = server.clone();
    let mut group = row()
        .key("captain-report-policy")
        .role("radiogroup")
        .focus_group()
        .label("Crew reports")
        .padding(2.0)
        .gap(2.0)
        .radius(6.0)
        .fill(p.ink.alpha(if s.dark { 0.07 } else { 0.06 }))
        .border(1.0, p.line)
        .w(220.0)
        .shrink()
        .max_w(220.0)
        .on_key(move |s: &mut State, ev: &rune::KeyEvent, tx: &mut Tx| {
            let index = |focused: Option<&str>| {
                POLICY_SEGMENTS
                    .iter()
                    .position(|(key, _)| Some(*key) == focused)
            };
            let step = match ev.key {
                rune::Key::Left | rune::Key::Up => -1i64,
                rune::Key::Right | rune::Key::Down => 1,
                rune::Key::Enter => {
                    if let Some(index) = index(tx.focused()) {
                        let server = expected.clone();
                        choose(s, &server, captain, POLICY_SEGMENTS[index].1);
                    }
                    return true;
                }
                _ => return false,
            };
            let next = (index(tx.focused()).unwrap_or(0) as i64 + step)
                .rem_euclid(POLICY_SEGMENTS.len() as i64) as usize;
            tx.focus(POLICY_SEGMENTS[next].0, true);
            true
        });
    for (key, choice) in POLICY_SEGMENTS {
        let on = shown == choice;
        let word = match choice {
            ReportPolicy::React => "React",
            ReportPolicy::Context => "Context",
            ReportPolicy::Ignore => "Ignore",
        };
        let a11y = match choice {
            ReportPolicy::React => {
                format!("Crew reports: React, wake {name} when a report arrives")
            }
            ReportPolicy::Context => {
                format!("Crew reports: Context, keep reports for {name}'s next turn")
            }
            ReportPolicy::Ignore => {
                format!("Crew reports: Ignore, keep reports without sending them")
            }
        };
        let expected = server.clone();
        let mut segment = row()
            .key(key)
            .role("radio")
            .checked(on)
            .label(a11y)
            .cursor(rune::Cursor::Pointer)
            .focusable(true)
            .h(24.0)
            .grow(1.0)
            .center()
            .radius(6.0)
            .fill(if on { p.card } else { Color::TRANSPARENT })
            // The chosen segment has a 1 px frame. Idle ones keep a clear
            // border so no segment changes size when the choice moves.
            .border(1.0, if on { p.frame } else { Color::TRANSPARENT })
            .hover(if on { p.card } else { hover_wash(s, p) })
            .hover_motion(motion(), motion())
            .on_press(move |s: &mut State| choose(s, &expected, captain, choice))
            .child(label(
                word,
                TextStyle::new(12.0, 500.0),
                if on { p.ink } else { p.muted },
            ));
        if let Some(reason) = &reason {
            segment = segment.help(reason.clone());
        }
        group = group.child(segment.disabled(disabled));
    }
    let mut control = row().h(28.0).gap(8.0).center_y();
    if !narrow {
        control = control.child(meta_label("Reports", p.muted));
    }
    control = control.child(spacer()).child(group);
    block = block.child(control);
    if pending.is_some() {
        block = block.child(meta_label("Updating", p.faint).key("captain-report-policy-pending"));
    } else if let Some(reason) = reason {
        block = block.child(quiet_note(reason, p).key("captain-report-policy-reason"));
    } else {
        let note = match current {
            ReportPolicy::React => format!(
                "New reports wake {name} when they arrive. Earlier reports keep their setting."
            ),
            ReportPolicy::Context => format!(
                "New reports wait until you message {name}. They still need delivery before crew can leave."
            ),
            ReportPolicy::Ignore => format!(
                "New reports are kept here but never sent to {name}. Questions for you still show."
            ),
        };
        block = block.child(quiet_note(note, p).key("captain-report-policy-note"));
    }
    block
}

/// Recovery has no KSG counterpart; use its ThreadDetailsSection and
/// ThreadDetailsControl pattern already used by the Brief's Crew section.
pub(super) fn recovery_block(s: &State, p: Palette, captain: usize) -> El {
    let server = s.machines.selected.clone();
    let current = s.captain_recovery_mode(captain);
    let (mode, description, action, next) = match current {
        RecoveryMode::Attended => (
            "Attended",
            "After restart, held events wait for your next message or an explicit resume.",
            "Allow unattended recovery",
            RecoveryMode::Unattended,
        ),
        RecoveryMode::Unattended => (
            "Unattended",
            "After restart, eligible events can resume once workers are reconciled. Approvals and uncertain work still wait.",
            "Wait for me after restart",
            RecoveryMode::Attended,
        ),
    };
    let mut block = col()
        .key("brief-captain-recovery")
        .role("group")
        .label("Restart recovery")
        .px(10.0)
        .pt(8.0)
        .gap(4.0)
        .child(
            row()
                .center_y()
                .child(meta_label("Restart recovery", p.muted))
                .child(spacer())
                .child(label(mode, META, p.ink).key("captain-recovery-mode")),
        )
        .child(quiet_note(description, p));
    let reason = s.captain_recovery_unavailable(&server, captain);
    let error = s.captain_recovery_errors.get(&(server.clone(), captain));
    if let Some(error) = error {
        block = block.child(label(error, META, p.danger).wrap().role("alert"));
    }
    if let Some(reason) = reason {
        block = block.child(quiet_note(reason, p).key("captain-recovery-unavailable"));
    } else {
        block = block.child(row().child(super::crew_ui::ghost_button(
            s,
            "captain-recovery-change".into(),
            action,
            p,
            move |s| {
                if s.machines.selected == server
                    && let Err(error) = s.change_captain_recovery(&server, captain, current, next)
                {
                    s.captain_recovery_errors
                        .insert((server.clone(), captain), error);
                }
            },
        )));
    }
    let mut waits = s.active_captain_wait_reasons(captain);
    for (index, reason) in waits.by_ref().take(3).enumerate() {
        block = block.child(
            row()
                .key(format!("captain-recovery-wait-{index}"))
                .gap(6.0)
                .child(meta_label("Waiting", p.brass))
                .child(label(reason, META, p.ink).wrap().grow(1.0).min_w(0.0)),
        );
    }
    let remaining = waits.count();
    if remaining > 0 {
        block = block.child(quiet_note(format!("{remaining} more active waits"), p));
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::runtime;

    /// A captain home with one busy crew thread and one approval request, so
    /// the in-progress and needs-you groups have rows to draw.
    fn brief(rt: &mut rune::Runtime<State>) {
        // Reports must flush, which a demo session refuses; the workspace shape
        // is the demo's, but the stores are real.
        rt.state.data_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        rt.state.load_global_for_test();
        crate::captain_brief_demo(&mut rt.state);
        let home = rt.state.current().id;
        let captain = rt.state.current().captain.unwrap();
        for i in 0..4 {
            let id = rt.state.next_id;
            rt.state.next_id += 1;
            let mut thread = Thread::new(id);
            thread.crew_captain = Some(captain);
            thread.captain = Some(captain);
            thread.parent = Some(home);
            thread.title = format!("Crew worker {i}");
            thread.apply(json!({"type":"agent_start"}));
            rt.state.threads.push(thread);
        }
        let id = rt.state.next_id;
        rt.state.next_id += 1;
        // A provider request routes to the home through the plain parent
        // fallback; giving it crew provenance would demand canonical
        // membership instead.
        let mut asking = Thread::new(id);
        asking.parent = Some(home);
        asking.title = "Deploy worker".into();
        asking.request = Some(json!({"method":"confirm","title":"Allow deploy?","message":""}));
        rt.state.threads.push(asking);
        rt.state.show_activity = true;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
    }

    /// Scroll a Brief key into its viewport before pressing it: the sheet is
    /// 792 px tall at 620 and later sections sit below the first screen.
    fn press_in_brief(rt: &mut rune::Runtime<State>, key: &str) {
        rt.update(|_, tx| tx.scroll_into_view(key.to_string()));
        rt.frame(0.0);
        let (rect, _) = rt
            .presented(key)
            .unwrap_or_else(|| panic!("{key} is not on screen"));
        rt.pointer_down(rect[0] + 8.0, rect[1] + 4.0);
        rt.pointer_up(rect[0] + 8.0, rect[1] + 4.0);
        rt.frame(0.0);
    }

    /// Every label the panel presented, for wording assertions.
    fn presented_labels(rt: &rune::Runtime<State>) -> Vec<String> {
        rt.elements().into_iter().filter_map(|e| e.label).collect()
    }

    /// One element's accessibility label or shown text, by key.
    fn element_text(rt: &rune::Runtime<State>, key: &str) -> Option<String> {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(key))
            .and_then(|e| e.text.or(e.label))
    }

    #[test]
    fn the_activity_groups_and_policy_radios_present_at_every_width() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            brief(&mut rt);
            assert!(rt.presented("brief-activity").is_some(), "{width}");
            assert!(
                rt.presented("brief-activity-needs-you").is_some(),
                "{width}"
            );
            assert!(
                rt.presented("brief-activity-in-progress").is_some(),
                "{width}"
            );
            // Four busy workers draw three rows and one Show more button.
            assert!(
                rt.presented("brief-activity-in-progress-more").is_some(),
                "{width}"
            );
            let (policy, _) = rt.presented("captain-report-policy").unwrap();
            assert!(
                policy[2] >= 180.0,
                "{width}: the policy labels need room, {policy:?}"
            );
            for key in [
                "captain-report-policy-react",
                "captain-report-policy-context",
                "captain-report-policy-ignore",
            ] {
                assert!(rt.presented(key).is_some(), "{width}: {key}");
            }
            // No status line joins facts with a middle dot.
            for text in presented_labels(&rt) {
                assert!(!text.contains(" · "), "{width}: {text}");
            }
            rt.state.show_activity = false;
        }
    }

    #[test]
    fn show_more_expands_and_collapses_in_place() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            brief(&mut rt);
            press_in_brief(&mut rt, "brief-activity-in-progress-more");
            rt.frame(1.0);
            // The group now draws all four workers and the button reads fewer.
            let four = rt
                .elements()
                .into_iter()
                .filter(|e| {
                    e.key
                        .as_deref()
                        .is_some_and(|key| key.starts_with("brief-activity-in-progress-crew-"))
                })
                .count();
            assert_eq!(four, 4, "{width}");
            assert_eq!(
                element_text(&rt, "brief-activity-in-progress-more").as_deref(),
                Some("Show fewer"),
                "{width}"
            );
            press_in_brief(&mut rt, "brief-activity-in-progress-more");
            rt.frame(1.0);
            let three = rt
                .elements()
                .into_iter()
                .filter(|e| {
                    e.key
                        .as_deref()
                        .is_some_and(|key| key.starts_with("brief-activity-in-progress-crew-"))
                })
                .count();
            assert_eq!(three, 3, "{width}");
            rt.state.show_activity = false;
        }
    }

    #[test]
    fn a_needs_you_row_opens_its_source_thread() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        let asking = rt
            .state
            .threads
            .iter()
            .find(|t| t.title == "Deploy worker")
            .unwrap()
            .id;
        let asking_captain = rt.state.current().captain.unwrap_or(0);
        let entry = rt
            .state
            .captain_activity_digest(asking_captain)
            .unwrap()
            .needs_you
            .into_iter()
            .find(|entry| entry.thread_id == asking)
            .expect("the request is a needs-you entry");
        press_in_brief(&mut rt, &format!("brief-activity-needs-you-{}", entry.id));
        assert_eq!(rt.state.current().id, asking);
    }

    #[test]
    fn a_row_whose_source_is_gone_is_not_a_button() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        // A crew report is a durable record: its row outlives the worker.
        let worker = rt
            .state
            .threads
            .iter()
            .find(|t| t.title == "Crew worker 0")
            .unwrap()
            .id;
        rt.state.demo = false;
        let report_id = rt
            .state
            .mcp_call(
                worker,
                "captain_crew_report",
                &json!({
                    "requestId":uuid::Uuid::new_v4().to_string(),
                    "outcome":"completed","summary":"Fixed the tokenizer edge case"
                }),
            )
            .unwrap()["reportId"]
            .as_str()
            .unwrap()
            .to_owned();
        rt.state.threads.retain(|t| t.id != worker);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let key = format!("brief-activity-whats-new-{report_id}");
        rt.update(|_, tx| tx.scroll_into_view(key.clone()));
        rt.frame(0.0);
        assert!(rt.presented(&key).is_some());
        let label = element_text(&rt, &key).expect("the row labels itself");
        assert!(label.ends_with(", source not on this machine"), "{label}");
    }

    /// A durable batch so `set_captain_report_policy` can write, exactly as
    /// `captain_event_policy`'s fixtures do.
    fn durable(rt: &rune::Runtime<State>) -> crate::database::Database {
        let database = crate::database::Database::open(&rt.state.data_dir).unwrap();
        database.begin().unwrap();
        database
    }

    /// The id of the busy worker named `title` in the fixture.
    fn worker(rt: &rune::Runtime<State>, title: &str) -> usize {
        rt.state
            .threads
            .iter()
            .find(|t| t.title == title)
            .unwrap_or_else(|| panic!("{title} is in the fixture"))
            .id
    }

    /// One plain crew report on `worker`, through the same seam the worker uses.
    fn report(rt: &mut rune::Runtime<State>, worker: usize, question: Option<Value>) -> String {
        let mut params = json!({
            "requestId":uuid::Uuid::new_v4().to_string(),
            "outcome":"blocked","summary":"waiting on a call"
        });
        if let Some(question) = question {
            params["question"] = question;
        }
        rt.state
            .mcp_call(worker, "captain_crew_report", &params)
            .unwrap()["reportId"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn delivery_words_and_tags_follow_the_captured_policy() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        let _database = durable(&rt);
        let captain = rt.state.current().captain.unwrap();
        let name = rt.state.captains[captain].name.clone();
        rt.state.demo = false;
        // React first, then Context, then Ignore: the newest lands first in
        // What's new, so the Context row proves its tag is the snapshot, not
        // the Captain's current policy.
        let w0 = worker(&rt, "Crew worker 0");
        let w1 = worker(&rt, "Crew worker 1");
        let w2 = worker(&rt, "Crew worker 2");
        let react = report(&mut rt, w0, None);
        rt.state
            .set_captain_report_policy(captain, ReportPolicy::Context, ReportPolicy::React)
            .unwrap();
        let context = report(&mut rt, w1, None);
        rt.state
            .set_captain_report_policy(captain, ReportPolicy::Ignore, ReportPolicy::Context)
            .unwrap();
        let ignore = report(&mut rt, w2, None);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let row = |id: &str| format!("brief-activity-whats-new-{id}");
        rt.update(|_, tx| tx.scroll_into_view(row(&ignore)));
        rt.frame(0.0);
        for (id, detail) in [
            (&ignore, format!("Not sent to {name}")),
            (&context, format!("Waiting to reach {name}")),
        ] {
            let label = element_text(&rt, &row(id)).expect("the row labels itself");
            assert!(label.contains(&detail), "{id}: {label}");
        }
        // The Context row keeps its snapshot tag while the Captain runs Ignore.
        for id in [&ignore, &context] {
            assert!(rt.presented(&format!("{}-tag", row(id))).is_some());
        }
        // The oldest row sits behind Show more until the group expands.
        assert!(rt.presented(&row(&react)).is_none());
        press_in_brief(&mut rt, "brief-activity-whats-new-more");
        rt.frame(1.0);
        let label = element_text(&rt, &row(&react)).expect("the row labels itself");
        assert!(
            label.contains(&format!("Waiting to reach {name}")),
            "{label}"
        );
        assert!(rt.presented(&format!("{}-tag", row(&react))).is_none());
    }

    #[test]
    fn the_truncation_line_counts_what_the_digest_bound_drops() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        rt.state.demo = false;
        let worker = worker(&rt, "Crew worker 0");
        for _ in 0..40 {
            report(&mut rt, worker, None);
        }
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        press_in_brief(&mut rt, "brief-activity-whats-new-more");
        rt.frame(1.0);
        let shown = rt
            .elements()
            .into_iter()
            .filter(|e| {
                e.key
                    .as_deref()
                    .is_some_and(|key| key.starts_with("brief-activity-whats-new-crew-"))
            })
            .count();
        assert_eq!(shown, 32, "the digest returns at most 32 reports");
        let line = element_text(&rt, "brief-activity-whats-new-truncated")
            .expect("the truncated line draws once expanded");
        assert!(line.starts_with("8 older not listed"), "{line}");
        // The sub-heading's count is the group's total, not the shown rows.
        assert!(
            rt.elements()
                .iter()
                .any(|e| e.text.as_deref() == Some("40"))
        );
    }

    #[test]
    fn a_crew_question_row_opens_its_question_card() {
        let mut rt = runtime(620.0);
        brief(&mut rt);
        rt.state.demo = false;
        let home = rt.state.current().id;
        let w0 = worker(&rt, "Crew worker 0");
        let rid = report(
            &mut rt,
            w0,
            Some(json!({"text":"Which branch?","recipient":"user"})),
        );
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        press_in_brief(&mut rt, &format!("brief-activity-needs-you-{rid}"));
        rt.frame(1.0);
        // The sheet closes, the primary home is selected and the card's answer
        // field takes focus — an answer typed on the worker is not a record.
        assert!(!rt.state.show_activity);
        assert_eq!(rt.state.current().id, home);
        let focused = rt.elements().into_iter().find(|e| e.focused);
        assert_eq!(
            focused.and_then(|e| e.key).as_deref(),
            Some(format!("needs-you-report-{rid}-text").as_str())
        );
    }

    #[test]
    fn a_saved_answer_reads_as_saved_not_accepted() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        rt.state.demo = false;
        let w0 = worker(&rt, "Crew worker 0");
        let rid = report(
            &mut rt,
            w0,
            Some(json!({"text":"Which branch?","recipient":"user"})),
        );
        rt.state
            .crew_question_draft_edit("local", &rid, 0)
            .replace_text("main");
        rt.state.submit_crew_question_answer("local", &rid).unwrap();
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let key = format!("brief-activity-needs-you-{rid}");
        rt.update(|_, tx| tx.scroll_into_view(key.clone()));
        rt.frame(0.0);
        let label = element_text(&rt, &key).expect("the row stays listed");
        assert!(label.contains("Answer saved"), "{label}");
        assert!(label.contains("Not yet accepted"), "{label}");
        // The question card itself leaves once the answer record exists.
        assert!(rt.presented(&format!("needs-you-report-{rid}")).is_none());
    }

    #[test]
    fn a_policy_send_shows_pending_until_its_exact_receipt() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        let (client, mut commands) =
            crate::client::Client::test_connected(rt.state.data_dir.clone());
        *client.info.lock().unwrap() = json!({"capabilities":[
            crate::protocol::CAPTAIN_STATE,
            crate::protocol::CAPTAIN_EVENTS,
        ]});
        rt.state.client = Some(client);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        press_in_brief(&mut rt, "captain-report-policy-context");
        let command = commands.try_recv().expect("the CAS command was sent");
        assert_eq!(command.method, "captain.reportPolicy");
        assert_eq!(command.params["expectedPolicy"], "react");
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(rt.presented("captain-report-policy-pending").is_some());
        // The exact receipt settles it; a rejection lands on the error row.
        assert!(rt.state.captain_policy_receipt(
            "local",
            &command,
            &json!({"result":{"status":"accepted","result":{"saved":true}}})
        ));
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(rt.presented("captain-report-policy-pending").is_none());
        press_in_brief(&mut rt, "captain-report-policy-ignore");
        let command = commands.try_recv().expect("the CAS command was sent");
        assert!(rt.state.captain_policy_receipt(
            "local",
            &command,
            &json!({"result":{"status":"rejected",
                "result":{"error":"Report policy changed since 'ignore'."}}})
        ));
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(rt.presented("captain-report-policy-error").is_some());
    }

    #[test]
    fn the_brief_shows_recovery_waits_and_sends_explicit_opt_in() {
        for width in [620.0, 960.0, 1280.0, 1920.0] {
            let mut rt = runtime(width);
            brief(&mut rt);
            let captain = rt.state.current().captain.unwrap();
            let db = crate::database::Database::open(&rt.state.data_dir).unwrap();
            db.begin().unwrap();
            rt.state
                .execute_command(&crate::protocol::Command {
                    command_id: uuid::Uuid::new_v4(),
                    method: "captain.inbox.await".into(),
                    params: json!({"captain":captain,"correlationId":"ci","reason":"Waiting for CI."}),
                })
                .unwrap();
            let (client, mut commands) =
                crate::client::Client::test_connected(rt.state.data_dir.clone());
            *client.info.lock().unwrap() = json!({"capabilities":[
                crate::protocol::CAPTAIN_STATE,
                crate::protocol::CAPTAIN_EVENTS,
                crate::protocol::CAPTAIN_INBOX,
            ]});
            rt.state.client = Some(client);
            rt.invalidate();
            rt.frame(0.0);
            rt.frame(1.0);
            press_in_brief(&mut rt, "captain-recovery-change");
            let command = commands.try_recv().expect("recovery command");
            assert_eq!(command.method, "captain.recoveryMode");
            assert_eq!(command.params["captain"], captain);
            assert_eq!(command.params["mode"], "unattended");
            assert_eq!(command.params["expectedMode"], "attended");
            assert_eq!(
                rt.state.captain_recovery_mode(captain),
                RecoveryMode::Attended
            );
            assert!(
                rt.state
                    .captain_recovery_unavailable("local", captain)
                    .is_some()
            );
            rt.update(|_, tx| tx.scroll_into_view("captain-recovery-wait-0"));
            rt.frame(0.0);
            assert!(rt.presented("captain-recovery-wait-0").is_some(), "{width}");
            assert!(rt.state.captain_recovery_receipt(
                "local",
                &command,
                &json!({"result":{"status":"rejected","result":{"error":"Recovery mode changed."}}}),
            ));
            assert_eq!(
                rt.state.captain_recovery_errors.values().next().unwrap(),
                "Recovery mode changed."
            );
        }
    }

    #[test]
    fn an_unreachable_machine_gives_its_reason_without_sending() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        let (mut client, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
        *client.info.lock().unwrap() = json!({"capabilities":[
            crate::protocol::CAPTAIN_STATE,
            crate::protocol::CAPTAIN_EVENTS,
        ]});
        // An unreachable link: a client that never connected.
        client.linked.insert(
            "studio".into(),
            crate::client::Client::new(rt.state.data_dir.clone()),
        );
        rt.state.client = Some(client);
        let projection = rt.state.projection();
        rt.state
            .machines
            .views
            .insert("studio".into(), projection.into());
        rt.state.machines.selected = "studio".into();
        // The link reports disconnected only off the demo path.
        rt.state.demo = false;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        let reason =
            element_text(&rt, "captain-report-policy-reason").expect("the reason note draws");
        assert!(reason.ends_with("is unreachable."), "{reason}");
        assert!(!source_known(
            &rt.state,
            "studio",
            worker(&rt, "Crew worker 0")
        ));
        for element in rt.elements() {
            if element
                .key
                .as_deref()
                .is_some_and(|key| key.starts_with("brief-activity-in-progress-crew-"))
            {
                assert_ne!(element.role, "button");
                assert!(!element.focusable);
            }
            if element.role == "radio" {
                assert!(
                    !element.focusable,
                    "{:?} stays out of the Tab order",
                    element.key
                );
            }
        }
    }

    #[test]
    fn the_saved_preview_bounds_itself_when_activity_sits_above_it() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        let captain = rt.state.current().captain.unwrap();
        rt.state.demo = false;
        let text = (0..20)
            .map(|line| format!("Digest line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        rt.state
            .captain_document_write(captain, "DIGEST.md", text, 0)
            .expect("the local captain takes a document write");
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        rt.update(|_, tx| tx.scroll_into_view("brief-digest-preview"));
        rt.frame(0.0);
        let (rect, _) = rt
            .presented("brief-digest-preview")
            .expect("the demo seeds a saved digest");
        // Six lines plus the row's padding when activity is above it.
        assert!(rect[3] <= 104.0, "{rect:?}");
    }

    #[test]
    fn a_press_without_the_durable_server_shows_the_reason() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        // The headless State has no client, so the send refuses; the reason
        // lands on the block's error row, never a toast.
        rt.state.client = None;
        press_in_brief(&mut rt, "captain-report-policy-context");
        assert!(rt.presented("captain-report-policy-error").is_some());
        assert_eq!(rt.state.captain_report_policy(0), ReportPolicy::React);
    }

    #[test]
    fn a_peer_without_the_capability_shows_only_the_note() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        // A linked studio that negotiated nothing past captain-state-v1.
        let (mut client, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
        let (remote, _) = crate::client::Client::test_connected(rt.state.data_dir.clone());
        *remote.info.lock().unwrap() = json!({"capabilities":[crate::protocol::CAPTAIN_STATE]});
        *client.info.lock().unwrap() = json!({"capabilities":[crate::protocol::CAPTAIN_STATE]});
        client.linked.insert("studio".into(), remote);
        rt.state.client = Some(client);
        let projection = rt.state.projection();
        rt.state
            .machines
            .views
            .insert("studio".into(), projection.into());
        rt.state.machines.selected = "studio".into();
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(rt.presented("captain-report-policy").is_none());
        let reason =
            element_text(&rt, "captain-report-policy-reason").expect("the reason note draws");
        assert!(reason.starts_with("Update Shina on "), "{reason}");
    }

    #[test]
    fn arrows_move_focus_without_sending_and_space_selects() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        rt.update(|_, tx| tx.focus("captain-report-policy-react", true));
        rt.frame(0.0);
        // Right reaches Context but sends nothing: each selection is a remote
        // command, so arrows cannot queue CAS writes on the way past.
        rt.key(rune::KeyEvent {
            key: rune::Key::Right,
            mods: Default::default(),
        });
        rt.frame(0.0);
        assert!(rt.state.captain_policy_pending.is_empty());
        assert_eq!(rt.state.captain_report_policy(0), ReportPolicy::React);
        let focused = rt.elements().into_iter().find(|e| e.focused);
        assert_eq!(
            focused.and_then(|e| e.key).as_deref(),
            Some("captain-report-policy-context")
        );
        // Space presses the focused segment; the headless State refuses the
        // send and the refusal lands on the error row.
        rt.key(rune::KeyEvent {
            key: rune::Key::Text(" ".into()),
            mods: Default::default(),
        });
        rt.frame(0.0);
        rt.frame(1.0);
        assert!(
            rt.presented("captain-report-policy-error").is_some()
                || !rt.state.captain_policy_errors.is_empty()
        );
    }

    #[test]
    fn the_manual_digest_summary_is_unchanged_by_the_activity_rows() {
        let mut rt = runtime(1280.0);
        brief(&mut rt);
        // The activity view never writes DIGEST.md; the manual preview keeps
        // its own revision and rows.
        let meta = rt.state.captain_document_meta(0);
        let digest_meta = meta.iter().find(|m| m.name == "DIGEST.md").unwrap();
        assert_eq!(
            digest_meta.revision,
            rt.state.captain_activity_digest(0).unwrap().manual_revision
        );
    }
}
