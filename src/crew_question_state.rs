//! Crew question answer drafts and their pending remote sends. A draft is keyed by
//! the machine and report its card was rendered from, keeps the user's text while
//! an answer waits for its exact `captain.report.answer` receipt or was refused,
//! and carries one stable request id so a retry is idempotent on the backend.
use super::crew_reports::{CrewArtifact, CrewOutcome, QuestionRecipient, recruited_root};
use super::*;
use rune::TextEdit;

/// One open crew question as a needs-you card reads it: the report's claim and
/// delivery, the question text, its bounded artifact claims, and the revision the
/// next answer must name. Reading it settles nothing: delivery, the answer record
/// and the source thread's acceptance stay separate facts.
pub struct CrewQuestionView {
    pub report_id: String,
    pub captain: usize,
    pub thread_id: usize,
    pub title: String,
    pub outcome: CrewOutcome,
    pub created_at: u64,
    pub delivered: bool,
    /// The report's captured policy, not the Captain's current setting.
    pub policy: ReportPolicy,
    pub question: String,
    pub artifacts: Vec<CrewArtifact>,
    /// `expectedRevision` for the next answer: 0 while the question is open.
    pub expected_revision: u64,
    /// Whether the source thread is still a recruited root of this captain.
    pub still_in_crew: bool,
}

/// The typed-but-unsent answer to one crew question on one machine.
pub struct CrewQuestionDraft {
    pub text: TextEdit,
    /// The exact `captain.report.answer` command id awaiting its receipt.
    pub pending: Option<uuid::Uuid>,
    pub error: Option<String>,
    pub expected_revision: u64,
    /// Stable idempotency key: every send of this draft names the same UUID.
    pub request_id: String,
}

impl State {
    /// Open questions on `captain`'s reports for one recipient. A question is open
    /// while no answer record names its report; delivery state never decides this.
    pub fn open_crew_questions(
        &self,
        captain: usize,
        recipient: QuestionRecipient,
    ) -> Vec<CrewQuestionView> {
        self.captain_reports
            .iter()
            .filter(|report| report.captain == captain)
            .filter(|report| {
                report
                    .question
                    .as_ref()
                    .is_some_and(|question| question.recipient == recipient)
            })
            .filter(|report| {
                !self
                    .captain_question_answers
                    .iter()
                    .any(|answer| answer.report_id == report.id)
            })
            .map(|report| CrewQuestionView {
                report_id: report.id.clone(),
                captain,
                thread_id: report.thread_id,
                title: report.title.clone(),
                outcome: report.outcome,
                created_at: report.created_at,
                delivered: report.delivered,
                policy: report.policy,
                question: report
                    .question
                    .as_ref()
                    .map(|question| question.text.clone())
                    .unwrap_or_default(),
                artifacts: report.artifacts.clone(),
                expected_revision: 0,
                still_in_crew: self
                    .threads
                    .iter()
                    .find(|thread| thread.id == report.thread_id)
                    .is_some_and(|thread| recruited_root(thread, captain)),
            })
            .collect()
    }

    pub fn crew_question_draft(&self, server: &str, report_id: &str) -> Option<&CrewQuestionDraft> {
        self.crew_question_drafts
            .get(&(server.to_string(), report_id.to_string()))
    }

    /// The answer field's editor. The draft, and with it the stable request id,
    /// is created on the first edit so rendering never mutates state.
    pub(crate) fn crew_question_draft_edit(
        &mut self,
        server: &str,
        report_id: &str,
        expected_revision: u64,
    ) -> &mut TextEdit {
        &mut self
            .crew_question_drafts
            .entry((server.to_string(), report_id.to_string()))
            .or_insert_with(|| CrewQuestionDraft {
                text: TextEdit::default(),
                pending: None,
                error: None,
                expected_revision,
                request_id: uuid::Uuid::new_v4().to_string(),
            })
            .text
    }

    /// Why a card's answer controls are disabled on the selected machine, if they
    /// are. Mirrors the gates `captain.report.answer` enforces so the card can say
    /// why before the user types.
    pub fn crew_question_blocker(&self, captain: usize) -> Option<String> {
        match self.captain_access(captain) {
            CaptainAccess::Full => {}
            CaptainAccess::Earlier => {
                return Some("Read only: an earlier conversation is open.".into());
            }
            CaptainAccess::UpdateRequired => {
                return Some(crate::protocol::CAPTAIN_STATE_UPDATE_REQUIRED.into());
            }
            CaptainAccess::Unreachable => {
                return Some(format!(
                    "{} is unreachable.",
                    self.machine_label(&self.machines.selected)
                ));
            }
            CaptainAccess::NoRecords => return Some("No captain records.".into()),
        }
        self.require_captain_reports_capability().err()
    }

    /// The Answer button's seam. `expected_server` must be the machine the card
    /// rendered for; on a linked machine the draft records the pending command id
    /// and only its exact receipt settles it. A local save clears the draft; a
    /// refusal keeps the typed text and shows the message.
    pub fn submit_crew_question_answer(
        &mut self,
        expected_server: &str,
        report_id: &str,
    ) -> Result<(), String> {
        if expected_server != self.machines.selected {
            return Err("That answer belonged to another machine.".into());
        }
        let report = self
            .captain_reports
            .iter()
            .find(|report| report.id == report_id)
            .ok_or("Unknown report.")?;
        let captain = report.captain;
        if report
            .question
            .as_ref()
            .is_none_or(|question| question.recipient != QuestionRecipient::User)
        {
            return Err("That question is not addressed to you.".into());
        }
        // Press-time access check: the open thread must be this captain's current
        // primary home, not merely some thread with Full access, so a stale
        // callback from an earlier conversation or another thread cannot send.
        let current = self.current();
        let own_primary = current.captain_home
            && !current.delegated
            && !current.archived()
            && current.captain == Some(captain)
            && self.captain_home(captain) == Some(current.id);
        if !own_primary {
            return Err(
                "Only this captain's current conversation answers its crew questions.".into(),
            );
        }
        if let Some(reason) = self.crew_question_blocker(captain) {
            return Err(reason);
        }
        let key = (expected_server.to_string(), report_id.to_string());
        let Some(draft) = self.crew_question_drafts.get(&key) else {
            return Err("Write an answer first.".into());
        };
        if draft.pending.is_some() {
            return Err("The answer is already waiting for a reply.".into());
        }
        if draft.text.text.trim().is_empty() {
            return Err("Write an answer first.".into());
        }
        let (answer, expected_revision, request_id) = (
            draft.text.text.clone(),
            draft.expected_revision,
            draft.request_id.clone(),
        );
        match self.captain_report_answer(
            expected_server,
            captain,
            report_id,
            &answer,
            expected_revision,
            &request_id,
        ) {
            Ok(value) if value["pending"] == true => {
                let pending = value["command"]
                    .as_str()
                    .and_then(|id| uuid::Uuid::parse_str(id).ok());
                let draft = self.crew_question_drafts.get_mut(&key).expect("draft");
                draft.pending = pending;
                draft.error = None;
                Ok(())
            }
            Ok(_) => {
                self.crew_question_drafts.remove(&key);
                Ok(())
            }
            Err(error) => {
                let draft = self.crew_question_drafts.get_mut(&key).expect("draft");
                draft.error = Some(error.clone());
                // A definitive refusal ends this send intent: the next press is a
                // fresh request id, so an edited answer can never collide with the
                // rejected command the server already hashed.
                draft.request_id = uuid::Uuid::new_v4().to_string();
                Err(error)
            }
        }
    }

    /// Matches a `captain.report.answer` receipt to the draft holding its exact
    /// command id on that server. Acceptance removes the draft; the card leaves
    /// only when the answer record appears in the projection. A rejection clears
    /// the pending id and keeps the typed answer with the backend's message.
    pub(crate) fn crew_question_receipt(
        &mut self,
        server: &str,
        command: &crate::protocol::Command,
        value: &Value,
    ) -> bool {
        if command.method != "captain.report.answer" {
            return false;
        }
        let key = self
            .crew_question_drafts
            .iter()
            .find(|(key, draft)| key.0 == server && draft.pending == Some(command.command_id))
            .map(|(key, _)| key.clone());
        let Some(key) = key else {
            return false;
        };
        let accepted =
            value["result"]["status"] == "accepted" && value["result"]["result"]["saved"] == true;
        if accepted {
            self.crew_question_drafts.remove(&key);
            return true;
        }
        let message = value["error"]["message"]
            .as_str()
            .or_else(|| value["result"]["result"]["error"].as_str())
            .unwrap_or("The answer was not saved.")
            .to_string();
        let draft = self
            .crew_question_drafts
            .get_mut(&key)
            .expect("matched draft");
        draft.pending = None;
        draft.error = Some(message);
        // Same rotation as a synchronous refusal: editing the kept text sends a
        // new intent under a new request id.
        draft.request_id = uuid::Uuid::new_v4().to_string();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::crew_questions::AnswerActor;
    use serde_json::json;

    fn fixture() -> State {
        let mut state = State::new(
            std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()),
            false,
        );
        state.load_global_for_test();
        state.captains = Captain::presets();
        state.captain_reports.clear();
        state.captain_question_answers.clear();
        state.captain_homes.clear();
        state
            .threads
            .retain(|t| t.crew_captain.is_none() && !t.captain_home);
        state.active = 0;
        state
            .tabs
            .retain(|id| state.threads.iter().any(|t| t.id == *id));
        state.open_captain(0);
        state
    }

    fn crew_thread(state: &mut State) -> usize {
        let id = state.next_id;
        state.next_id += 1;
        let mut thread = Thread::new(id);
        thread.crew_captain = Some(0);
        thread.captain = Some(0);
        thread.title = "Crew worker".into();
        thread.run_id = Some("run-x".into());
        state.threads.push(thread);
        id
    }

    fn question_report(state: &mut State, worker: usize, recipient: &str) -> String {
        state
            .mcp_call(
                worker,
                "captain_crew_report",
                &json!({
                    "requestId":uuid::Uuid::new_v4().to_string(),
                    "outcome":"blocked","summary":"waiting",
                    "question":{"text":"Which branch?","recipient":recipient}
                }),
            )
            .unwrap()["reportId"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn receipt(saved: bool) -> Value {
        json!({"result":{"status":if saved {"accepted"} else {"rejected"},
            "result":{"saved":saved},"error":{}},"error":{"message":"nope"}})
    }

    #[test]
    fn open_questions_split_by_recipient_and_hide_answered() {
        let mut state = fixture();
        let worker = crew_thread(&mut state);
        let user = question_report(&mut state, worker, "user");
        let crew = question_report(&mut state, worker, "captain");
        assert_eq!(
            state
                .open_crew_questions(0, QuestionRecipient::User)
                .iter()
                .map(|view| view.report_id.as_str())
                .collect::<Vec<_>>(),
            [user.as_str()]
        );
        assert_eq!(
            state
                .open_crew_questions(0, QuestionRecipient::Captain)
                .iter()
                .map(|view| view.report_id.as_str())
                .collect::<Vec<_>>(),
            [crew.as_str()]
        );
        state
            .crew_question_draft_edit("local", &user, 0)
            .replace_text("main");
        state.submit_crew_question_answer("local", &user).unwrap();
        assert!(
            state
                .open_crew_questions(0, QuestionRecipient::User)
                .is_empty()
        );
        assert_eq!(state.crew_answers().len(), 1);
    }

    #[test]
    fn draft_is_created_on_first_edit_and_submits_with_its_stable_request_id() {
        let mut state = fixture();
        let worker = crew_thread(&mut state);
        let report = question_report(&mut state, worker, "user");
        assert!(state.crew_question_draft("local", &report).is_none());
        state
            .crew_question_draft_edit("local", &report, 0)
            .replace_text("main");
        let request_id = state
            .crew_question_draft("local", &report)
            .unwrap()
            .request_id
            .clone();
        assert!(uuid::Uuid::parse_str(&request_id).is_ok());
        state.submit_crew_question_answer("local", &report).unwrap();
        assert!(state.crew_question_draft("local", &report).is_none());
        let answer = &state.crew_answers()[0];
        assert_eq!(answer.request_id, request_id);
        assert_eq!(answer.answer, "main");
        assert_eq!(answer.actor, AnswerActor::User);
    }

    #[test]
    fn submit_requires_text_the_matching_server_and_a_user_recipient() {
        let mut state = fixture();
        let worker = crew_thread(&mut state);
        let report = question_report(&mut state, worker, "user");
        let crew = question_report(&mut state, worker, "captain");
        assert!(state.submit_crew_question_answer("local", &report).is_err());
        state.crew_question_draft_edit("local", &report, 0);
        assert!(state.submit_crew_question_answer("local", &report).is_err());
        assert!(
            state
                .submit_crew_question_answer("studio", &report)
                .is_err()
        );
        state
            .crew_question_draft_edit("local", &crew, 0)
            .replace_text("take it myself");
        assert_eq!(
            state
                .submit_crew_question_answer("local", &crew)
                .unwrap_err(),
            "That question is not addressed to you."
        );
        assert!(state.crew_answers().is_empty());
    }

    #[test]
    fn a_stale_callback_off_the_primary_home_cannot_send() {
        let mut state = fixture();
        let worker = crew_thread(&mut state);
        let report = question_report(&mut state, worker, "user");
        state
            .crew_question_draft_edit("local", &report, 0)
            .replace_text("main");
        // Full access on an unrelated thread is not enough: only the captain's
        // current primary home answers its crew questions.
        let ordinary = state
            .threads
            .iter()
            .position(|t| !t.captain_home && t.crew_captain.is_none())
            .expect("an ordinary thread");
        state.open_kept_tab(ordinary);
        assert_eq!(
            state
                .submit_crew_question_answer("local", &report)
                .unwrap_err(),
            "Only this captain's current conversation answers its crew questions."
        );
        assert!(state.crew_answers().is_empty());
        // The draft and its request id survive; returning to the home still sends.
        let home = state
            .threads
            .iter()
            .position(|t| t.id == state.captain_home(0).unwrap())
            .unwrap();
        state.open_kept_tab(home);
        state.submit_crew_question_answer("local", &report).unwrap();
        assert_eq!(state.crew_answers().len(), 1);
    }

    #[test]
    fn receipt_matches_only_exact_server_id_and_method() {
        let mut state = fixture();
        let worker = crew_thread(&mut state);
        let report = question_report(&mut state, worker, "user");
        state
            .crew_question_draft_edit("local", &report, 0)
            .replace_text("main");
        let pending = uuid::Uuid::new_v4();
        let draft = state
            .crew_question_drafts
            .get_mut(&("local".to_string(), report.clone()))
            .unwrap();
        draft.pending = Some(pending);
        let command = crate::protocol::Command {
            command_id: pending,
            method: "captain.report.answer".into(),
            params: Value::Null,
        };
        let other = crate::protocol::Command {
            command_id: uuid::Uuid::new_v4(),
            method: "captain.report.answer".into(),
            params: Value::Null,
        };
        let mut wrong_method = command.clone();
        wrong_method.method = "captain.crew.join".into();
        assert!(!state.crew_question_receipt("local", &other, &receipt(true)));
        assert!(!state.crew_question_receipt("studio", &command, &receipt(true)));
        assert!(!state.crew_question_receipt("local", &wrong_method, &receipt(true)));
        assert!(state.crew_question_draft("local", &report).is_some());
        let original = state
            .crew_question_draft("local", &report)
            .unwrap()
            .request_id
            .clone();
        // A rejection keeps the typed answer and shows the backend's message.
        assert!(state.crew_question_receipt("local", &command, &receipt(false)));
        let draft = state.crew_question_draft("local", &report).unwrap();
        assert_eq!(draft.text.text, "main");
        assert!(draft.pending.is_none());
        assert_eq!(draft.error.as_deref(), Some("nope"));
        // A definitive rejection rotates the request id: an edited retry is a new
        // intent and can never collide with the rejected command's hash.
        assert_ne!(draft.request_id, original);
        let command = crate::protocol::Command {
            command_id: uuid::Uuid::new_v4(),
            method: "captain.report.answer".into(),
            params: Value::Null,
        };
        state
            .crew_question_drafts
            .get_mut(&("local".to_string(), report.clone()))
            .unwrap()
            .pending = Some(command.command_id);
        assert!(state.crew_question_receipt("local", &command, &receipt(true)));
        assert!(state.crew_question_draft("local", &report).is_none());
    }

    #[test]
    fn a_rejection_from_the_backend_keeps_the_typed_answer() {
        let mut state = fixture();
        let worker = crew_thread(&mut state);
        let report = question_report(&mut state, worker, "user");
        state
            .crew_question_draft_edit("local", &report, 0)
            .replace_text("   ");
        // The UI gate trims first, so the backend's message never reaches it here;
        // a second report's stale revision exercises the refusal path instead.
        state
            .crew_question_drafts
            .get_mut(&("local".to_string(), report.clone()))
            .unwrap()
            .expected_revision = 7;
        state
            .crew_question_drafts
            .get_mut(&("local".to_string(), report.clone()))
            .unwrap()
            .text
            .replace_text("main");
        assert!(state.submit_crew_question_answer("local", &report).is_err());
        let draft = state.crew_question_draft("local", &report).unwrap();
        assert_eq!(draft.text.text, "main");
        assert!(draft.error.is_some());
        assert!(state.crew_answers().is_empty());
        // The refusal rotated the intent id; editing and resending at the current
        // revision lands under the new one, never the rejected command's id.
        let rotated = draft.request_id.clone();
        state
            .crew_question_drafts
            .get_mut(&("local".to_string(), report.clone()))
            .unwrap()
            .expected_revision = 0;
        state
            .crew_question_draft_edit("local", &report, 0)
            .replace_text("main, rebased");
        state.submit_crew_question_answer("local", &report).unwrap();
        assert_eq!(state.crew_answers()[0].request_id, rotated);
        assert_eq!(state.crew_answers()[0].answer, "main, rebased");
    }
}
