//! Source-backed Captain activity digest. Rebuilt on each read from the durable
//! crew report, answer, task, request and membership records; it never writes,
//! acknowledges deliveries, parses DIGEST.md, or wakes a process. Per-report
//! delivery policy never applies here: Ignore only suppresses delivery, and a
//! genuine human question always stays listed.
use super::crew_reports::{CrewOutcome, CrewReport, QuestionRecipient, recruited_root};
use super::*;
use serde::{Deserialize, Serialize};
use sha2::Digest;

pub const DIGEST_NEEDS_YOU_MAX: usize = 16;
pub const DIGEST_WHATS_NEW_MAX: usize = 32;
pub const DIGEST_IN_PROGRESS_MAX: usize = 16;
const TITLE_MAX_CHARS: usize = 200;
const SUMMARY_MAX_CHARS: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestEntryKind {
    CrewQuestion,
    ProviderApproval,
    ProviderQuestion,
    CrewReport,
    CrewThread,
    DelegatedTask,
}

/// One digest row: facts copied from its source record plus the evidence ids a
/// caller needs to open the source through the existing read APIs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestEntry {
    /// Stable per-entry id: the report's delivery id, the task's id, or
    /// `request-<threadId>` for a pending provider request.
    pub id: String,
    pub thread_id: usize,
    #[serde(default)]
    pub report_id: Option<String>,
    #[serde(default)]
    pub answer_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    pub kind: DigestEntryKind,
    pub status: String,
    pub title: String,
    pub summary: String,
    pub created_at: u64,
    /// Whether the source report or result reached the Captain's accepted
    /// history. Delivery never resolves a question.
    #[serde(default)]
    pub report_delivered: Option<bool>,
    /// Whether the report's question has a saved answer.
    #[serde(default)]
    pub answer_saved: bool,
    /// Whether that answer was accepted at its source thread. Answer
    /// acceptance is a separate fact from the answer existing.
    #[serde(default)]
    pub answer_accepted: bool,
    /// The report's captured wake policy (`react`, `context`, `ignore`) so a
    /// caller can tell a suppressed never-deliver report from one still on its
    /// way. `None` for entries that are not report-derived.
    #[serde(default)]
    pub report_policy: Option<String>,
}

/// How many source records each section dropped past its fixed bound.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestTruncation {
    pub needs_you: usize,
    pub whats_new: usize,
    pub in_progress: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptainActivityDigest {
    pub captain: usize,
    /// Owning server id, from this State's selected machine. Entries hold no
    /// local paths; the caller pairs them with this server.
    pub server: String,
    pub home_thread_id: Option<usize>,
    /// Content version hashed from the canonical section fields and truncation
    /// counts. Same records, same version; wallclock is not an input.
    pub version: String,
    /// DIGEST.md's saved revision so a caller can keep showing the manual
    /// digest preview and notice when it changes. This digest never reads its
    /// text as structure and never writes it.
    pub manual_revision: u64,
    pub needs_you: Vec<DigestEntry>,
    pub whats_new: Vec<DigestEntry>,
    pub in_progress: Vec<DigestEntry>,
    pub truncated: DigestTruncation,
}

fn bounded(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn outcome_id(outcome: CrewOutcome) -> &'static str {
    match outcome {
        CrewOutcome::Completed => "completed",
        CrewOutcome::Failed => "failed",
        CrewOutcome::Cancelled => "cancelled",
        CrewOutcome::Progress => "progress",
        CrewOutcome::Blocked => "blocked",
    }
}

impl State {
    /// Builds the automatic activity digest for `captain` from durable records.
    /// Read-only: no checkpoint, no injection, no acceptance marks, no wakes.
    pub fn captain_activity_digest(&self, captain: usize) -> Result<CaptainActivityDigest, String> {
        self.require_captain(captain)?;
        let home = self.captain_home(captain);
        let answer_for = |report: &CrewReport| {
            self.captain_question_answers
                .iter()
                .find(|answer| answer.report_id == report.id)
        };
        let mut needs_you = Vec::new();
        let mut whats_new = Vec::new();
        let mut in_progress = Vec::new();
        let mut truncated = DigestTruncation::default();

        // Open user-recipient questions stay listed until the answer itself is
        // accepted at the worker; a delivered report, a delivery hold or an
        // Ignore report policy never hides a genuine human question.
        for report in self
            .captain_reports
            .iter()
            .filter(|report| report.captain == captain)
            .filter(|report| {
                report
                    .question
                    .as_ref()
                    .is_some_and(|q| q.recipient == QuestionRecipient::User)
            })
        {
            let answer = answer_for(report);
            if answer.is_some_and(|answer| answer.delivered) {
                continue;
            }
            needs_you.push(DigestEntry {
                id: report.id.clone(),
                thread_id: report.thread_id,
                report_id: Some(report.id.clone()),
                answer_id: answer.map(|a| a.id.clone()),
                task_id: None,
                kind: DigestEntryKind::CrewQuestion,
                status: if answer.is_some() {
                    "answer_pending".into()
                } else {
                    "open".into()
                },
                title: bounded(&report.title, TITLE_MAX_CHARS),
                summary: bounded(
                    &report
                        .question
                        .as_ref()
                        .map(|q| q.text.as_str())
                        .unwrap_or(""),
                    SUMMARY_MAX_CHARS,
                ),
                created_at: report.created_at,
                report_delivered: Some(report.delivered),
                answer_saved: answer.is_some(),
                answer_accepted: answer.is_some_and(|a| a.delivered),
                report_policy: Some(report.policy.id().into()),
            });
        }

        // Actionable provider requests routed to the current home by canonical
        // membership, falling back to the parent's effective home. Captain-
        // recipient report questions are never user requests and stay out. A
        // stale nonresumable projection or a request the provider contract
        // cannot answer is not an actionable item.
        if let Some(home) = home {
            for thread in self.threads.iter().filter(|thread| {
                !thread.archived()
                    && thread.requests_supported
                    && thread.responding_request.is_none()
                    && thread
                        .request
                        .as_ref()
                        .is_some_and(crate::approval::can_respond)
            }) {
                let routed = if self.captain_provenanced(thread) {
                    self.captain_member_at(thread, home)
                } else {
                    thread.parent.map(|id| self.effective_home(id)) == Some(home)
                };
                if !routed {
                    continue;
                }
                let request = thread.request.as_ref().expect("request");
                let approval = request["method"] == "confirm";
                // Generation plus the provider's own request id keeps
                // consecutive requests on one thread distinct and stable.
                needs_you.push(DigestEntry {
                    id: format!(
                        "request-{}-{}-{}",
                        thread.id, thread.request_generation, request["id"]
                    ),
                    thread_id: thread.id,
                    report_id: None,
                    answer_id: None,
                    task_id: self
                        .tasks
                        .iter()
                        .find(|task| task.child_thread_id == thread.id)
                        .map(|task| task.id.clone()),
                    kind: if approval {
                        DigestEntryKind::ProviderApproval
                    } else {
                        DigestEntryKind::ProviderQuestion
                    },
                    status: "open".into(),
                    title: bounded(&thread.title, TITLE_MAX_CHARS),
                    summary: bounded(
                        &[string(request, "title"), string(request, "message")]
                            .into_iter()
                            .filter(|part| !part.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        SUMMARY_MAX_CHARS,
                    ),
                    created_at: 0,
                    report_delivered: None,
                    answer_saved: false,
                    answer_accepted: false,
                    report_policy: None,
                });
            }
            // Linked-machine workers expose the same kind of request through
            // their remote task record; the same nonresumable gate applies.
            for task in self
                .tasks
                .iter()
                .filter(|task| self.task_home(task) == Some(home))
            {
                let Some(remote) = &task.remote else { continue };
                if task.state.finished() || (!remote.needs_you && remote.request.is_none()) {
                    continue;
                }
                if remote
                    .request
                    .as_ref()
                    .is_some_and(|request| !crate::approval::can_respond(request))
                {
                    continue;
                }
                let approval = remote.request.as_ref().is_some_and(|request| {
                    matches!(request["kind"].as_str(), Some("confirm"))
                        || request["method"] == "confirm"
                });
                needs_you.push(DigestEntry {
                    id: format!(
                        "{}-{}",
                        task.id,
                        remote
                            .request
                            .as_ref()
                            .map(|request| request["id"].to_string())
                            .unwrap_or_default()
                    ),
                    thread_id: task.child_thread_id,
                    report_id: None,
                    answer_id: None,
                    task_id: Some(task.id.clone()),
                    kind: if approval {
                        DigestEntryKind::ProviderApproval
                    } else {
                        DigestEntryKind::ProviderQuestion
                    },
                    status: "remote".into(),
                    title: bounded(&task.title, TITLE_MAX_CHARS),
                    summary: format!("Needs you on {}", remote.label),
                    created_at: task.created_at,
                    report_delivered: None,
                    answer_saved: false,
                    answer_accepted: false,
                    report_policy: None,
                });
            }
        }

        // Newest reports first, bounded. Ignore-policy reports stay in the
        // list; delivery and answer acceptance travel as separate facts so
        // nothing here claims work shipped.
        for report in self
            .captain_reports
            .iter()
            .rev()
            .filter(|report| report.captain == captain)
        {
            if whats_new.len() == DIGEST_WHATS_NEW_MAX {
                truncated.whats_new += 1;
                continue;
            }
            let answer = answer_for(report);
            whats_new.push(DigestEntry {
                id: report.id.clone(),
                thread_id: report.thread_id,
                report_id: Some(report.id.clone()),
                answer_id: answer.map(|a| a.id.clone()),
                task_id: None,
                kind: DigestEntryKind::CrewReport,
                status: outcome_id(report.outcome).into(),
                title: bounded(&report.title, TITLE_MAX_CHARS),
                summary: bounded(&report.summary, SUMMARY_MAX_CHARS),
                created_at: report.created_at,
                report_delivered: Some(report.delivered),
                answer_saved: report.question.is_some() && answer.is_some(),
                answer_accepted: answer.is_some_and(|a| a.delivered),
                report_policy: Some(report.policy.id().into()),
            });
        }

        // Independent crew members that are actually busy or queued, and
        // Task-owned children routed to the current home with an active task
        // state. Historical parent links never reopen moved or released work.
        if let Some(home) = home {
            for thread in self
                .threads
                .iter()
                .filter(|thread| !thread.archived() && recruited_root(thread, captain))
            {
                if !(thread.status.busy() || thread.configuring || thread.queued()) {
                    continue;
                }
                in_progress.push(DigestEntry {
                    id: format!("crew-{}", thread.id),
                    thread_id: thread.id,
                    report_id: None,
                    answer_id: None,
                    task_id: None,
                    kind: DigestEntryKind::CrewThread,
                    status: if thread.status.busy() {
                        status_word(thread.status).into()
                    } else {
                        "queued".into()
                    },
                    title: bounded(&thread.title, TITLE_MAX_CHARS),
                    summary: bounded(&thread.phase, SUMMARY_MAX_CHARS),
                    created_at: 0,
                    report_delivered: None,
                    answer_saved: false,
                    answer_accepted: false,
                    report_policy: None,
                });
            }
            for task in self.tasks.iter().filter(|task| {
                matches!(
                    task.state,
                    TaskState::Queued | TaskState::Starting | TaskState::Running
                ) && self.task_home(task) == Some(home)
            }) {
                if in_progress
                    .iter()
                    .any(|entry: &DigestEntry| entry.thread_id == task.child_thread_id)
                {
                    continue;
                }
                in_progress.push(DigestEntry {
                    id: task.id.clone(),
                    thread_id: task.child_thread_id,
                    report_id: None,
                    answer_id: None,
                    task_id: Some(task.id.clone()),
                    kind: DigestEntryKind::DelegatedTask,
                    status: task.state.id().into(),
                    title: bounded(&task.title, TITLE_MAX_CHARS),
                    summary: bounded(task.brief.lines().next().unwrap_or(""), SUMMARY_MAX_CHARS),
                    created_at: task.created_at,
                    report_delivered: None,
                    answer_saved: false,
                    answer_accepted: false,
                    report_policy: None,
                });
            }
        }

        truncated.needs_you = needs_you.len().saturating_sub(DIGEST_NEEDS_YOU_MAX);
        needs_you.truncate(DIGEST_NEEDS_YOU_MAX);
        truncated.in_progress = in_progress.len().saturating_sub(DIGEST_IN_PROGRESS_MAX);
        in_progress.truncate(DIGEST_IN_PROGRESS_MAX);

        let meta = self.captain_document_view(captain, "DIGEST.md");
        let canonical = serde_json::to_string(&json!({
            "needsYou":needs_you, "whatsNew":whats_new,
            "inProgress":in_progress, "truncated":truncated,
        }))
        .expect("digest serialization");
        let hash = format!("{:x}", sha2::Sha256::digest(canonical.as_bytes()));
        Ok(CaptainActivityDigest {
            captain,
            server: self.machines.selected.clone(),
            home_thread_id: home,
            version: format!("v1-{}", &hash[..16]),
            manual_revision: meta.revision,
            needs_you,
            whats_new,
            in_progress,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::crew_questions::{AnswerActor, CrewAnswer};

    fn fixture() -> State {
        // A test can build more than one fixture. Loading the per-test profile
        // here would let one fixture pick up another's asynchronous checkpoints.
        let mut state = State::new(std::path::PathBuf::new(), false);
        state.captains = Captain::presets();
        state
    }

    fn crew_thread(state: &mut State, captain: usize) -> usize {
        let id = state.next_id;
        state.next_id += 1;
        let mut thread = Thread::new(id);
        thread.crew_captain = Some(captain);
        thread.captain = Some(captain);
        thread.title = "Crew worker".into();
        state.threads.push(thread);
        id
    }

    fn report(
        state: &mut State,
        captain: usize,
        thread_id: usize,
        question: Option<crew_reports::CrewQuestion>,
        delivered: bool,
    ) -> String {
        let id = format!("crew-{}", uuid::Uuid::new_v4());
        state.captain_reports.push(CrewReport {
            id: id.clone(),
            captain,
            thread_id,
            run_id: format!("run-{thread_id}"),
            title: "Report".into(),
            outcome: CrewOutcome::Progress,
            policy: crate::model::ReportPolicy::React,
            summary: "Half done".into(),
            error: String::new(),
            created_at: 1,
            request_id: Some(uuid::Uuid::new_v4().to_string()),
            question,
            artifacts: Vec::new(),
            delivered,
            hold_delivery: false,
        });
        id
    }

    fn user_question() -> Option<crew_reports::CrewQuestion> {
        Some(crew_reports::CrewQuestion {
            text: "Ship it?".into(),
            recipient: QuestionRecipient::User,
        })
    }

    #[test]
    fn a_delivered_report_does_not_resolve_its_question() {
        let mut state = fixture();
        state.open_captain(0);
        let member = crew_thread(&mut state, 0);
        let id = report(&mut state, 0, member, user_question(), true);
        let digest = state.captain_activity_digest(0).unwrap();
        assert_eq!(digest.needs_you.len(), 1);
        let entry = &digest.needs_you[0];
        assert_eq!(entry.kind, DigestEntryKind::CrewQuestion);
        assert_eq!(entry.report_id.as_deref(), Some(id.as_str()));
        assert_eq!(entry.report_delivered, Some(true));
        assert!(!entry.answer_saved);
        assert!(!digest.whats_new[0].answer_saved);
    }

    #[test]
    fn ignore_policy_reports_stay_listed_and_questions_stay_open() {
        let mut state = fixture();
        state.open_captain(0);
        let member = crew_thread(&mut state, 0);
        let id = report(&mut state, 0, member, user_question(), false);
        state.captain_reports[0].policy = crate::model::ReportPolicy::Ignore;
        report(&mut state, 0, member, None, false);
        state.captain_reports[1].policy = crate::model::ReportPolicy::Ignore;
        let digest = state.captain_activity_digest(0).unwrap();
        assert_eq!(digest.needs_you.len(), 1);
        assert_eq!(digest.needs_you[0].report_id.as_deref(), Some(id.as_str()));
        assert_eq!(digest.whats_new.len(), 2);
    }

    #[test]
    fn answer_acceptance_is_a_separate_fact_from_answering() {
        let mut state = fixture();
        state.open_captain(0);
        let member = crew_thread(&mut state, 0);
        let id = report(&mut state, 0, member, user_question(), true);
        state.captain_question_answers.push(CrewAnswer {
            id: format!("crew-answer-{}", uuid::Uuid::new_v4()),
            report_id: id,
            captain: 0,
            thread_id: member,
            request_id: uuid::Uuid::new_v4().to_string(),
            answer: "Yes".into(),
            actor: AnswerActor::User,
            created_at: 2,
            delivered: false,
            hold_delivery: true,
        });
        let digest = state.captain_activity_digest(0).unwrap();
        assert_eq!(digest.needs_you[0].status, "answer_pending");
        assert!(digest.needs_you[0].answer_saved);
        assert!(!digest.needs_you[0].answer_accepted);
        state.captain_question_answers[0].delivered = true;
        let digest = state.captain_activity_digest(0).unwrap();
        assert!(digest.needs_you.is_empty());
        assert!(digest.whats_new[0].answer_saved);
        assert!(digest.whats_new[0].answer_accepted);
    }

    #[test]
    fn moved_crew_requests_reach_only_the_current_captain() {
        let mut state = fixture();
        state.open_captain(0);
        state.open_captain(1);
        let member = crew_thread(&mut state, 1);
        // A stale historical parent link must not reopen it at captain 0.
        let former = state.captain_home(0);
        let thread = state.threads.iter_mut().find(|t| t.id == member).unwrap();
        thread.parent = former;
        thread.requests_supported = true;
        thread.request =
            Some(json!({"id":"req-1","method":"confirm","title":"Allow?","message":""}));
        assert!(
            state
                .captain_activity_digest(0)
                .unwrap()
                .needs_you
                .is_empty()
        );
        let digest = state.captain_activity_digest(1).unwrap();
        assert_eq!(digest.needs_you[0].kind, DigestEntryKind::ProviderApproval);
        assert_eq!(digest.needs_you[0].thread_id, member);
    }

    #[test]
    fn nonresumable_and_unsupported_requests_are_not_actionable() {
        let mut state = fixture();
        state.open_captain(0);
        let member = crew_thread(&mut state, 0);
        let set = |state: &mut State, request: Value| {
            let thread = state.threads.iter_mut().find(|t| t.id == member).unwrap();
            thread.requests_supported = true;
            thread.request = Some(request);
        };
        set(
            &mut state,
            json!({"id":"req-1","method":"confirm","responseCapability":"not_resumable"}),
        );
        assert!(
            state
                .captain_activity_digest(0)
                .unwrap()
                .needs_you
                .is_empty()
        );
        set(
            &mut state,
            json!({"id":"req-2","method":"confirm","canRespond":false}),
        );
        assert!(
            state
                .captain_activity_digest(0)
                .unwrap()
                .needs_you
                .is_empty()
        );
        set(&mut state, json!({"id":"req-3","method":"confirm"}));
        let first = state.captain_activity_digest(0).unwrap();
        assert_eq!(first.needs_you.len(), 1);
        let thread = state.threads.iter_mut().find(|t| t.id == member).unwrap();
        thread.request_generation = "next-runtime".into();
        thread.request = Some(json!({"id":"req-4","method":"confirm"}));
        let second = state.captain_activity_digest(0).unwrap();
        assert_ne!(first.needs_you[0].id, second.needs_you[0].id);
        assert_ne!(first.version, second.version);
    }

    #[test]
    fn private_rules_and_profile_instructions_stay_out() {
        let mut state = fixture();
        state.open_captain(0);
        state.captains[0].instructions = "PRIVATE-RULE".into();
        state.seed_captain_document(
            0,
            "AGENTS.md",
            captain_state::CaptainDocument {
                text: "AGENTS-SECRET".into(),
                revision: 1,
                updated_at: 1,
            },
        );
        state.seed_captain_document(
            0,
            "DIGEST.md",
            captain_state::CaptainDocument {
                text: "Manual digest".into(),
                revision: 3,
                updated_at: 9,
            },
        );
        let digest = state.captain_activity_digest(0).unwrap();
        let text = serde_json::to_string(&digest).unwrap();
        assert!(!text.contains("PRIVATE-RULE") && !text.contains("AGENTS-SECRET"));
        assert!(!text.contains("Manual digest"));
        assert_eq!(digest.manual_revision, 3);
    }

    #[test]
    fn bounds_are_fixed_and_output_is_deterministic() {
        let mut state = fixture();
        state.open_captain(0);
        let member = crew_thread(&mut state, 0);
        for _ in 0..40 {
            report(&mut state, 0, member, None, true);
        }
        let a = state.captain_activity_digest(0).unwrap();
        let b = state.captain_activity_digest(0).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.whats_new.len(), DIGEST_WHATS_NEW_MAX);
        assert_eq!(a.truncated.whats_new, 8);
        assert_eq!(a.server, state.machines.selected);
        assert!(state.captain_activity_digest(99).is_err());
    }

    #[test]
    fn restored_records_rebuild_the_same_view() {
        let mut state = fixture();
        state.open_captain(0);
        let member = crew_thread(&mut state, 0);
        report(&mut state, 0, member, user_question(), true);
        report(&mut state, 0, member, None, true);
        let snapshot = json!({"captainReports":state.captain_reports});
        let before = state.captain_activity_digest(0).unwrap();

        let mut restored = fixture();
        restored.open_captain(0);
        let mut thread = Thread::new(member);
        thread.crew_captain = Some(0);
        thread.captain = Some(0);
        restored.threads.push(thread);
        restored.next_id = restored.next_id.max(member + 1);
        restored.restore_crew_reports(&snapshot).unwrap();
        let after = restored.captain_activity_digest(0).unwrap();
        assert_eq!(before.version, after.version);
        assert_eq!(after.needs_you.len(), 1);
        assert_eq!(after.whats_new.len(), 2);
    }
}
