//! Structured crew questions in the needs-you stack: one card per open report
//! question addressed to the user, and a collapsed count line for the ones
//! addressed to the captain. Ported from KSG Code's `ChildThreadApprovals` card
//! (`rounded-lg border p-3` with a scrollable detail) and the free-text answer
//! half of `ComposerPendingUserInputPanel`; the backend carries text questions
//! only, so there are no option rows.
//!
//! Cards keep the record's four facts separate: a report is "Reported
//! blocked/done/failed/cancelled/progress", never verified done; delivery ("Waiting
//! to reach", "Delivered to") is not an answer; and the card stays until an answer
//! record exists. Artifact claims render as bounded labels with the owning
//! machine named — Shina never opens their paths or runs their references.
use super::META_COMPACT as META;
use super::*;
use crate::model::ReportPolicy;
use crate::model::crew_question_state::CrewQuestionView;
use crate::model::crew_reports::{ArtifactKind, CrewOutcome, QuestionRecipient};
use rune::Transition;

/// What the record proves about a report's delivery to its Captain. React and
/// Context undelivered both read "Waiting to reach" — the client cannot see a
/// hold — and Ignore reads "Not sent", the one outcome `needs_delivery` makes
/// provable. Shared by the needs-you cards and the Brief's activity digest.
pub(super) fn delivery_words(delivered: bool, policy: ReportPolicy, captain: &str) -> String {
    match (delivered, policy) {
        (true, _) => format!("Delivered to {captain}"),
        (false, ReportPolicy::Ignore) => format!("Not sent to {captain}"),
        (false, _) => format!("Waiting to reach {captain}"),
    }
}

/// The outlined pixel badge a report captured under Context or Ignore carries
/// at the end of a status line. React reports carry none: it is the default
/// and the behaviour users already know. The badge shows the snapshot, so it
/// never changes when the Captain's policy changes later.
pub(super) fn policy_badge(key: String, policy: ReportPolicy, p: Palette) -> Option<El> {
    let word = match policy {
        ReportPolicy::Context => "Context",
        ReportPolicy::Ignore => "Ignore",
        ReportPolicy::React => return None,
    };
    Some(
        outlined(
            el().key(key)
                .h(16.0)
                .px(4.0)
                .radius(6.0)
                .center()
                .child(label(word, META_LABEL, p.muted)),
            p,
        )
        .a11y_hidden(),
    )
}

/// The outcome claim's status and word. The crew thread reported the outcome;
/// nothing here marks the work itself shipped.
pub(super) fn outcome_status(outcome: CrewOutcome) -> Status {
    match outcome {
        CrewOutcome::Completed => Status::Completed,
        CrewOutcome::Failed => Status::Failed,
        CrewOutcome::Blocked => Status::Waiting,
        CrewOutcome::Progress => Status::Running,
        CrewOutcome::Cancelled => Status::Ready,
    }
}
pub(super) fn outcome_word(outcome: CrewOutcome) -> &'static str {
    match outcome {
        CrewOutcome::Completed => "Reported done",
        CrewOutcome::Failed => "Reported failed",
        CrewOutcome::Blocked => "Reported blocked",
        CrewOutcome::Progress => "Reported progress",
        CrewOutcome::Cancelled => "Reported cancelled",
    }
}

/// Opens a report's source thread, routing remote threads through their machine
/// link exactly like the needs-you cards' Open button.
fn open_report_thread(s: &mut State, id: usize) {
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

/// An artifact claim row: icon by kind, the record's bounded label, and its kind
/// word. The row is not a button; references on another machine are never opened
/// here and never carry a check mark.
fn artifact_row(p: Palette, artifact: &crate::model::crew_reports::CrewArtifact) -> El {
    let (kind, word) = match artifact.kind {
        ArtifactKind::File => ("file", "file"),
        ArtifactKind::Url => ("open", "link"),
        ArtifactKind::PullRequest => ("issue", "pull request"),
    };
    row()
        .center_y()
        .gap(7.0)
        .child(icon(kind, p.muted))
        .child(
            label(&artifact.label, SMALL, p.ink)
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(label(word, META, p.faint))
}

/// One report question card styled after KSG Code's `ComposerRequestCard`. `read_only`
/// draws the captain-recipient form: the same claim, question and evidence, with only
/// "Open thread" to act on.
fn card(s: &State, p: Palette, view: &CrewQuestionView, read_only: bool, pager: Option<El>) -> El {
    let status = outcome_status(view.outcome);
    let (secondary, _) = quiet_inks(p);
    let rid = view.report_id.clone();
    let server = s.machines.selected.clone();
    let captain_name = s
        .captains
        .get(view.captain)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let machine = s.machine_label(&server);
    let mut head = row()
        .min_h(32.0)
        .pl(10.0)
        .pr(13.0)
        .center_y()
        .gap(8.0)
        .child(crew_ui::mark_slot(status, p))
        .child(meta_text("Question from", status_ink(status, p)))
        .child(
            label(&view.title, TextStyle::new(13.0, 500.0), p.ink)
                .truncate()
                .shrink()
                .min_w(0.0),
        )
        .child(spacer())
        .child(meta_text(outcome_word(view.outcome), status_ink(status, p)));
    if let Some(pager) = pager {
        head = head.child(pager);
    }
    let mut detail = col().gap(8.0);
    let question = truncate(&view.question, 2000);
    detail = detail.child(
        col()
            .max_h(160.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                memo(
                    format!("needs-you-report-md-{rid}"),
                    (question.clone(), s.dark),
                    move || {
                        let style = MarkdownStyle::neutral(s.dark)
                            .text(secondary)
                            .muted(p.muted)
                            .body_font(BODY)
                            .inline_code_bg(p.ink.alpha(if s.dark { 0.07 } else { 0.055 }))
                            .selection(p.coral_bg)
                            .code_block(p.card, p.line);
                        Markdown::new(&question).render(&style)
                    },
                )
                .key(format!("needs-you-report-q-{rid}")),
            ),
    );
    if !view.still_in_crew {
        detail = detail.child(
            label(
                format!("No longer in {captain_name}'s crew."),
                META,
                p.muted,
            )
            .key(format!("needs-you-report-{rid}-released")),
        );
    }
    if !view.artifacts.is_empty() {
        let mut list = col().gap(4.0);
        for artifact in &view.artifacts {
            list = list.child(artifact_row(p, artifact));
        }
        detail = detail.child(
            col()
                .key(format!("needs-you-report-{rid}-evidence"))
                .gap(6.0)
                .child(list)
                .child(
                    label(
                        format!(
                            "These are claims by {} on {machine}. Shina does not open them.",
                            view.title
                        ),
                        META,
                        p.faint,
                    )
                    .wrap(),
                ),
        );
    }
    let mut body = col()
        .gap(6.0)
        .child(super::request_card_well(p, false, detail));
    let mut actions = row().gap(6.0).flex_wrap().center_y();
    let thread_id = view.thread_id;
    if !read_only {
        let blank;
        let draft = s.crew_question_draft(&server, &view.report_id);
        let edit = match draft {
            Some(draft) => &draft.text,
            None => {
                blank = rune::TextEdit::default();
                &blank
            }
        };
        let pending = draft.is_some_and(|draft| draft.pending.is_some());
        let empty = edit.text.trim().is_empty();
        let blocker = s.crew_question_blocker(view.captain);
        let expected_revision = view.expected_revision;
        let edit_key = (server.clone(), rid.clone());
        let submit_key = (server.clone(), rid.clone());
        body = body
            .child(
                col().pl(10.0).pr(13.0).w_full().min_w(0.0).child(
                    text_input(edit, BODY)
                        .key(format!("needs-you-report-{rid}-text"))
                        .label(format!("Your answer to {}", view.title))
                        .placeholder("Write an answer", p.muted)
                        .color(p.ink)
                        .w_full()
                        .min_h(30.0)
                        .max_h(120.0)
                        .px(10.0)
                        .py(6.0)
                        .radius(8.0)
                        .fill(p.card)
                        .field_focus(edit, p)
                        .disabled(pending)
                        .on_edit(move |s: &mut State| {
                            s.crew_question_draft_edit(&edit_key.0, &edit_key.1, expected_revision)
                        })
                        .on_submit(move |s: &mut State, _| {
                            let _ = s.submit_crew_question_answer(&submit_key.0, &submit_key.1);
                        }),
                ),
            )
            .child(
                label(
                    format!(
                        "Your answer goes to {} and {} sees it in the report.",
                        view.title, captain_name
                    ),
                    META,
                    p.muted,
                )
                .pl(10.0)
                .pr(13.0)
                .wrap(),
            );
        let disabled = pending || blocker.is_some() || empty;
        let send_key = (server.clone(), rid.clone());
        actions = actions.child(
            crew_ui::solid_button(
                format!("needs-you-report-{rid}-answer"),
                if pending { "Sending" } else { "Answer" },
                p,
                move |s: &mut State| {
                    let _ = s.submit_crew_question_answer(&send_key.0, &send_key.1);
                },
            )
            .disabled(disabled)
            .help(if pending {
                format!("Waiting for {machine} to confirm.")
            } else if let Some(reason) = &blocker {
                reason.clone()
            } else if empty {
                "Write an answer first.".to_string()
            } else {
                "Send the answer.".to_string()
            }),
        );
        if let Some(reason) = blocker {
            body = body.child(
                label(reason, SMALL, p.muted)
                    .key(format!("needs-you-report-{rid}-blocked"))
                    .pl(10.0)
                    .pr(13.0)
                    .wrap(),
            );
        }
        if let Some(error) = draft.and_then(|draft| draft.error.as_deref()) {
            body = body.child(
                label(error, SMALL, p.danger)
                    .key(format!("needs-you-report-{rid}-error"))
                    .role("alert")
                    .pl(10.0)
                    .pr(13.0)
                    .wrap(),
            );
        }
    }
    actions = actions.child(crew_ui::ghost_button(
        s,
        format!("needs-you-report-{rid}-open"),
        "Open thread",
        p,
        move |s| open_report_thread(s, thread_id),
    ));
    let age = timer_label(lifecycle::now().saturating_sub(view.created_at) / 1000);
    let delivery = delivery_words(view.delivered, view.policy, &captain_name);
    let mut footer = row()
        .key(format!("needs-you-report-{rid}-footer"))
        .pl(10.0)
        .pr(13.0)
        .pb(4.0)
        .gap(8.0)
        .center_y()
        .child(label(delivery.clone(), META, p.faint))
        .child(meta_text(age, p.faint));
    if let Some(badge) = policy_badge(format!("needs-you-report-{rid}-tag"), view.policy, p) {
        footer = footer.child(badge);
    }
    body = body
        .child(super::request_card_footer(actions))
        .child(footer);
    let mut root = col()
        .key(format!("needs-you-report-{rid}"))
        .role("group")
        .label(format!(
            "Question from {}, {}, {}",
            view.title,
            outcome_word(view.outcome).to_lowercase(),
            delivery.to_lowercase()
        ))
        .padding(4.0)
        .gap(4.0)
        .transition(Transition::offset(0.0, 6.0).combined(Transition::opacity()));
    if read_only {
        root = root.radius(8.0).fill(p.code_bg).border(1.0, p.line);
    }
    root.child(
        el().key(format!("needs-you-report-bar-{rid}"))
            .pin(Some(0.0), Some(6.0), None, Some(6.0))
            .w(2.0)
            .fill(p.brass)
            .a11y_hidden(),
    )
    .child(head)
    .child(body)
}

/// Whether `cards` draws, so the composer's stack budget can price it before building.
pub(super) fn cards_shows(s: &State) -> bool {
    let t = s.current();
    s.is_captain_home(t)
        && t.captain.is_some_and(|captain| {
            s.captains.get(captain).is_some()
                && (!s
                    .open_crew_questions(captain, QuestionRecipient::User)
                    .is_empty()
                    || !s
                        .open_crew_questions(captain, QuestionRecipient::Captain)
                        .is_empty())
        })
}
/// The cards and the collapsed captain-questions line for the current captain
/// home, mounted inside `crew_ui::needs_you` after its live request cards.
/// `cards` returns None when nothing is open, and no report question ever counts
/// the captain row's "Needs you" mark by itself — that comes from live requests.
pub(super) fn user_question_count(s: &State) -> usize {
    let t = s.current();
    if !s.is_captain_home(t) {
        return 0;
    }
    t.captain
        .filter(|captain| s.captains.get(*captain).is_some())
        .map(|captain| {
            s.open_crew_questions(captain, QuestionRecipient::User)
                .len()
        })
        .unwrap_or(0)
}

/// One paged user report question card (indexed into `open_crew_questions(captain, User)`).
pub(super) fn user_question_card(
    s: &State,
    p: Palette,
    index: usize,
    pager: Option<El>,
) -> Option<El> {
    let t = s.current();
    if !s.is_captain_home(t) {
        return None;
    }
    let captain = t.captain?;
    let user = s.open_crew_questions(captain, QuestionRecipient::User);
    let view = user.get(index)?;
    Some(
        col()
            .key("needs-you-reports")
            .role("group")
            .label("Crew report questions")
            .child(card(s, p, view, false, pager)),
    )
}

/// The collapsed captain-questions line (when crew questions are waiting on the captain).
pub(super) fn captain_questions_line(s: &State, p: Palette) -> Option<El> {
    let t = s.current();
    if !s.is_captain_home(t) {
        return None;
    }
    let captain = t.captain?;
    let profile = s.captains.get(captain)?;
    let crew = s.open_crew_questions(captain, QuestionRecipient::Captain);
    if crew.is_empty() {
        return None;
    }
    let mut stack = col().gap(8.0);
    let name = profile.name.clone();
    let count = crew.len();
    let open = s.crew_questions_captain_open;
    stack = stack.child(
        row()
            .key("needs-you-captain-line")
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(format!("{count} questions waiting on {name}"))
            .focusable(true)
            .center_y()
            .gap(6.0)
            .h(32.0)
            .px(8.0)
            .radius(6.0)
            .hover(crew_ui::hover_wash(s, p))
            .press(crew_ui::active_wash(s, p))
            .hover_motion(crew_ui::motion(), crew_ui::motion())
            .on_press(|s: &mut State| {
                s.crew_questions_captain_open = !s.crew_questions_captain_open;
            })
            .child(icon(if open { "chevron-up" } else { "chevron" }, p.muted))
            .child(meta_text(
                format!("{count} questions waiting on {name}"),
                p.muted,
            )),
    );
    if open {
        for view in &crew {
            stack = stack.child(card(s, p, view, true, None));
        }
    }
    Some(
        stack
            .key("needs-you-reports-captain")
            .role("group")
            .label("Crew report questions"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::press;
    use rune::{Env, Runtime};
    use serde_json::json;

    fn crew_runtime(width: f32) -> Runtime<State> {
        // A tall viewport: `press` clicks real coordinates, and a narrow card
        // stacks past 720 px where the composer would swallow the tap.
        let mut rt = Runtime::with_text(
            State::new(".".into(), false),
            view,
            (width, 1400.0),
            Env::new(false, 1.0),
            rune::text::TextSystem::new(FONTS, (SANS_FAMILY, "Cascadia Mono, Consolas")),
        );
        rt.set_tab_visits_controls(true);
        rt.state.data_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        rt.state.load_global_for_test();
        crate::captain_workspace_demo(&mut rt.state);
        // Reports must flush, which a demo session refuses; the workspace shape
        // is the demo's, but the stores are real.
        rt.state.demo = false;
        rt.invalidate();
        rt.frame(0.0);
        rt
    }

    fn question_report(rt: &mut Runtime<State>, recipient: &str) -> String {
        let captain = rt.state.current().captain.unwrap();
        let id = rt.state.next_id;
        rt.state.next_id += 1;
        let mut worker = Thread::new(id);
        worker.title = "Question worker".into();
        worker.crew_captain = Some(captain);
        worker.captain = Some(captain);
        worker.run_id = Some("run-q".into());
        rt.state.threads.push(worker);
        rt.state
            .mcp_call(
                id,
                "captain_crew_report",
                &json!({
                    "requestId":uuid::Uuid::new_v4().to_string(),
                    "outcome":"blocked","summary":"waiting on a call",
                    "question":{"text":"Which branch should the fix land on?","recipient":recipient},
                    "artifacts":[{"kind":"file","label":"bench.txt","reference":"benches/bench.txt"}]
                }),
            )
            .unwrap()["reportId"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn a_user_question_card_lays_out_at_supported_widths() {
        for width in [620.0, 1280.0] {
            let mut rt = crew_runtime(width);
            let rid = question_report(&mut rt, "user");
            rt.invalidate();
            rt.frame(1.0);
            // The demo has one live approval on page 0; advance the unified pager to the
            // report question card on page 1.
            press(&mut rt, "needs-you-next");
            rt.frame(1.0);
            for key in [
                format!("needs-you-report-{rid}"),
                format!("needs-you-report-bar-{rid}"),
                format!("needs-you-report-q-{rid}"),
                format!("needs-you-report-{rid}-evidence"),
                format!("needs-you-report-{rid}-text"),
                format!("needs-you-report-{rid}-answer"),
                format!("needs-you-report-{rid}-open"),
                format!("needs-you-report-{rid}-footer"),
            ] {
                let (rect, _) = rt
                    .presented(&key)
                    .unwrap_or_else(|| panic!("{width}: {key} is missing"));
                assert!(
                    rect[0] >= 0.0 && rect[0] + rect[2] <= width + 1.0 && rect[2] > 0.0,
                    "{width}: {key} {rect:?}"
                );
            }
            // Answering through the card's own seam retires the card only once the
            // answer record exists.
            rt.state
                .crew_question_draft_edit("local", &rid, 0)
                .replace_text("main");
            rt.invalidate();
            rt.frame(1.0);
            // The stack lives in `needs-you`'s 320 pt scroll clip: the Answer
            // button only takes a press once it is scrolled into that viewport.
            let answer = format!("needs-you-report-{rid}-answer");
            rt.update(|_, tx| tx.scroll_into_view_in("needs-you", answer.clone()));
            rt.frame(0.0);
            let clip = rt.presented("needs-you").unwrap().0;
            let (rect, _) = rt.presented(&answer).unwrap();
            assert!(
                rect[1] >= clip[1] && rect[1] + rect[3] <= clip[1] + clip[3] + 1.0,
                "{width}: answer {rect:?} outside needs-you clip {clip:?}"
            );
            press(&mut rt, &answer);
            rt.frame(1.0);
            assert_eq!(rt.state.crew_answers().len(), 1, "{width}");
            assert!(rt.presented(&format!("needs-you-report-{rid}")).is_none());
        }
    }

    #[test]
    fn captain_questions_stay_collapsed_behind_a_count_line() {
        for width in [620.0, 1280.0] {
            let mut rt = crew_runtime(width);
            let rid = question_report(&mut rt, "captain");
            rt.invalidate();
            rt.frame(1.0);
            assert!(rt.presented(&format!("needs-you-report-{rid}")).is_none());
            let (rect, _) = rt
                .presented("needs-you-captain-line")
                .unwrap_or_else(|| panic!("{width}: count line is missing"));
            assert!(
                rect[0] >= 0.0 && rect[0] + rect[2] <= width + 1.0,
                "{width}"
            );
            press(&mut rt, "needs-you-captain-line");
            rt.frame(1.0);
            assert!(rt.presented(&format!("needs-you-report-{rid}")).is_some());
            assert!(
                rt.presented(&format!("needs-you-report-{rid}-open"))
                    .is_some()
            );
            // The read-only form has no answer field or Answer button.
            assert!(
                rt.presented(&format!("needs-you-report-{rid}-text"))
                    .is_none()
            );
            assert!(
                rt.presented(&format!("needs-you-report-{rid}-answer"))
                    .is_none()
            );
        }
    }

    #[test]
    fn delivery_words_says_only_what_the_record_proves() {
        for policy in [
            ReportPolicy::React,
            ReportPolicy::Context,
            ReportPolicy::Ignore,
        ] {
            assert_eq!(delivery_words(true, policy, "Shina"), "Delivered to Shina");
        }
        assert_eq!(
            delivery_words(false, ReportPolicy::React, "Shina"),
            "Waiting to reach Shina"
        );
        assert_eq!(
            delivery_words(false, ReportPolicy::Context, "Shina"),
            "Waiting to reach Shina"
        );
        assert_eq!(
            delivery_words(false, ReportPolicy::Ignore, "Shina"),
            "Not sent to Shina"
        );
    }

    /// A report under `policy`, so the card reads the snapshot, not the
    /// Captain's current setting.
    fn policy_report(rt: &mut Runtime<State>, policy: ReportPolicy) -> String {
        let captain = rt.state.current().captain.unwrap();
        let current = rt.state.captain_report_policy(captain);
        rt.state
            .set_captain_report_policy(captain, policy, current)
            .unwrap();
        question_report(rt, "user")
    }

    #[test]
    fn the_card_footer_speaks_only_what_the_record_proves() {
        let mut rt = crew_runtime(960.0);
        // A durable batch so `set_captain_report_policy` can write.
        let database = crate::database::Database::open(&rt.state.data_dir).unwrap();
        database.begin().unwrap();
        let captain = rt.state.current().captain.unwrap();
        let name = rt.state.captains[captain].name.clone();
        let context = policy_report(&mut rt, ReportPolicy::Context);
        let ignore = policy_report(&mut rt, ReportPolicy::Ignore);
        rt.invalidate();
        rt.frame(1.0);
        // Context waits and keeps its question visible; Ignore never sends and
        // still keeps the question. Both carry the snapshot tag.
        for (page, (rid, words)) in [
            (
                &context,
                format!("waiting to reach {}", name.to_lowercase()),
            ),
            (&ignore, format!("not sent to {}", name.to_lowercase())),
        ]
        .into_iter()
        .enumerate()
        {
            // Page 0 is the demo's live approval; pages 1 and 2 hold the two report cards.
            rt.state.needs_you_page = page + 1;
            rt.invalidate();
            rt.frame(1.0);
            let card = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some(&format!("needs-you-report-{rid}")))
                .expect("the card stays");
            let label = card.label.expect("the card labels itself");
            assert!(label.contains(&words), "{rid}: {label}");
            assert!(
                rt.presented(&format!("needs-you-report-{rid}-tag"))
                    .is_some(),
                "{rid}: its snapshot tag stays while the Captain runs Ignore"
            );
        }
        assert!(
            rt.presented(&format!("needs-you-report-{ignore}-text"))
                .is_some(),
            "Ignore never hides a question for the user"
        );
    }
}
