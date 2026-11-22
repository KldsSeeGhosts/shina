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
const ROW_TEXT: TextStyle = TextStyle::new(13.0, 500.0).lh(19.0).tracking(-0.08);
use super::META_COMPACT as META;

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
    close_brief(s);
    // The card opens on the primary home's chat; a covering Inspector would hide it.
    if s.brief.record.is_some() || s.inspector_covers() {
        let key = s.panel_key();
        s.right_panels.close(&key);
    }
    let kind = entry["kind"].as_str().unwrap_or("");
    if kind == "crew_question" {
        let rid = entry["reportId"].as_str().unwrap_or("").to_string();
        let question_index = (!rid.is_empty()
            && s.captain_phase(captain) == crate::model::CaptainPhase::Active
            && s.captain_home(captain).is_some())
        .then(|| {
            s.open_crew_questions(captain, QuestionRecipient::User)
                .iter()
                .position(|view| view.report_id == rid)
        })
        .flatten();
        if let Some(q_idx) = question_index {
            let home = s.captain_home(captain).expect("home checked above");
            if s.current().id != home
                && let Some(index) = s.threads.iter().position(|t| t.id == home)
            {
                s.show_thread(index);
            }
            let live_count = s
                .attention()
                .into_iter()
                .filter(|item| super::crew_ui::needs_you_item(s, home, item))
                .count();
            s.needs_you_page = super::crew_ui::thread_request_pages(s) + live_count + q_idx;
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
/// The Overview's "What's new" (spec §3.4): the digest's "What's new" group on its
/// own. "Needs you" lives in the request bar and "In progress" in the Crew tab.
pub(super) fn activity_whats_new(s: &State, p: Palette, captain: usize) -> El {
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
    let whats_new = digest["whatsNew"].as_array().cloned().unwrap_or_default();
    if whats_new.is_empty() {
        return quiet_note(format!("Nothing new from {captain_name}'s crew yet."), p)
            .key("brief-activity-empty");
    }
    col()
        .key("brief-activity")
        .role("group")
        .label("What's new")
        .gap(4.0)
        .pb(4.0)
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
        let key = rt.state.panel_key();
        rt.state
            .right_panels
            .open(&key, crate::right_panel::SingletonKind::Overview);
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

    /// Open the Inspector on `kind`, as pressing its strip segment does.
    fn open_tab(rt: &mut rune::Runtime<State>, kind: crate::right_panel::SingletonKind) {
        let key = rt.state.panel_key();
        rt.state.right_panels.open(&key, kind);
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(0.5);
    }

    /// Every label the panel presented, for wording assertions.
    fn presented_labels(rt: &rune::Runtime<State>) -> Vec<String> {
        rt.elements().into_iter().filter_map(|e| e.label).collect()
    }

    #[test]
    fn the_activity_groups_and_policy_radios_present_at_every_width() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            brief(&mut rt);
            // "What's new" is the only activity group left on the Overview: the
            // request lives in the needs-you bar and the busy workers in the Crew
            // tab (spec §3.4). With no reports yet it reads as the quiet note.
            assert!(
                rt.presented("brief-activity-empty").is_some()
                    || rt.presented("brief-activity-whats-new").is_some(),
                "{width}"
            );
            assert!(
                rt.presented("brief-activity-needs-you").is_none(),
                "{width}"
            );
            assert!(
                rt.presented("brief-activity-in-progress").is_none(),
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
            let key = rt.state.panel_key();
            rt.state.right_panels.close(&key);
        }
    }

    #[test]
    fn show_more_expands_and_collapses_in_place() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            brief(&mut rt);
            // The busy workers moved to the Crew tab's Working group (spec §3.5),
            // which lists them all without a Show more fold.
            open_tab(&mut rt, crate::right_panel::SingletonKind::Crew);
            let worker_ids: Vec<usize> = rt
                .state
                .threads
                .iter()
                .filter(|t| t.title.starts_with("Crew worker"))
                .map(|t| t.id)
                .collect();
            let working = rt
                .elements()
                .into_iter()
                .filter(|e| {
                    worker_ids
                        .iter()
                        .any(|id| e.key.as_deref() == Some(format!("crew-chip-{id}").as_str()))
                })
                .count();
            assert_eq!(working, 4, "{width}: the four busy crew workers");
            assert!(rt.presented("crew-group-working").is_some(), "{width}");
            let key = rt.state.panel_key();
            rt.state.right_panels.close(&key);
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
        // The request bar above the composer carries the open-source link now
        // (spec §3.6): the pressable title replaces the row's own button. The
        // demo seeds another request, so page until this one is the shown card.
        while rt
            .presented(&format!("needs-you-source-{asking}"))
            .is_none()
        {
            let Some((next, _)) = rt.presented("needs-you-next") else {
                break;
            };
            rt.pointer_down(next[0] + 8.0, next[1] + 4.0);
            rt.pointer_up(next[0] + 8.0, next[1] + 4.0);
            rt.frame(0.3);
        }
        press_in_brief(&mut rt, &format!("needs-you-source-{asking}"));
        assert_eq!(rt.state.current().id, asking);
    }

    /// Persist just the captain roster so a durable open can validate the
    /// memory.json the demo wrote. A full checkpoint would save demo threads
    /// that do not survive index validation.
    fn save_captain_index(rt: &rune::Runtime<State>) {
        let index = json!({
            "version":4,
            "captains":rt.state.captains.iter().map(crate::captains::Captain::to_value).collect::<Vec<_>>(),
            "projects":[],
            "threads":[],
            "tasks":[],
            "nextId":1,
            "nextProjectId":1,
        });
        std::fs::write(rt.state.data_dir.join("index.json"), index.to_string()).unwrap();
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
    fn a_crew_question_row_opens_its_question_card() {
        let mut rt = runtime(620.0);
        brief(&mut rt);
        // At 620 pt the Inspector covers the chat lane; close it to inspect the home's
        // needs-you bar, then advance the unified pager past the demo's live approval.
        let key = rt.state.panel_key();
        rt.state.right_panels.close(&key);
        rt.state.demo = false;
        let w0 = worker(&rt, "Crew worker 0");
        let rid = report(
            &mut rt,
            w0,
            Some(json!({"text":"Which branch?","recipient":"user"})),
        );
        // `brief()` seeds two live approvals (pages 0 and 1); the report question is page 2.
        rt.state.needs_you_page = 2;
        rt.invalidate();
        rt.frame(0.0);
        rt.frame(1.0);
        // The question card now lives in the needs-you bar on the captain home
        // itself (spec §3.6); there is no Inspector row to press. The answer
        // field is focusable right where the request sits.
        assert!(rt.presented(&format!("needs-you-report-{rid}")).is_some());
        let field = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some(format!("needs-you-report-{rid}-text").as_str()));
        assert!(
            field.is_some_and(|e| e.focusable),
            "the answer field takes focus"
        );
    }

    #[test]
    fn the_brief_shows_recovery_waits_and_sends_explicit_opt_in() {
        for width in [1280.0] {
            let mut rt = runtime(width);
            brief(&mut rt);
            save_captain_index(&rt);
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
            open_tab(&mut rt, crate::right_panel::SingletonKind::Overview);
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
}
