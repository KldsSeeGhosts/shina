//! Captain and crew surfaces: task results in a captain's transcript, the crew roster,
//! the needs-you stack and the worker's parent bar above the composer, and the attention row and
//! recovery notice at the top of the sidebar.
//!
//! One design language for all of them: hover is ink at 6 %, pressed or active at 8 %, hairline
//! borders in `line`, rows at a 4 px radius, cards at 6 px, and motion of 0.18 s on
//! `cubic-bezier(.23, 1, .32, 1)`. The skin follows KSG Code's neutral theme, the app's shared
//! `Palette` and tokens in `ui.rs`.
use super::*;
use crate::model::placement::{PlacementMode, PlacementPhase, ThreadPlacement};
use crate::model::{AttentionItem, AttentionKind, PeerOrigin, TaskState, TaskView};
use rune::Transition;

/// Row corners, shared by cards, chips and popover items.
const ROW_RADIUS: f32 = 4.0;
/// Where a card's second and later lines start: past the status mark's 14 px slot and its gap.
const CARD_INDENT: f32 = 22.0;
pub(super) fn motion() -> rune::Curve {
    rune::Curve::tween(0.18, rune::Easing::cubic(0.23, 1.0, 0.32, 1.0))
}
/// Ink washes. Rune blends in linear light, where a light ink on a dark ground reads stronger,
/// so the dark theme takes a little over half the alpha for the same visible step.
pub(super) fn hover_wash(s: &State, p: Palette) -> Color {
    p.ink.alpha(if s.dark { 0.035 } else { 0.06 })
}
pub(super) fn active_wash(s: &State, p: Palette) -> Color {
    p.ink.alpha(if s.dark { 0.05 } else { 0.08 })
}
/// A quiet, outlined action: Decline, Dismiss, Open.
pub(super) fn ghost_button(
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
        .child(label(title, TextStyle::new(13.0, 500.0), p.ink))
}
/// The one filled action in a group: Approve, Resume, Open to answer.
pub(super) fn solid_button(
    key: String,
    title: &str,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    framed(
        row()
            .key(key)
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(title)
            .focusable(true)
            .center_y()
            .h(28.0)
            .px(12.0)
            .radius(6.0)
            .fill(p.coral)
            // Rune cannot move a button on press, so a pressed key darkens instead.
            .press(p.coral.alpha(0.9))
            .on_press(action)
            .child(label(title, TextStyle::new(13.0, 600.0), p.on_coral)),
        p,
    )
}

pub(super) fn open_thread(s: &mut State, id: usize) {
    if let Some(remote) = s.remote_task(id).cloned()
        && let Some(thread) = remote.thread_id
        && s.client
            .as_ref()
            .is_some_and(|c| c.linked.contains_key(&remote.server_id))
        && s.machines.views.get(&remote.server_id).is_some_and(|view| {
            view["index"]["threads"]
                .as_array()
                .is_some_and(|threads| threads.iter().any(|t| t["id"] == thread))
        })
    {
        s.open_machine_thread(&remote.server_id, thread);
        return;
    }
    if let Some(index) = s.threads.iter().position(|t| t.id == id) {
        s.show_worker_ghost(index);
    }
}

/// A task's place in the shared status vocabulary, and the word for it. A child that is
/// asking for something needs the user whatever its task record says.
fn task_status(s: &State, task: &TaskView) -> (Status, &'static str) {
    // No provider starts before its checkout is ready, so a pending or failed placement is
    // the whole story until then.
    let placement = || match worker_placement(s, task.child_id).map(|placement| placement.phase) {
        Some(PlacementPhase::Failed)
            if !matches!(task.state, TaskState::Completed | TaskState::Cancelled) =>
        {
            Some((Status::Failed, "Setup failed"))
        }
        Some(PlacementPhase::Pending) if !task.state.finished() => {
            Some((Status::Ready, "Preparing"))
        }
        _ => None,
    };
    if let Some(remote) = &task.remote {
        if remote.state == crate::model::RemoteState::UpdateRequired {
            return (Status::Waiting, "Update required");
        }
        if remote.stop_owed && remote.state != crate::model::RemoteState::TargetOffline {
            return (Status::Waiting, "Stop pending");
        }
        if remote.needs_you && !task.state.finished() {
            return (Status::Waiting, "Needs you");
        }
        if let Some(status) = placement() {
            return status;
        }
        if task.state != TaskState::Queued {
            return (
                match remote.state {
                    crate::model::RemoteState::Failed | crate::model::RemoteState::Revoked => {
                        Status::Failed
                    }
                    crate::model::RemoteState::Done
                    | crate::model::RemoteState::ResultPendingDelivery => Status::Completed,
                    crate::model::RemoteState::TargetOffline => Status::Paused,
                    crate::model::RemoteState::UpdateRequired => Status::Waiting,
                    _ => Status::Running,
                },
                remote.state.word(),
            );
        }
    }
    if s.threads
        .iter()
        .any(|t| t.id == task.child_id && t.request.is_some())
    {
        return (Status::Waiting, "Needs you");
    }
    if let Some(status) = placement() {
        return status;
    }
    match task.state {
        TaskState::Queued => (Status::Ready, "Queued"),
        TaskState::Starting => (Status::Running, "Starting"),
        TaskState::Running => (Status::Running, "Working"),
        TaskState::Completed => (Status::Completed, "Done"),
        TaskState::Failed => (Status::Failed, "Failed"),
        TaskState::Cancelled => (Status::Ready, "Dismissed"),
        TaskState::Interrupted => (Status::Waiting, "Interrupted"),
    }
}

/// Where a worker actually runs: the placement mirrored from the target for a remote task,
/// else the worker thread's own record.
pub(super) fn worker_placement(s: &State, thread: usize) -> Option<ThreadPlacement> {
    s.remote_task(thread)
        .and_then(|remote| remote.placement.clone())
        .and_then(|value| serde_json::from_value(value).ok())
        .or_else(|| s.threads.iter().find(|t| t.id == thread)?.placement.clone())
}

/// The machine a placement's checkout lives on, by name.
pub(super) fn placement_machine(s: &State, thread: usize, placement: &ThreadPlacement) -> String {
    match s.remote_task(thread) {
        Some(remote) => remote.label.clone(),
        None if placement.executor.is_empty() || placement.executor == "local" => {
            s.machine_label(&s.machines.selected)
        }
        None => s.machine_label(&placement.executor),
    }
}

/// The checkout's kind in the status vocabulary. A shared checkout of a real project, the main
/// one or a parent's existing worktree, is read only; one without a project is the thread's
/// private scratch folder. `ThreadPlacement::read_only` decides, so the word matches the
/// access the worker actually gets.
fn placement_word(placement: &ThreadPlacement) -> &'static str {
    match placement.mode {
        PlacementMode::Shared if placement.read_only() => "Read only",
        PlacementMode::Shared => "Scratch folder",
        PlacementMode::Isolated => "Worktree",
        PlacementMode::Existing if placement.primary_checkout => "Main checkout",
        PlacementMode::Existing => "Existing worktree",
    }
}

/// Every resolved field of a placement, one per line, for the tooltip and the screen reader.
pub(super) fn placement_details(placement: &ThreadPlacement, machine: &str) -> String {
    let mut lines = vec![format!("{} on {machine}", placement_word(placement))];
    if let Some(checkout) = &placement.checkout {
        lines.push(format!("Checkout: {checkout}"));
    }
    if let Some(branch) = &placement.branch {
        lines.push(format!("Branch: {branch}"));
    }
    let commit = placement
        .base_commit
        .as_deref()
        .map(|commit| commit.get(..12).unwrap_or(commit));
    match (&placement.base_branch, commit) {
        (Some(base), Some(commit)) => lines.push(format!("Base: {base} at {commit}")),
        (Some(base), None) => lines.push(format!("Base: {base}")),
        (None, Some(commit)) => lines.push(format!("Base: {commit}")),
        (None, None) => {}
    }
    if let Some(upstream) = placement.upstream_thread_id {
        lines.push(format!("Builds on thread #{upstream}"));
    }
    if let Some(repository) = &placement.repository
        && placement.checkout.as_ref() != Some(repository)
    {
        lines.push(format!("Repository: {repository}"));
    }
    lines.join("\n")
}

/// What a failed placement leaves the user to do. Nothing started, and the reservation stays,
/// so a retry reuses the same checkout.
fn placement_recovery(s: &State, thread: usize, sender: &str) -> String {
    let setup_failed = s.threads.iter().any(|t| {
        t.id == thread
            && t.worktree_setup
                .as_ref()
                .is_some_and(|setup| setup.phase == crate::model::WorktreeSetupPhase::Failed)
    });
    if setup_failed {
        if s.current().id == thread {
            "Setup failed. Retry or skip this worker's setup script below.".into()
        } else {
            "Nothing started. Open the worker to retry or skip its setup script.".into()
        }
    } else {
        format!("Nothing started. New work from {sender} retries in the same checkout.")
    }
}

/// The mark a worker's sidebar row shares with its crew chip when its row has no other mark:
/// brass for interrupted (it needs a look before resuming), muted for queued. Finished workers
/// have none.
pub(super) fn worker_task_mark(s: &State, thread: &crate::model::Thread) -> Option<Status> {
    let task = s
        .tasks_for(thread.parent?)
        .into_iter()
        .find(|task| task.child_id == thread.id)?;
    match task.state {
        TaskState::Interrupted | TaskState::Queued => Some(task_status(s, &task).0),
        _ => None,
    }
}

/// The elapsed time, or the queue position while a task waits for a slot.
fn task_clock(task: &TaskView) -> String {
    if let Some(position) = task.queue_position {
        return format!("#{position} queued");
    }
    match task.started_at {
        Some(start) => {
            let end = task.finished_at.unwrap_or_else(lifecycle::now);
            timer_label(end.saturating_sub(start) / 1000)
        }
        None => String::new(),
    }
}

/// A status mark centred in a fixed 14 px slot, so a check and a dot share one left edge.
pub(super) fn mark_slot(status: Status, p: Palette) -> El {
    el().size(14.0, 14.0)
        .center()
        .child(status_mark(status, p, 7.0))
}

pub(super) fn is_delegation(tool: &Tool) -> bool {
    tool.name == "delegate_task" || tool.name.ends_with("__delegate_task")
}

/// The task a `delegate_task` call started. Its result names the child thread; while that is
/// unreadable, the n-th delegation in the transcript is the parent's n-th task.
fn delegated_task(s: &State, t: &Thread, index: usize) -> Option<TaskView> {
    let tasks = s.tasks_for(t.id);
    let tool = &t.tools[index];
    let named = serde_json::from_str::<serde_json::Value>(&tool.output)
        .ok()
        .and_then(|value| value["thread_id"].as_u64())
        .or_else(|| tool.structured["thread_id"].as_u64());
    if let Some(id) = named {
        return tasks.into_iter().find(|task| task.child_id == id as usize);
    }
    let ordinal = t.tools[..index]
        .iter()
        .filter(|tool| is_delegation(tool) && !tool.failed)
        .count();
    tasks.into_iter().nth(ordinal)
}

/// A tool batch in the transcript. In a captain's home each delegation becomes a subagent
/// row; every other call keeps the shared tool rows. KSG hides the delegation call and
/// shows the subagent's own row in its place (`withoutSubagentDelegationRows`).
pub(super) fn tool_part(s: &State, p: Palette, indices: &[usize]) -> El {
    let t = s.current();
    if !s.is_captain_home(t) {
        return inline_tools(s, p, indices);
    }
    let mut cards = vec![];
    let mut rest = vec![];
    for &index in indices {
        match is_delegation(&t.tools[index])
            .then(|| delegated_task(s, t, index))
            .flatten()
        {
            Some(task) => cards.push(task),
            None => rest.push(index),
        }
    }
    if cards.is_empty() {
        return inline_tools(s, p, indices);
    }
    // KSG's `V2SubagentGroup`: one row each on the tool rows' rhythm, in start order.
    let mut part = col().gap(2.0);
    if !rest.is_empty() {
        part = part.child(inline_tools(s, p, &rest));
    }
    for task in &cards {
        part = part.child(task_card(s, p, task));
    }
    part
}

/// `text-label`: a subagent row's title.
const SUBAGENT_TITLE: TextStyle = TextStyle::new(14.0, 500.0).lh(20.0);
/// `text-body`: what the subagent is doing, or what it returned.
const SUBAGENT_DETAIL: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0);
/// `text-caption`: the model, the state and the time.
const SUBAGENT_CAPTION: TextStyle = TextStyle::new(11.0, 500.0).lh(16.0);

/// What a subagent's line starts with, besides its title.
struct SubagentLine<'a> {
    key: String,
    /// The accessible name.
    name: String,
    child: usize,
    model_label: &'a str,
    remote: Option<El>,
    detail: &'a str,
    failed: bool,
}

/// The start of a subagent's line, ported from KSG Code's `SubagentTimelineLink`
/// (`chat/V2LifecycleRow.tsx`): the tool rows' metrics (24 px tall, flush left, 8 px gaps,
/// 6 px end inset), the bare provider logo in the 24 px icon column, the title, then what
/// the subagent is doing or returned, then its model. Callers append the time and the
/// state. The whole line opens the worker.
fn subagent_line(s: &State, p: Palette, title: El, line: SubagentLine) -> El {
    let child = line.child;
    let mut row_ = row()
        .key(line.key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(line.name)
        .focusable(true)
        .min_h(24.0)
        .w_full()
        .min_w(0.0)
        .pr(6.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .on_press(move |s: &mut State| open_thread(s, child))
        .child(el().size(24.0, 24.0).mr(-3.0).center().child(provider_icon(
            s,
            p,
            model_vendor(line.model_label, line.model_label),
            16.0,
        )))
        .child(title);
    if let Some(remote) = line.remote {
        row_ = row_.child(remote);
    }
    row_ = row_.child(
        label(
            line.detail,
            SUBAGENT_DETAIL,
            if line.failed { p.danger } else { p.muted },
        )
        .truncate()
        .grow(1.0)
        .min_w(0.0),
    );
    let (model, _) = split_model_label(line.model_label);
    if !model.is_empty() {
        row_ = row_.child(label(model, SUBAGENT_CAPTION, p.muted));
    }
    row_
}

/// A delegated task in the captain's transcript, as KSG Code's `SubagentTimelineLink` draws a
/// subagent: logo, title, what it is doing, model, time, state. The remote machine tag ports
/// KSG's "on {environment}" label.
pub(super) fn task_card(s: &State, p: Palette, task: &TaskView) -> El {
    let (status, word) = task_status(s, task);
    let child = task.child_id;
    let placement = worker_placement(s, child);
    // A failed placement's reason is the line's detail.
    let setup_error = placement
        .as_ref()
        .filter(|p_| p_.phase == PlacementPhase::Failed)
        .and_then(|p_| p_.error.clone())
        .map(|e| truncate(&e, 200));
    let detail = setup_error.unwrap_or_else(|| {
        // The view falls back to the bare state id when the task has said nothing yet.
        if task.last_activity_line == task.state.id() {
            String::new()
        } else {
            task.last_activity_line.clone()
        }
    });
    let mut line = subagent_line(
        s,
        p,
        label(&task.title, SUBAGENT_TITLE, p.ink)
            .key(format!("task-title-{child}"))
            .truncate()
            .shrink()
            .min_w(0.0),
        SubagentLine {
            key: format!("task-card-{child}"),
            name: format!("{}. {word}. Open the worker", task.title),
            child,
            model_label: &task.model_label,
            remote: task.remote.as_ref().map(|remote| {
                machine_tag(
                    s,
                    p,
                    &remote.server_id,
                    format!("task-machine-{child}"),
                    SUBAGENT_CAPTION,
                    true,
                )
            }),
            detail: &detail,
            failed: status == Status::Failed,
        },
    );
    if let Some(placement) = &placement {
        line = line.help(placement_details(
            placement,
            &placement_machine(s, child, placement),
        ));
    }
    // A state the trailing mark cannot say on its own gets its word. A queued task's
    // clock already says so.
    if !matches!(status, Status::Running | Status::Completed) && task.queue_position.is_none() {
        line = line.child(
            label(word, SUBAGENT_CAPTION, super::status_ink(status, p))
                .key(format!("task-meta-{child}")),
        );
    }
    line.child(
        label(
            task_clock(task),
            SUBAGENT_CAPTION.features(TABULAR),
            p.faint,
        )
        .key(format!("task-clock-{child}")),
    )
    .child(mark_slot(status, p))
}

/// A task result Shina delivered to its captain, read from `Task::completion_message`.
pub(super) struct Completion<'a> {
    pub child: usize,
    pub title: &'a str,
    pub status: &'a str,
    pub reply: &'a str,
    pub error: &'a str,
}

pub(super) fn parse_completion(text: &str) -> Option<Completion<'_>> {
    let (first, rest) = text.split_once('\n')?;
    let child = first
        .strip_prefix("Shina: task #")?
        .strip_suffix(" finished")?
        .parse()
        .ok()?;
    let (_, rest) = rest.split_once('\n')?; // Task id
    let (title, rest) = rest.split_once('\n')?;
    let (status, rest) = rest.split_once('\n')?;
    let body = rest.strip_prefix("Last reply:\n")?;
    let (reply, error) = body.rsplit_once("\nError: ").unwrap_or((body, "none"));
    Some(Completion {
        child,
        title: title.strip_prefix("Title: ")?,
        status: status.strip_prefix("Status: ")?,
        reply,
        error,
    })
}

/// A worker's result in the captain's home, as KSG Code draws a subagent's notification
/// (`SubagentNotificationLink`, the event form of `SubagentTimelineLink`): the same line as
/// its task row, ending in the outcome and the time it finished. The line opens the worker,
/// where the full reply is.
pub(super) fn result_block(s: &State, p: Palette, completion: &Completion) -> El {
    let child = completion.child;
    let failed = completion.status == "failed";
    let word = match completion.status {
        "completed" => "Finished",
        "failed" => "Failed",
        "cancelled" => "Dismissed",
        "interrupted" => "Stopped",
        other => other,
    };
    let task = s
        .tasks_for(s.current().id)
        .into_iter()
        .find(|task| task.child_id == child);
    let error = completion.error.trim();
    let reply = completion.reply.trim();
    // KSG's settled detail: the result, else the error.
    let detail = if failed && error != "none" && !error.is_empty() {
        error
    } else if reply == "(no reply)" {
        ""
    } else {
        reply
    };
    let detail = truncate(detail.lines().next().unwrap_or_default(), 200);
    let mut line = subagent_line(
        s,
        p,
        label(completion.title, SUBAGENT_TITLE, p.ink)
            .truncate()
            .shrink()
            .min_w(0.0),
        SubagentLine {
            key: format!("result-open-{child}"),
            name: format!("Result from {}. {word}. Open the worker", completion.title),
            child,
            model_label: task.as_ref().map_or("", |task| task.model_label.as_str()),
            remote: None,
            detail: &detail,
            failed,
        },
    )
    .child(label(
        word,
        SUBAGENT_CAPTION,
        if failed { p.danger } else { p.muted },
    ));
    if let Some(at) = task.and_then(|task| task.finished_at)
        && let Some(time) = chrono::DateTime::from_timestamp_millis(at as i64)
    {
        line = line.child(label(
            time.with_timezone(&chrono::Local)
                .format("%-I:%M %p")
                .to_string(),
            SUBAGENT_CAPTION.features(TABULAR),
            p.faint,
        ));
    }
    // The transcript keys the element it gets back, so the line keeps its own key inside.
    col().child(line)
}

/// KSG has no crew report record. Adapt its result row geometry, reading the saved
/// record by exact delivery ID rather than interpreting a worker's prose as status.
pub(super) fn report_message(s: &State, p: Palette, text: &str) -> Option<El> {
    use crate::model::crew_reports::{CrewOutcome, QuestionRecipient};
    let header = text
        .strip_prefix(crate::model::INJECTION_PREFIX)?
        .split_once('\n')?
        .0;
    let id = header.strip_prefix("[delivery ")?.strip_suffix(']')?;
    let captain = s
        .current()
        .captain
        .filter(|_| s.is_captain_home(s.current()))?;
    let report = s
        .crew_reports()
        .iter()
        .find(|report| report.id == id && report.captain == captain)?;
    let (word, status) = match report.outcome {
        CrewOutcome::Completed => ("Reported done", Status::Completed),
        CrewOutcome::Failed => ("Reported failed", Status::Failed),
        CrewOutcome::Cancelled => ("Reported cancelled", Status::Ready),
        CrewOutcome::Progress => ("Reported progress", Status::Running),
        CrewOutcome::Blocked => ("Reported blocked", Status::Waiting),
    };
    let thread = report.thread_id;
    let mut body = col().gap(8.0).pl(CARD_INDENT).child(
        label(truncate(&report.summary, 6000), body_style(false), p.ink)
            .wrap()
            .selectable(true),
    );
    if !report.error.is_empty() {
        body = body.child(
            label(truncate(&report.error, 2000), SMALL, p.danger)
                .wrap()
                .selectable(true),
        );
    }
    for artifact in report.artifacts.iter().take(16) {
        body = body.child(
            col()
                .gap(2.0)
                .child(label(&artifact.label, SMALL, p.muted).wrap())
                .child(
                    label(
                        format!(
                            "Unverified reference on {}: {}",
                            s.machine_label(&s.machines.selected),
                            artifact.reference
                        ),
                        SMALL,
                        p.faint,
                    )
                    .wrap_anywhere()
                    .selectable(true),
                ),
        );
    }
    if let Some(question) = &report.question {
        let answer = s
            .crew_answers()
            .iter()
            .find(|answer| answer.report_id == report.id);
        let state = match answer {
            None => "Open",
            Some(answer) if !answer.delivered => "Answer saved, waiting for the source thread",
            Some(_) => "Answer accepted by the source thread",
        };
        body = body.child(
            label(
                format!(
                    "Asked {}: {}. {state}",
                    match question.recipient {
                        QuestionRecipient::User => "you",
                        QuestionRecipient::Captain => &s.captains[captain].name,
                    },
                    question.text
                ),
                SMALL,
                p.brass,
            )
            .wrap(),
        );
    }
    Some(
        col()
            .key(format!("crew-report-{}", report.id))
            .gap(6.0)
            .child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(mark_slot(status, p))
                    .child(meta_text(word, status_ink(status, p)))
                    .child(
                        label(&report.title, ACTIVITY, p.muted)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(ghost_button(
                        s,
                        format!("crew-report-{}-open", report.id),
                        "Open thread",
                        p,
                        move |s| open_thread(s, thread),
                    )),
            )
            .child(body),
    )
}

/// A brief or follow-up Shina delivered to a worker: `(is_follow_up, parent id, text)`.
pub(super) fn parse_brief(text: &str) -> Option<(bool, usize, &str)> {
    let (first, body) = text.split_once(
        "

",
    )?;
    let (follow_up, parent) = match first.strip_prefix("Shina: task from parent #") {
        Some(parent) => (false, parent),
        None => (true, first.strip_prefix("Shina: message from parent #")?),
    };
    Some((follow_up, parent.parse().ok()?, body))
}

/// The worker's brief, as the user would read a quoted request: who sent it, on a rail, in
/// the body ink. The header opens the sender.
pub(super) fn brief_block(
    s: &State,
    p: Palette,
    wide: bool,
    (follow_up, parent, body): (bool, usize, &str),
) -> El {
    let (secondary, tertiary) = quiet_inks(p);
    let sender = s.threads.iter().find(|t| t.id == parent);
    let captain = sender
        .and_then(|t| t.captain)
        .and_then(|c| s.captains.get(c));
    let name = match (captain, sender) {
        (Some(captain), _) => format!("Captain {}", captain.name),
        (None, Some(thread)) => thread.title.clone(),
        (None, None) => format!("thread #{parent}"),
    };
    let mut header = row()
        .key(format!("brief-open-{parent}"))
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!("Brief from {name}. Open it"))
        .focusable(true)
        .center_y()
        .gap(7.0)
        .h(26.0)
        .px(6.0)
        .ml(-6.0)
        .radius(6.0)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .on_press(move |s: &mut State| open_thread(s, parent));
    if let Some(index) = sender.and_then(|t| t.captain) {
        header = header.child(avatar(s, p, index, 16.0, Status::Ready));
    }
    header = header
        .child(label(
            if follow_up {
                "Follow-up from"
            } else {
                "Brief from"
            },
            ACTIVITY,
            secondary,
        ))
        .child(
            label(
                name,
                TextStyle {
                    weight: 600.0,
                    ..ACTIVITY
                },
                p.ink,
            )
            .truncate()
            .shrink()
            .min_w(0.0),
        )
        .child(icon("open", tertiary).opacity(0.8));
    // Where this worker runs, at the end of the brief's header row (spec §3.8).
    if let Some(placement) = worker_placement(s, s.current().id) {
        header = header.child(spacer()).child(
            label(
                placement_details(
                    &placement,
                    &placement_machine(s, s.current().id, &placement),
                )
                .replace('\n', " · "),
                SMALL,
                tertiary,
            )
            .key("worker-placement")
            .truncate()
            .min_w(0.0),
        );
    }
    col().gap(4.0).child(header).child(
        row()
            .child(
                el().w(1.0)
                    .h_full()
                    .ml(7.5)
                    .fill(p.line)
                    .hairline()
                    .a11y_hidden(),
            )
            .child(
                label(body.trim(), body_style(wide), p.ink)
                    .wrap_anywhere()
                    .selectable(true)
                    .pl(CARD_INDENT - 8.0)
                    .ml(6.0)
                    .grow(1.0)
                    .min_w(0.0),
            ),
    )
}

/// One member of a captain's Crew Roster: the thread, its task when one exists, and the
/// ordering rank. Needs-you rows lead, then working, then queued, then idle, each by
/// `updated`. `crew-chip-{id}` keys stay stable across all of them.
pub(super) struct RosterMember {
    pub(super) thread_id: usize,
    pub(super) title: String,
    pub(super) model: String,
    pub(super) status: Status,
    pub(super) word: String,
    pub(super) clock: String,
    pub(super) remote_server: Option<String>,
    pub(super) rank: usize,
    pub(super) updated: u64,
    pub(super) place: Option<String>,
    pub(super) queued: bool,
}

pub(super) fn roster_members(s: &State, home: usize, captain: usize) -> Vec<RosterMember> {
    let tasks: std::collections::HashMap<usize, TaskView> = s
        .tasks_for(home)
        .into_iter()
        .filter(|task| {
            task.remote.as_ref().is_some_and(|remote| remote.stop_owed)
                || matches!(
                    task.state,
                    TaskState::Queued | TaskState::Starting | TaskState::Running
                )
                || s.threads
                    .iter()
                    .any(|t| t.id == task.child_id && t.request.is_some())
        })
        .map(|task| (task.child_id, task))
        .collect();
    let mut seen: std::collections::HashSet<usize> = Default::default();
    let mut members = Vec::new();
    let ids: Vec<usize> = s
        .captain_crew(captain)
        .into_iter()
        .chain(tasks.keys().copied())
        .filter(|id| seen.insert(*id))
        .collect();
    for id in ids {
        let thread = s.threads.iter().find(|t| t.id == id);
        let task = tasks.get(&id);
        if thread.is_none() && task.is_none() {
            continue;
        }
        let (status, word) = match task {
            Some(task) => {
                let (status, word) = task_status(s, task);
                (status, word.to_string())
            }
            None => {
                // An untasked member is independent crew: a request in flight is a
                // needs-you like any task's, the rest follows the thread's own status.
                let status = if thread.is_some_and(|t| t.request.is_some()) {
                    Status::Waiting
                } else {
                    thread.map(|t| t.status).unwrap_or(Status::Ready)
                };
                (status, crate::model::status_word(status).into())
            }
        };
        let waiting = status == Status::Waiting;
        let clock = match task {
            Some(task) if waiting || task.remote.is_some() => word.clone(),
            Some(task) => task_clock(task),
            None => word.clone(),
        };
        let rank = if thread.is_some_and(|t| t.request.is_some()) || status == Status::Waiting {
            0
        } else if status.busy()
            || task
                .is_some_and(|task| matches!(task.state, TaskState::Starting | TaskState::Running))
        {
            1
        } else if task.is_some_and(|task| task.state == TaskState::Queued) {
            2
        } else {
            3
        };
        members.push(RosterMember {
            thread_id: id,
            title: task.map(|task| task.title.clone()).unwrap_or_else(|| {
                let title = thread.map(|t| t.title.as_str()).unwrap_or_default();
                if title.is_empty() {
                    "Untitled".into()
                } else {
                    title.to_string()
                }
            }),
            model: task
                .map(|task| task.model_label.clone())
                .or_else(|| thread.map(|thread| thread.model.clone()))
                .unwrap_or_default(),
            status,
            word,
            clock,
            remote_server: task
                .and_then(|task| task.remote.as_ref().map(|r| r.server_id.clone()))
                .or_else(|| thread.and_then(|t| (t.machine != "local").then(|| t.machine.clone()))),
            rank,
            updated: thread.map(|t| t.updated).unwrap_or(0),
            queued: task.is_some_and(|task| task.state == TaskState::Queued),
            place: worker_placement(s, id).map(|placement| {
                placement_details(&placement, &placement_machine(s, id, &placement))
            }),
        });
    }
    members.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| b.updated.cmp(&a.updated)));
    members
}

/// The other workers this machine runs for one peer delegation: threads sharing the
/// open worker's origin server and parent task. Same rank and status rules as
/// `roster_members`' untasked arm.
pub(super) fn peer_sibling_members(
    s: &State,
    open: usize,
    origin: &PeerOrigin,
) -> Vec<RosterMember> {
    let mut members: Vec<RosterMember> = s
        .threads
        .iter()
        .filter(|thread| {
            thread.id != open
                && thread.peer_origin.as_ref().is_some_and(|other| {
                    other.server_id == origin.server_id
                        && other.parent_thread_id == origin.parent_thread_id
                })
        })
        .map(|thread| {
            let status = if thread.request.is_some() {
                Status::Waiting
            } else {
                thread.status
            };
            let word = crate::model::status_word(status).to_string();
            let rank = if thread.request.is_some() || status == Status::Waiting {
                0
            } else if status.busy() {
                1
            } else {
                3
            };
            RosterMember {
                thread_id: thread.id,
                title: if thread.title.is_empty() {
                    "Untitled".into()
                } else {
                    thread.title.clone()
                },
                model: thread.model.clone(),
                status,
                clock: word.clone(),
                word,
                remote_server: (thread.machine != "local").then(|| thread.machine.clone()),
                rank,
                updated: thread.updated,
                queued: false,
                place: worker_placement(s, thread.id).map(|placement| {
                    placement_details(&placement, &placement_machine(s, thread.id, &placement))
                }),
            }
        })
        .collect();
    members.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| b.updated.cmp(&a.updated)));
    members
}

/// KSG's `ProviderSubagentBar` identity and context on two lines.
/// Click opens the worker; a secondary press opens the member's More menu. `key` keeps the
/// roster's `crew-chip-{id}` keys and lets the siblings menu reuse the builder with its own.
pub(super) fn roster_row(
    s: &State,
    p: Palette,
    member: &RosterMember,
    key: String,
    menu: bool,
) -> El {
    let child = member.thread_id;
    let (_, tertiary) = quiet_inks(p);
    let waiting = member.status == Status::Waiting;
    let clock_width = super::text_width(&member.clock) + 4.0;
    let mut row_ = row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(match &member.place {
            Some(place) => format!(
                "{}. {}. {}. Open the worker",
                member.title,
                member.word,
                place.replace('\n', ". ")
            ),
            None => format!("{}. {}. Open the worker", member.title, member.word),
        })
        .focusable(true)
        .center_y()
        .gap(8.0)
        .h(44.0)
        .px(10.0)
        .radius(ROW_RADIUS)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .hover_group_focus()
        .transition(Transition::offset(0.0, 4.0).combined(Transition::opacity()))
        .on_press(move |s: &mut State| {
            if waiting {
                // On a Needs you member, jump to the home's request bar at that item's page
                // and dismiss a covering Inspector so the request is immediately visible.
                if !s.is_captain_home(s.current())
                    && let Some(captain) = s
                        .threads
                        .iter()
                        .find(|t| t.id == child)
                        .and_then(|t| t.crew_captain.or(t.captain))
                {
                    s.open_captain(captain);
                }
                let items = needs_you_items(s);
                if let Some(idx) = items.iter().position(|item| item.thread_id == child) {
                    s.needs_you_page = thread_request_pages(s) + idx;
                    if s.inspector_covers() {
                        let key = s.panel_key();
                        s.right_panels.close(&key);
                    }
                    return;
                }
            }
            open_thread(s, child);
        })
        .child(crewmate_avatar(s, p, &member.model, member.status, 24.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(label(&member.title, TITLE, p.ink).truncate())
                .child(
                    label(
                        // The clock column names the state, so this line is the model.
                        if member.model.is_empty() {
                            member.word.clone()
                        } else {
                            member.model.clone()
                        },
                        SMALL,
                        p.muted,
                    )
                    .truncate(),
                ),
        )
        .child(
            member
                .remote_server
                .as_ref()
                .map(|server| {
                    machine_tag(s, p, server, format!("crew-machine-{child}"), SMALL, true)
                })
                .unwrap_or_else(el),
        )
        // The timer never shrinks: the title gives way first.
        .child(
            label(
                member.clock.clone(),
                if waiting { super::META } else { TIMER },
                if waiting { p.brass } else { tertiary },
            )
            .min_w(clock_width)
            .key(format!("crew-chip-clock-{child}")),
        );
    if menu {
        row_ = row_
            .child(
                icon_button(
                    &format!("crew-chip-more-{child}"),
                    "Crew actions",
                    "more",
                    p,
                    move |s: &mut State| {
                        s.brief.more = Some(child);
                    },
                )
                .group_opacity(0.0, 1.0),
            )
            .on_secondary_press(move |s: &mut State, _at, _| {
                s.brief.more = Some(child);
            });
    }
    if let Some(place) = &member.place {
        row_ = row_.help(place.clone());
    }
    row_
}

/// Pages the current thread's own request takes at the front of the unified request
/// banner: one when a non-questionnaire request is showing, otherwise none.
pub(super) fn thread_request_pages(s: &State) -> usize {
    let t = s.current();
    usize::from(
        t.request.is_some()
            && !t.request_held_by_typing_guard()
            && t.native.questionnaire.is_none(),
    )
}

/// Total number of paged requests in the unified composer request surface: the current
/// thread's own non-questionnaire request (if visible), plus Captain home crew live
/// requests and user-addressed crew report questions.
pub(super) fn unified_request_total(s: &State) -> usize {
    let t = s.current();
    let thread_req = thread_request_pages(s);
    if s.is_captain_home(t) {
        thread_req + needs_you_items(s).len() + super::crew_questions_ui::user_question_count(s)
    } else {
        thread_req
    }
}

/// The live crew approval/question items for the current Captain home.
pub(super) fn needs_you_items(s: &State) -> Vec<AttentionItem> {
    let t = s.current();
    if !s.is_captain_home(t) {
        return Vec::new();
    }
    s.attention()
        .into_iter()
        .filter(|item| needs_you_item(s, t.id, item))
        .collect()
}

pub(super) fn pager_controls(p: Palette, page: usize, total: usize) -> El {
    row()
        .gap(4.0)
        .center_y()
        .child(icon_button(
            "needs-you-prev",
            "Previous request",
            "chevron-left",
            p,
            move |s: &mut State| {
                let total = unified_request_total(s);
                let max = total.saturating_sub(1);
                s.needs_you_page = s.needs_you_page.min(max).saturating_sub(1);
            },
        ))
        .child(label(format!("{}/{total}", page + 1), TIMER, p.faint).key("needs-you-page-count"))
        .child(icon_button(
            "needs-you-next",
            "Next request",
            "chevron-right",
            p,
            move |s: &mut State| {
                let total = unified_request_total(s);
                let max = total.saturating_sub(1);
                s.needs_you_page = (s.needs_you_page + 1).min(max);
            },
        ))
}

/// The one paged crew request inside the unified composer request surface, ported from
/// KSG Code's `ComposerRequestCard` and `ChildThreadApprovals`: compact header row with
/// status mark, request title, source link, pager and actions. Under 520 pt the actions
/// wrap into a footer row.
pub(super) fn needs_you_card(
    s: &State,
    p: Palette,
    item: &AttentionItem,
    page: usize,
    total: usize,
    lane: f32,
) -> El {
    let id = item.thread_id;
    let approval = item.kind == AttentionKind::Approval;
    let Some(child) = s.threads.iter().find(|c| c.id == id) else {
        return el();
    };
    let Some(request) = &child.request else {
        return el();
    };
    let remote = s.remote_task(id);
    let linked = remote.is_none_or(|remote| {
        s.client
            .as_ref()
            .is_some_and(|client| client.commands_for(&remote.server_id).is_some())
    });
    let known = remote.is_none_or(|remote| {
        remote.request.as_ref().is_some_and(|r| {
            matches!(
                r["kind"].as_str(),
                Some("confirm" | "input" | "editor" | "select")
            )
        })
    });
    let can_answer =
        known && linked && remote.is_none_or(|remote| s.machine_connected(&remote.server_id));
    let title = crate::model::string(request, "title");
    let message = crate::model::string(request, "message");
    // Multi-line or long authorization payloads (such as Claude's plan approval) cannot
    // be authorized safely from a one-line bar; route them to Review instead of blind Approve.
    let needs_full_review = approval && (message.contains('\n') || message.chars().count() > 140);
    let kind_word = if approval { "Approval" } else { "Question" };
    let mut meta = row()
        .gap(6.0)
        .center_y()
        .child(meta_text(kind_word, p.brass))
        .child(
            row()
                .key(format!("needs-you-source-{id}"))
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(format!("Open {}", item.short_title))
                .focusable(true)
                .center_y()
                .gap(4.0)
                .radius(4.0)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .on_press(move |s: &mut State| open_thread(s, id))
                .child(
                    label(item.short_title.clone(), SMALL, p.muted)
                        .truncate()
                        .min_w(0.0),
                )
                .child(icon("chevron-right", p.faint).size(12.0, 12.0)),
        );
    if !known && let Some(remote) = remote {
        meta = row()
            .gap(6.0)
            .center_y()
            .child(meta_text(kind_word, p.brass))
            .child(
                label(
                    format!("Update required on {} to answer.", remote.label),
                    SMALL,
                    p.muted,
                )
                .truncate()
                .min_w(0.0),
            );
    }
    if !linked && let Some(remote) = remote {
        meta = row()
            .gap(6.0)
            .center_y()
            .child(meta_text(kind_word, p.brass))
            .child(
                label(
                    format!("Link this desktop to {} to answer.", remote.label),
                    SMALL,
                    p.muted,
                )
                .key(format!("link-request-notice-{id}"))
                .truncate()
                .min_w(0.0),
            );
    }
    let mut actions = row().gap(6.0).center_y();
    if total > 1 {
        actions = actions.child(pager_controls(p, page, total));
    }
    actions = if needs_full_review {
        actions.child(
            solid_button(format!("review-{id}"), "Review", p, move |s| {
                open_thread(s, id);
            })
            .disabled(!can_answer),
        )
    } else if approval {
        actions.child(super::approval_actions(s, id, true, can_answer, p))
    } else {
        actions.child(
            solid_button(format!("answer-{id}"), "Open to answer", p, move |s| {
                open_thread(s, id);
                s.current_mut().reply.focused = true;
            })
            .disabled(!can_answer),
        )
    };
    if !linked && remote.is_some() {
        actions = actions.child(ghost_button(
            s,
            format!("link-request-{id}"),
            "Link machine",
            p,
            |s| {
                s.machines.dialog = true;
            },
        ));
    }
    let lane_narrow = lane < 520.0;
    let mut content = col()
        .key(format!("needs-you-{id}"))
        .role("group")
        .label(format!("{kind_word} from {}", item.short_title))
        .help(if !message.is_empty() && message != title {
            message.to_string()
        } else {
            String::new()
        })
        .w_full()
        .min_w(0.0)
        .pl(10.0)
        .pr(13.0)
        .py(6.0)
        .gap(6.0)
        .transition(Transition::offset(0.0, 6.0).combined(Transition::opacity()));
    let row_ = row()
        .gap(10.0)
        .center_y()
        .child(mark_slot(Status::Waiting, p))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    label(
                        if title.is_empty() {
                            item.short_title.clone()
                        } else {
                            title.to_string()
                        },
                        TextStyle::new(13.0, 500.0).tracking(-0.08),
                        p.ink,
                    )
                    .truncate()
                    .min_w(0.0),
                )
                .child(meta),
        );
    if lane_narrow {
        content = content.child(row_).child(
            row()
                .key("needs-you-actions-wrap")
                .gap(6.0)
                .center_y()
                .justify_end()
                .child(actions),
        );
    } else {
        content = content.child(row_.child(actions));
    }
    content
}

/// One attention row's filter for `needs_you`: canonical crew membership overrides the
/// historical parent link, so a moved or released worker's questions reach only its
/// current captain. Ordinary delegation keeps the parent match.
pub(crate) fn needs_you_item(s: &State, home: usize, item: &AttentionItem) -> bool {
    if !matches!(item.kind, AttentionKind::Approval | AttentionKind::Question) {
        return false;
    }
    let Some(child) = s.threads.iter().find(|thread| thread.id == item.thread_id) else {
        return false;
    };
    if s.captain_provenanced(child) {
        s.captain_member_at(child, home)
    } else {
        item.parent_id == Some(home)
    }
}

/// Whether `parent_bar` draws, so the composer's stack budget can price it before building.
/// Spec §3.8: the bar only exists while the worker has something the user must act on.
pub(super) fn parent_bar_shows(s: &State) -> bool {
    worker_notice(s, s.current().id).is_some()
}

/// What the notice bar has to say, when a worker or recruited member thread has
/// something the user must act on: a placement that needs recovery, an offline or
/// outdated remote target, or a pending crew change. `None` on a healthy thread.
fn worker_notice(s: &State, thread: usize) -> Option<String> {
    let t = s.threads.iter().find(|t| t.id == thread)?;
    let is_worker =
        t.peer_origin.is_some() || (t.delegated && t.parent.is_some()) || t.crew_captain.is_some();
    if !is_worker {
        return None;
    }
    // A failed placement the user must retry or skip.
    if let Some(placement) = worker_placement(s, thread) {
        if placement.phase == PlacementPhase::Failed {
            let sender = t
                .captain
                .or(t.crew_captain)
                .and_then(|c| s.captains.get(c))
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "the captain".into());
            return Some(format!(
                "Setup failed. {}",
                placement_recovery(s, thread, &sender)
            ));
        }
    }
    // A remote target needing attention.
    if let Some(remote) = s.remote_task(thread) {
        match remote.state {
            crate::model::RemoteState::TargetOffline => {
                return Some(format!(
                    "{} went offline. Work resumes when it reconnects.",
                    remote.label
                ));
            }
            crate::model::RemoteState::UpdateRequired => {
                return Some(format!("Update required on {} to continue.", remote.label));
            }
            _ => {}
        }
    }
    // A task that stopped and did not resume by itself.
    if interrupted_task(s, thread).is_some() {
        return Some("This task stopped before it finished.".into());
    }
    // A recruited member's pending crew change.
    if t.peer_origin.is_none() {
        let server = s.machines.selected.clone();
        let pending = s
            .crew_pending
            .iter()
            .find(|((srv, _), change)| change.thread == thread && *srv == server);
        if let Some((_, change)) = pending {
            let word = match change.kind {
                crate::model::CrewKind::Join => "Joining the crew…",
                crate::model::CrewKind::Release => "Leaving the crew…",
                crate::model::CrewKind::Move { .. } => "Moving to another crew…",
            };
            return Some(word.to_string());
        }
    }
    None
}

/// The worker's own task, when it is interrupted and reconciled, so the user can resume or
/// dismiss it.
fn interrupted_task(s: &State, thread: usize) -> Option<String> {
    s.resumable_task(thread)
}

/// The worker's notice bar (spec §3.8): 36 pt, borderless `p.selected`, an alert icon in
/// brass and the reason in SMALL ink. A healthy worker shows nothing — the top bar
/// already carries the captain link and status.
fn notice_bar(key: &'static str, reason: String, action: Option<El>, p: Palette) -> El {
    let mut bar = row()
        .key(key)
        .role("status")
        .label(reason.clone())
        .min_h(36.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(8.0)
        .fill(p.selected)
        .child(icon("alert", p.brass).size(16.0, 16.0))
        .child(label(reason, SMALL, p.ink).wrap().grow(1.0).min_w(0.0));
    if let Some(button) = action {
        bar = bar.child(button);
    }
    bar
}

pub(super) fn parent_bar(s: &State, p: Palette, _lane: f32) -> Option<El> {
    let t = s.current();
    if t.crew_captain.is_some() && !t.delegated && t.peer_origin.is_none() {
        // A recruited root member's notices go to `crew_bar`, not here.
        return None;
    }
    let thread_id = t.id;
    let setup_failed = t
        .worktree_setup
        .as_ref()
        .is_some_and(|setup| setup.phase == crate::model::WorktreeSetupPhase::Failed);
    let offline = s.remote_task(thread_id).is_some_and(|r| {
        matches!(
            r.state,
            crate::model::RemoteState::TargetOffline | crate::model::RemoteState::UpdateRequired
        )
    });
    let interrupted = interrupted_task(s, thread_id);
    worker_notice(s, thread_id).map(|reason| {
        let action = if setup_failed {
            Some(ghost_button(
                s,
                "parent-bar-retry-setup".into(),
                "Retry setup",
                p,
                |s: &mut State| {
                    s.retry_worktree_setup();
                },
            ))
        } else if offline {
            Some(ghost_button(
                s,
                "parent-bar-machines".into(),
                "Machines",
                p,
                |s: &mut State| s.machines.dialog = true,
            ))
        } else if let Some(task) = interrupted {
            // Resume and Dismiss sit on the worker, where its partial work is in view.
            let (resume, dismiss) = (task.clone(), task);
            Some(
                row()
                    .gap(6.0)
                    .center_y()
                    .child(
                        ghost_button(
                            s,
                            format!("resume-task-{thread_id}"),
                            "Resume",
                            p,
                            move |s| {
                                if let Err(error) = s.resume_task(&resume) {
                                    s.current_mut().error = Some(error);
                                }
                            },
                        )
                        .h(24.0),
                    )
                    .child(
                        ghost_button(
                            s,
                            format!("dismiss-task-{thread_id}"),
                            "Dismiss",
                            p,
                            move |s| {
                                if let Err(error) = s.dismiss_task(&dismiss) {
                                    s.current_mut().error = Some(error);
                                }
                            },
                        )
                        .h(24.0),
                    ),
            )
        } else {
            None
        };
        notice_bar("parent-bar", reason, action, p)
    })
}

/// Whether `crew_bar` draws, so the composer's stack budget can price it before building.
/// Only a pending crew change on a recruited member counts — a healthy member shows
/// nothing.
pub(super) fn crew_bar_shows(s: &State) -> bool {
    let t = s.current();
    !t.delegated
        && t.peer_origin.is_none()
        && t.crew_captain.is_some()
        && worker_notice(s, t.id).is_some()
}

/// The independent crew member's notice bar: same borderless `p.selected` treatment as
/// `parent_bar`, shown only while a pending crew change needs the user.
pub(super) fn crew_bar(s: &State, p: Palette, _lane: f32) -> Option<El> {
    let t = s.current();
    if t.delegated || t.peer_origin.is_some() || t.crew_captain.is_none() {
        return None;
    }
    worker_notice(s, t.id).map(|reason| notice_bar("crew-bar", reason, None, p))
}

/// The restart notice: what happened, Review (the first interrupted task) and dismiss.
pub(super) fn recovery_banner(s: &State, p: Palette) -> Option<El> {
    let Some(notice) = &s.recovery_notice else {
        return None;
    };
    let interrupted = s
        .attention()
        .into_iter()
        .find(|item| item.kind == AttentionKind::Interrupted)
        .map(|item| item.thread_id);
    // The text follows the live count of stopped tasks. The stored notice covers an unclean
    // shutdown, where nothing is marked as stopped yet.
    let stopped = s
        .attention()
        .iter()
        .filter(|item| item.kind == AttentionKind::Interrupted)
        .count();
    let text = match stopped {
        0 => notice.clone(),
        1 => "1 task stopped when Shina closed. Review partial changes before resuming.".into(),
        n => {
            format!("{n} tasks stopped when Shina closed. Review partial changes before resuming.")
        }
    };
    let mut actions = row().key("recovery-actions").gap(6.0).center_y();
    if let Some(id) = interrupted {
        actions = actions.child(solid_button(
            "review-recovery".into(),
            "Review",
            p,
            move |s| {
                s.recovery_notice = None;
                open_thread(s, id);
            },
        ));
    }
    // The dismiss control sits at the top right of the text, where a card's close control is.
    Some(
        col()
            .key("recovery-notice")
            .role("status")
            .label(text.clone())
            .mb(6.0)
            .padding(10.0)
            .gap(8.0)
            .radius(6.0)
            .fill(p.card)
            .border(1.0, p.frame)
            .transition(Transition::offset(0.0, -6.0).combined(Transition::opacity()))
            .child(
                row()
                    .gap(8.0)
                    .child(el().pt(1.0).child(icon("alert", p.brass)))
                    .child(
                        label(text, SMALL.lh(17.0), p.ink)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(
                        icon_button("dismiss-recovery", "Dismiss notice", "close", p, |s| {
                            s.recovery_notice = None
                        })
                        .mt(-3.0)
                        .mr(-3.0),
                    ),
            )
            .child(actions),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_rows_require_the_exact_saved_delivery_and_captain() {
        let mut state = State::headless(
            std::env::temp_dir().join(format!("shina-report-row-{}", uuid::Uuid::new_v4())),
        );
        state.load_global_for_test();
        state.demo = true;
        state.captains = crate::captains::Captain::presets();
        state.open_captain(0);
        let id = state.next_id;
        state.next_id += 1;
        let mut worker = Thread::new(id);
        worker.crew_captain = Some(0);
        worker.captain = Some(0);
        worker.run_id = Some("report-row-run".into());
        state.threads.push(worker);
        state.demo = false;
        let report = state
            .mcp_call(
                id,
                "captain_crew_report",
                &json!({
                    "requestId": uuid::Uuid::new_v4().to_string(),
                    "outcome":"blocked", "summary":"Needs a decision",
                    "question":{"text":"Which branch?", "recipient":"user"}
                }),
            )
            .unwrap();
        let text = format!(
            "{}[delivery {}]\nCrew report",
            crate::model::INJECTION_PREFIX,
            report["reportId"].as_str().unwrap()
        );
        let palette = super::palette(false);
        assert!(report_message(&state, palette, &text).is_some());
        assert!(report_message(&state, palette, "Crew report").is_none());
        assert!(
            report_message(
                &state,
                palette,
                &format!(
                    "{}[delivery unknown]\nCrew report",
                    crate::model::INJECTION_PREFIX
                )
            )
            .is_none()
        );
        state.open_captain(1);
        assert!(report_message(&state, palette, &text).is_none());
    }

    #[test]
    fn completion_messages_parse_with_multiline_replies() {
        let text = "Shina: task #4 finished\nTask id: a-1\nTitle: Review\nStatus: completed\nLast reply:\nLine one\nLine two\nError: none";
        let c = parse_completion(text).unwrap();
        assert_eq!(
            (c.child, c.title, c.status, c.reply, c.error),
            (4, "Review", "completed", "Line one\nLine two", "none")
        );
        assert!(parse_completion("Shina: task from parent #1\n\nDo it").is_none());
    }

    #[test]
    fn untasked_crew_members_classify_from_thread_state() {
        let mut state = State::headless(
            std::env::temp_dir().join(format!("shina-roster-test-{}", uuid::Uuid::new_v4())),
        );
        state.load_global_for_test();
        state.demo = true;
        state.captains = crate::captains::Captain::presets();
        state.open_captain(0);
        let home = state.current().id;
        let mut members = Vec::new();
        for (title, status, request) in [
            ("Waiting on approval", Status::Ready, true),
            ("Working alone", Status::Running, false),
            ("Idle member", Status::Ready, false),
        ] {
            let id = state.next_id;
            state.next_id += 1;
            let mut thread = Thread::new(id);
            thread.title = title.into();
            thread.crew_captain = Some(0);
            thread.captain = Some(0);
            thread.status = status;
            if request {
                thread.request = Some(json!({"method":"confirm"}));
            }
            members.push(id);
            state.threads.push(thread);
        }
        let roster = roster_members(&state, home, 0);
        assert_eq!(roster.len(), 3);
        let by_id = |id: usize| roster.iter().find(|m| m.thread_id == id).unwrap();
        // A pending request is needs-you even when the thread status lags it,
        // and it sorts to the front with the other attention rows.
        assert_eq!(by_id(members[0]).status, Status::Waiting);
        assert_eq!(by_id(members[0]).rank, 0);
        assert_eq!(by_id(members[1]).status, Status::Running);
        assert_eq!(by_id(members[1]).rank, 1);
        assert_eq!(by_id(members[2]).status, Status::Ready);
        assert_eq!(by_id(members[2]).rank, 3);
        assert_eq!(roster[0].thread_id, members[0]);
    }
}
