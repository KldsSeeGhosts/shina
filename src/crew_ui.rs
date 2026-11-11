//! Captain and crew surfaces: task cards and results in a captain's transcript, the crew tray,
//! the needs-you stack and the worker's parent bar above the composer, and the attention row and
//! recovery notice at the top of the sidebar.
//!
//! One design language for all of them: hover is ink at 6 %, pressed or active at 8 %, hairline
//! borders at 10 % ink, rows at a 10 px radius, and motion of 0.18 s on
//! `cubic-bezier(.23, 1, .32, 1)`.
use super::*;
use crate::model::{AttentionItem, AttentionKind, TaskState, TaskView};
use rune::Transition;

/// Row corners, shared by cards, chips and popover items.
const ROW_RADIUS: f32 = 10.0;
/// Width of a card's elapsed-time slot: room for "1h 05m" and "#8 queued" in tabular figures.
const ELAPSED_SLOT: f32 = 72.0;
/// Where a card's second and later lines start: past the status mark's 14 px slot and its gap.
const CARD_INDENT: f32 = 22.0;
/// Below this lane width the crew tray stacks its chips, as KSG's composer bars drop to a column.
const TRAY_STACK_WIDTH: f32 = 560.0;

fn motion() -> rune::Curve {
    rune::Curve::tween(0.18, rune::Easing::cubic(0.23, 1.0, 0.32, 1.0))
}
/// Ink washes. Rune blends in linear light, where a light ink on a dark ground reads stronger,
/// so the dark theme takes a little over half the alpha for the same visible step.
fn hover_wash(s: &State, p: Palette) -> Color {
    p.ink.alpha(if s.dark { 0.035 } else { 0.06 })
}
fn active_wash(s: &State, p: Palette) -> Color {
    p.ink.alpha(if s.dark { 0.05 } else { 0.08 })
}
fn hairline(p: Palette) -> Color {
    p.ink.alpha(0.10)
}

/// A quiet, outlined action: Decline, Dismiss, Open.
fn ghost_button(
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
        .radius(8.0)
        .border(1.0, hairline(p))
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .on_press(action)
        .child(label(title, TextStyle::new(12.5, 500.0), p.ink))
}
/// The one filled action in a group: Approve, Resume, Open to answer.
fn solid_button(key: String, title: &str, p: Palette, action: impl Fn(&mut State) + 'static) -> El {
    row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(title)
        .focusable(true)
        .center_y()
        .h(28.0)
        .px(12.0)
        .radius(8.0)
        .fill(p.teal)
        .hover(p.teal.alpha(0.88))
        .on_press(action)
        .child(label(title, TextStyle::new(12.5, 560.0), p.on_teal))
}

fn open_thread(s: &mut State, id: usize) {
    if let Some(index) = s.threads.iter().position(|t| t.id == id) {
        s.select_thread(index);
    }
}

/// A task's place in the shared status vocabulary, and the word for it. A child that is
/// asking for something needs the user whatever its task record says.
fn task_status(s: &State, task: &TaskView) -> (Status, &'static str) {
    if s.threads
        .iter()
        .any(|t| t.id == task.child_id && t.request.is_some())
    {
        return (Status::Waiting, "Needs you");
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
fn mark_slot(status: Status, p: Palette) -> El {
    el().size(14.0, 14.0)
        .center()
        .child(status_mark(status, p, 7.0))
}

fn is_delegation(tool: &Tool) -> bool {
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

/// Whether a transcript part is only delegations in a captain's home, shown as task cards.
pub(super) fn is_card_part(s: &State, t: &Thread, part: &TranscriptPart) -> bool {
    s.is_captain_home(t)
        && matches!(part, TranscriptPart::Tools(indices)
            if indices.iter().all(|&index| is_delegation(&t.tools[index])))
}

/// A tool batch in the transcript. In a captain's home each delegation becomes a task card;
/// every other call keeps the shared tool rows.
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
    let mut part = col().gap(8.0).py(2.0);
    if !rest.is_empty() {
        part = part.child(inline_tools(s, p, &rest));
    }
    for task in &cards {
        part = part.child(task_card(s, p, task));
    }
    part
}

/// A delegated task in the captain's transcript: what it is, where and on which model it runs,
/// its live state, its last activity and its clock. The whole card opens the worker.
/// The meta line follows KSG Code's `ProviderSubagentBar` (provider mark, model, live status).
pub(super) fn task_card(s: &State, p: Palette, task: &TaskView) -> El {
    let (status, word) = task_status(s, task);
    let child = task.child_id;
    let (model, _) = split_model_label(&task.model_label);
    let (_, tertiary) = quiet_inks(p);
    let head = row()
        .center_y()
        .gap(8.0)
        .child(mark_slot(status, p))
        .child(
            label(
                &task.title,
                TextStyle::new(14.0, 560.0).tracking(-0.12),
                p.ink,
            )
            .key(format!("task-title-{child}"))
            .truncate()
            .shrink()
            .min_w(0.0),
        )
        .child(spacer())
        .child(
            row()
                .key(format!("task-clock-{child}"))
                .w(ELAPSED_SLOT)
                .min_w(ELAPSED_SLOT)
                .justify_end()
                .child(label(task_clock(task), SMALL.features(TABULAR), tertiary)),
        );
    let meta = row()
        .pl(CARD_INDENT)
        .center_y()
        .gap(6.0)
        .min_w(0.0)
        .child(label(word, SMALL, status_ink(status, p)))
        .child(label("·", SMALL, tertiary))
        .child(
            label(&task.workspace_label, SMALL, p.muted)
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(label("·", SMALL, tertiary))
        .child(provider_icon(
            s,
            p,
            model_vendor(&task.model_label, &task.model_label),
            12.0,
        ))
        .child(label(model, SMALL, p.muted).truncate().shrink().min_w(0.0));
    let mut card = col()
        .key(format!("task-card-{child}"))
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!("{}. {word}. Open the worker", task.title))
        .focusable(true)
        .gap(3.0)
        .px(12.0)
        .py(10.0)
        .radius(ROW_RADIUS)
        .fill(p.card)
        .border(1.0, hairline(p))
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .on_press(move |s: &mut State| open_thread(s, child))
        .child(head)
        .child(meta);
    // The record's fallback line is the state's id, which the meta line already says.
    if !task.last_activity_line.is_empty() && task.last_activity_line != task.state.id() {
        card = card.child(
            label(
                &task.last_activity_line,
                ACTIVITY,
                if status == Status::Failed {
                    p.danger
                } else {
                    tertiary
                },
            )
            .key(format!("task-activity-{child}"))
            .truncate()
            .pl(CARD_INDENT)
            .min_w(0.0),
        );
    }
    if task.state == TaskState::Interrupted {
        let (resume, dismiss) = (task.id.clone(), task.id.clone());
        card = card.child(
            row()
                .key(format!("task-actions-{child}"))
                .pl(CARD_INDENT)
                .pt(6.0)
                .gap(6.0)
                .child(solid_button(
                    format!("resume-task-{child}"),
                    "Resume",
                    p,
                    move |s| {
                        if let Err(error) = s.resume_task(&resume) {
                            s.current_mut().error = Some(error);
                        }
                    },
                ))
                .child(ghost_button(
                    s,
                    format!("dismiss-task-{child}"),
                    "Dismiss",
                    p,
                    move |s| {
                        if let Err(error) = s.dismiss_task(&dismiss) {
                            s.current_mut().error = Some(error);
                        }
                    },
                )),
        );
    }
    card
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

/// A worker's result in the captain's home: a quiet block on a rail under the task's context,
/// not a "Shina" bubble. The header opens the worker; the reply reads in the secondary ink.
pub(super) fn result_block(s: &State, p: Palette, wide: bool, completion: &Completion) -> El {
    let (secondary, tertiary) = quiet_inks(p);
    let child = completion.child;
    let status = match completion.status {
        "completed" => Status::Completed,
        "failed" => Status::Failed,
        "interrupted" => Status::Waiting,
        _ => Status::Ready,
    };
    let word = match completion.status {
        "completed" => "Finished",
        "failed" => "Failed",
        "cancelled" => "Dismissed",
        other => other,
    };
    let header = row()
        .key(format!("result-open-{child}"))
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!("Result from {}. Open the worker", completion.title))
        .focusable(true)
        .center_y()
        .gap(6.0)
        .h(26.0)
        .px(6.0)
        .ml(-6.0)
        .radius(8.0)
        .hover(hover_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .on_press(move |s: &mut State| open_thread(s, child))
        .child(mark_slot(status, p))
        .child(label(word, ACTIVITY, secondary))
        .child(
            label(completion.title, ACTIVITY, tertiary)
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(icon("open", tertiary).opacity(0.8));
    let reply = completion.reply.trim();
    let mut body = col().gap(6.0).pl(CARD_INDENT - 6.0).ml(6.0);
    if !reply.is_empty() && reply != "(no reply)" {
        let style = MarkdownStyle::neutral(s.dark)
            .text(secondary)
            .muted(p.muted)
            .body_font(body_style(wide))
            .inline_code_bg(p.ink.alpha(if s.dark { 0.07 } else { 0.055 }))
            .selection(p.teal_bg)
            .code_block(p.card, p.line);
        let reply = truncate(reply, 1200);
        body = body.child(
            memo(
                format!("result-reply-{child}"),
                (reply.clone(), s.dark, wide),
                || Markdown::new(&reply).render(&style),
            )
            .key(format!("result-reply-{child}")),
        );
    }
    if completion.error != "none" && !completion.error.trim().is_empty() {
        body = body.child(
            label(truncate(completion.error, 400), SMALL, p.danger)
                .wrap()
                .selectable(true),
        );
    }
    col().gap(4.0).child(header).child(
        row()
            .child(el().w(1.0).h_full().ml(6.5).fill(hairline(p)).a11y_hidden())
            .child(body.grow(1.0).min_w(0.0)),
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
        .radius(8.0)
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
                    weight: 560.0,
                    ..ACTIVITY
                },
                p.ink,
            )
            .truncate()
            .shrink()
            .min_w(0.0),
        )
        .child(icon("open", tertiary).opacity(0.8));
    col().gap(4.0).child(header).child(
        row()
            .child(el().w(1.0).h_full().ml(7.5).fill(hairline(p)).a11y_hidden())
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

/// The crew tray: running and queued tasks as compact chips above a captain's composer, in the
/// place KSG Code's `QueuedRunsControl` takes. It wraps on a wide lane and stacks on a narrow one.
pub(super) fn crew_tray(s: &State, p: Palette, lane: f32) -> Option<El> {
    let t = s.current();
    if !s.is_captain_home(t) {
        return None;
    }
    let tasks: Vec<_> = s
        .tasks_for(t.id)
        .into_iter()
        .filter(|task| {
            matches!(
                task.state,
                TaskState::Queued | TaskState::Starting | TaskState::Running
            )
        })
        .collect();
    if tasks.is_empty() {
        return None;
    }
    let working = tasks
        .iter()
        .filter(|task| task.state != TaskState::Queued)
        .count();
    let queued = tasks.len() - working;
    let mut summary = vec![];
    if working > 0 {
        summary.push(format!("{working} working"));
    }
    if queued > 0 {
        summary.push(format!("{queued} queued"));
    }
    let stacked = lane < TRAY_STACK_WIDTH;
    let (_, tertiary) = quiet_inks(p);
    let mut chips = if stacked {
        col().gap(2.0)
    } else {
        row().gap(6.0).flex_wrap()
    };
    for task in &tasks {
        let (status, word) = task_status(s, task);
        let child = task.child_id;
        let mut chip = row()
            .key(format!("crew-chip-{child}"))
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("{}. {word}. Open the worker", task.title))
            .focusable(true)
            .center_y()
            .gap(7.0)
            .h(30.0)
            .pl(8.0)
            .pr(10.0)
            .radius(ROW_RADIUS)
            .hover(hover_wash(s, p))
            .press(active_wash(s, p))
            .hover_motion(motion(), motion())
            .transition(Transition::offset(0.0, 4.0).combined(Transition::opacity()))
            .on_press(move |s: &mut State| open_thread(s, child))
            .child(mark_slot(status, p))
            .child(
                label(&task.title, TextStyle::new(13.0, 500.0), p.ink)
                    .truncate()
                    .shrink()
                    .min_w(0.0),
            )
            .child(label(task_clock(task), SMALL.features(TABULAR), tertiary));
        chip = if stacked {
            chip.w_full()
        } else {
            chip.max_w(260.0).fill(p.card).border(1.0, hairline(p))
        };
        chips = chips.child(chip);
    }
    Some(
        col()
            .key("crew-tray")
            .role("group")
            .label("Crew")
            .gap(4.0)
            .child(
                row()
                    .h(20.0)
                    .px(4.0)
                    .gap(6.0)
                    .center_y()
                    .child(label("Crew", TextStyle::new(12.0, 600.0), p.muted))
                    .child(
                        label(summary.join(" · "), SMALL.features(TABULAR), tertiary)
                            .key("crew-tray-summary"),
                    ),
            )
            .child(chips),
    )
}

/// Approvals and questions from this captain's crew, ported from KSG Code's
/// `ChildThreadApprovals`: bordered `rounded-lg p-3` cards in a `max-h-80` scrolling stack.
pub(super) fn needs_you(s: &State, p: Palette) -> Option<El> {
    let t = s.current();
    if !s.is_captain_home(t) {
        return None;
    }
    let items: Vec<AttentionItem> = s
        .attention()
        .into_iter()
        .filter(|item| {
            item.parent_id == Some(t.id)
                && matches!(item.kind, AttentionKind::Approval | AttentionKind::Question)
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    let mut stack = col().gap(8.0);
    for item in &items {
        let Some(child) = s.threads.iter().find(|c| c.id == item.thread_id) else {
            continue;
        };
        let Some(request) = &child.request else {
            continue;
        };
        let id = item.thread_id;
        let approval = item.kind == AttentionKind::Approval;
        let title = crate::model::string(request, "title");
        let message = crate::model::string(request, "message");
        let mut detail = col().gap(3.0);
        if !title.is_empty() {
            detail = detail.child(label(title, TextStyle::new(13.0, 500.0), p.ink).wrap());
        }
        if !message.is_empty() {
            detail = detail.child(
                label(message, SMALL.lh(18.0), p.muted)
                    .wrap_anywhere()
                    .selectable(true),
            );
        }
        let mut actions = row().gap(6.0).flex_wrap().center_y();
        actions = if approval {
            actions
                .child(solid_button(
                    format!("approve-{id}"),
                    "Approve",
                    p,
                    move |s| {
                        let _ = s.respond_to_request(id, json!({"confirmed": true}));
                    },
                ))
                .child(ghost_button(
                    s,
                    format!("decline-{id}"),
                    "Decline",
                    p,
                    move |s| {
                        let _ = s.respond_to_request(id, json!({"confirmed": false}));
                    },
                ))
                .child(ghost_button(
                    s,
                    format!("open-request-{id}"),
                    "Open",
                    p,
                    move |s| open_thread(s, id),
                ))
        } else {
            actions.child(solid_button(
                format!("answer-{id}"),
                "Open to answer",
                p,
                move |s| {
                    open_thread(s, id);
                    s.current_mut().reply.focused = true;
                },
            ))
        };
        stack = stack.child(
            col()
                .key(format!("needs-you-{id}"))
                .role("group")
                .label(format!(
                    "{} from {}",
                    if approval { "Approval" } else { "Question" },
                    item.short_title
                ))
                .padding(12.0)
                .gap(8.0)
                .radius(ROW_RADIUS)
                .fill(p.card)
                .border(1.0, hairline(p))
                .transition(Transition::offset(0.0, 6.0).combined(Transition::opacity()))
                .child(
                    row()
                        .center_y()
                        .gap(8.0)
                        .child(mark_slot(Status::Waiting, p))
                        .child(label(
                            if approval {
                                "Approval needed from"
                            } else {
                                "Question from"
                            },
                            TextStyle::new(13.0, 500.0),
                            p.muted,
                        ))
                        .child(
                            label(&item.short_title, TextStyle::new(13.0, 560.0), p.ink)
                                .truncate()
                                .shrink()
                                .min_w(0.0),
                        ),
                )
                .child(
                    col()
                        .pl(CARD_INDENT)
                        .gap(10.0)
                        .child(
                            col()
                                .max_h(160.0)
                                .scroll(ScrollSpec {
                                    stick_bottom: false,
                                })
                                .child(detail),
                        )
                        .child(actions),
                ),
        );
    }
    Some(
        col()
            .key("needs-you")
            .role("region")
            .label("Needs you")
            .max_h(320.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(stack),
    )
}

/// "Delegated by <captain> · Runs on its own · Open captain" above a worker's composer, ported
/// from KSG Code's `ProviderSubagentBar` (`min-h-12 rounded-2xl py-2 ps-5 pe-2 text-sm`).
pub(super) fn parent_bar(s: &State, p: Palette, lane: f32) -> Option<El> {
    let t = s.current();
    let parent = t
        .parent
        .filter(|_| t.delegated)
        .and_then(|id| s.threads.iter().find(|thread| thread.id == id))?;
    let parent_id = parent.id;
    let captain = parent.captain.or(t.captain);
    let (_, tertiary) = quiet_inks(p);
    let task = s
        .tasks_for(parent_id)
        .into_iter()
        .find(|task| task.child_id == t.id);
    let mut who = row().center_y().gap(8.0).min_w(0.0).shrink().child(label(
        "Delegated by",
        TextStyle::new(14.0, 400.0),
        p.muted,
    ));
    who = match captain.and_then(|c| s.captains.get(c).map(|profile| (c, profile))) {
        Some((index, profile)) => who.child(avatar(s, p, index, 20.0, Status::Ready)).child(
            label(&profile.name, TextStyle::new(14.0, 560.0), p.ink)
                .key("parent-bar-name")
                .truncate()
                .shrink()
                .min_w(0.0),
        ),
        None => who.child(
            label(&parent.title, TextStyle::new(14.0, 560.0), p.ink)
                .key("parent-bar-name")
                .truncate()
                .shrink()
                .min_w(0.0),
        ),
    };
    let mut bar = row()
        .key("parent-bar")
        .role("group")
        .label("Delegated work")
        .min_h(48.0)
        .py(8.0)
        .pl(20.0)
        .pr(8.0)
        .gap(12.0)
        .center_y()
        .radius(16.0)
        .border(1.0, hairline(p))
        .child(who);
    if let Some(task) = &task {
        let (status, word) = task_status(s, task);
        let clock = task_clock(task);
        bar = bar.child(
            row()
                .key("parent-bar-status")
                .center_y()
                .gap(6.0)
                .shrink()
                .min_w(0.0)
                .child(mark_slot(status, p))
                .child(label(word, SMALL, status_ink(status, p)))
                .child(label(clock, SMALL.features(TABULAR), tertiary)),
        );
    }
    bar = bar.child(spacer());
    if lane >= 640.0 {
        bar = bar.child(label(
            "Runs on its own",
            TextStyle::new(13.0, 400.0),
            p.muted,
        ));
    }
    Some(
        bar.child(
            row()
                .key("open-captain")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(if captain.is_some() {
                    "Open captain"
                } else {
                    "Open parent"
                })
                .focusable(true)
                .center_y()
                .gap(6.0)
                .h(32.0)
                .px(10.0)
                .radius(ROW_RADIUS)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .hover_motion(motion(), motion())
                .on_press(move |s: &mut State| open_thread(s, parent_id))
                .child(icon("back", p.muted))
                .child(label(
                    if captain.is_some() {
                        "Open captain"
                    } else {
                        "Open parent"
                    },
                    TextStyle::new(13.0, 500.0),
                    p.ink,
                )),
        ),
    )
}

fn attention_word(kind: AttentionKind) -> &'static str {
    match kind {
        AttentionKind::Approval => "Approval",
        AttentionKind::Question => "Question",
        AttentionKind::Failed => "Failed",
        AttentionKind::Interrupted => "Interrupted",
        AttentionKind::ReadyForReview => "Results ready",
    }
}
fn attention_status(kind: AttentionKind) -> Status {
    match kind {
        AttentionKind::Failed => Status::Failed,
        AttentionKind::ReadyForReview => Status::Completed,
        _ => Status::Waiting,
    }
}

/// The zero-size mark at the end of the sidebar's title-strip header where the attention pill
/// rests. The strip has a fixed height, so the pill arriving never moves a sidebar row.
pub(super) const ATTENTION_ANCHOR: &str = "attention-anchor";

/// "N need you" at the top of the sidebar, shown only while something needs the user. It is
/// one 28 px pill whatever the count, laid over the title strip beside Shina's name, so it
/// slides and fades in without moving anything. A narrow sidebar shows the count alone.
pub(super) fn attention_row(s: &State, p: Palette, compact: bool) -> Option<El> {
    let count = s.attention().len();
    if count == 0 {
        return None;
    }
    let open = s.attention_menu;
    let mut pill = row()
        .key("attention-row")
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!("{count} need you. Show them"))
        .help(format!("{count} need you"))
        .focusable(true)
        .h(28.0)
        .pl(6.0)
        .pr(if compact { 8.0 } else { 6.0 })
        .gap(5.0)
        .center_y()
        .radius(ROW_RADIUS)
        .fill(if open {
            active_wash(s, p)
        } else {
            p.gold.alpha(0.10)
        })
        .hover(active_wash(s, p))
        .press(active_wash(s, p))
        .hover_motion(motion(), motion())
        .transition(Transition::offset(6.0, 0.0).combined(Transition::opacity()))
        .anchored_fit(ATTENTION_ANCHOR, rune::Side::Left, rune::Align::Center, 0.0)
        .on_press(|s: &mut State| s.attention_menu = !s.attention_menu)
        .child(mark_slot(Status::Waiting, p))
        .child(
            label(
                if compact {
                    count.to_string()
                } else {
                    format!("{count} need you")
                },
                TextStyle::new(12.5, 560.0).features(TABULAR),
                p.ink,
            )
            .key("attention-count"),
        );
    if !compact {
        pill = pill.child(icon("chevron", p.muted));
    }
    Some(pill)
}

/// The attention row's popover: each item names its thread and why, and opens it.
pub(super) fn attention_menu(s: &State, p: Palette) -> El {
    let mut menu = popover("attention-menu", "Needs you", s, p)
        .role("menu")
        .w(280.0)
        .anchored_fit("attention-row", rune::Side::Bottom, rune::Align::Start, 6.0)
        .modal(true, |s: &mut State| s.attention_menu = false);
    let (_, tertiary) = quiet_inks(p);
    for item in s.attention() {
        let id = item.thread_id;
        let status = attention_status(item.kind);
        menu = menu.child(
            row()
                .key(format!("attention-item-{id}"))
                .role("menuitem")
                .cursor(rune::Cursor::Pointer)
                .label(format!(
                    "{}: {}",
                    attention_word(item.kind),
                    item.short_title
                ))
                .focusable(true)
                .h(32.0)
                .px(8.0)
                .gap(8.0)
                .center_y()
                .radius(8.0)
                .hover(hover_wash(s, p))
                .press(active_wash(s, p))
                .on_press(move |s: &mut State| {
                    s.attention_menu = false;
                    open_thread(s, id);
                })
                .child(mark_slot(status, p))
                .child(
                    label(&item.short_title, TextStyle::new(13.0, 450.0), p.ink)
                        .truncate()
                        .shrink()
                        .min_w(0.0),
                )
                .child(spacer())
                .child(label(
                    attention_word(item.kind),
                    SMALL,
                    if status == Status::Failed {
                        p.danger
                    } else {
                        tertiary
                    },
                )),
        );
    }
    menu
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
    let mut actions = row().gap(6.0).pl(CARD_INDENT);
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
    Some(
        col()
            .key("recovery-notice")
            .role("status")
            .label(notice.clone())
            .mb(6.0)
            .padding(10.0)
            .gap(8.0)
            .radius(ROW_RADIUS)
            .fill(p.card)
            .border(1.0, hairline(p))
            .transition(Transition::offset(0.0, -6.0).combined(Transition::opacity()))
            .child(
                row()
                    .gap(8.0)
                    .child(el().pt(1.0).child(icon("alert", p.gold)))
                    .child(
                        label(notice, SMALL.lh(17.0), p.ink)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(icon_button(
                        "dismiss-recovery",
                        "Dismiss notice",
                        "close",
                        p,
                        |s| s.recovery_notice = None,
                    )),
            )
            .child(actions),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
