//! Stable Captain identities and confirmation snapshots. Roster entries never
//! shift when a Captain is archived, demoted or deleted.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptainPhase {
    #[default]
    Active,
    Archived,
    Retired,
    Deleted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct CaptainLifecycle {
    pub version: u32,
    pub phase: CaptainPhase,
    #[serde(default)]
    pub archived_at: Option<u64>,
    #[serde(default)]
    pub retired_at: Option<u64>,
    #[serde(default)]
    pub former_home: Option<usize>,
    /// Exact pre-group state, including members already archived separately.
    #[serde(default)]
    pub archived_threads: BTreeMap<usize, Option<u64>>,
    /// Only enabled routines paused by archive may resume on unarchive.
    #[serde(default)]
    pub paused_routines: Vec<u64>,
}

impl Default for CaptainLifecycle {
    fn default() -> Self {
        Self {
            version: 1,
            phase: CaptainPhase::Active,
            archived_at: None,
            retired_at: None,
            former_home: None,
            archived_threads: BTreeMap::new(),
            paused_routines: Vec::new(),
        }
    }
}

impl State {
    fn captain_archive_notifications(
        &self,
        group: &BTreeSet<usize>,
    ) -> BTreeMap<usize, BTreeSet<String>> {
        let mut notifications: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
        for report in self.captain_reports.iter().chain(
            self.threads
                .iter()
                .flat_map(|thread| &thread.crew_report_outbox),
        ) {
            if let Some(home) = self.captain_home(report.captain)
                && group.contains(&home)
            {
                notifications
                    .entry(home)
                    .or_default()
                    .insert(report.message());
            }
        }
        for answer in &self.captain_question_answers {
            if group.contains(&answer.thread_id)
                && let Some(question) = self
                    .captain_reports
                    .iter()
                    .find(|report| report.id == answer.report_id)
                    .and_then(|report| report.question.as_ref())
            {
                notifications
                    .entry(answer.thread_id)
                    .or_default()
                    .insert(answer.message(&question.text));
            }
        }
        for task in &self.tasks {
            if let Some(home) = self.task_home(task)
                && group.contains(&home)
            {
                notifications
                    .entry(home)
                    .or_default()
                    .extend(task.result_notifications());
            }
        }
        notifications
    }

    fn captain_archive_blockers(&self, captain: usize) -> Vec<String> {
        let mut blockers = Vec::new();
        if self.captain_home(captain).is_none() {
            blockers.push("This Captain has no primary home.".into());
        }
        if let Err(error) = self.background.routines.captain_idle(captain) {
            blockers.push(error);
        }
        let group = self.captain_group_ids(captain);
        let notifications = self.captain_archive_notifications(&group);
        for thread in self.threads.iter().filter(|thread| {
            group.contains(&thread.id) || thread.captain_home && thread.captain == Some(captain)
        }) {
            let real_queued = thread.native.projected_queued
                || thread.queued_prompt.is_some()
                || thread.pending_prompt.is_some()
                || thread.native_pending_turns()
                || thread.has_recoverable_native_inputs()
                || thread.pending_injections.iter().any(|injection| {
                    !notifications
                        .get(&thread.id)
                        .is_some_and(|known| known.contains(&injection.text))
                });
            if thread.status.busy()
                || thread.native.operating()
                || thread.worktree_bootstrapping()
                || thread.staged_handoff.is_some()
                || thread.inflight_handoff.is_some()
                || thread.configuring
                || thread.initializing
                || thread.session_started.is_some()
                || thread.injection_id.is_some()
                || thread.operation_deadline.is_some()
                || thread.request.is_some()
                || thread.native.questionnaire.is_some()
                || real_queued
            {
                blockers.push(format!(
                    "Stop work and answer pending requests in thread #{} before archiving.",
                    thread.id
                ));
            }
        }
        if self.tasks.iter().any(|task| {
            group.contains(&task.child_thread_id)
                && matches!(
                    task.state,
                    TaskState::Queued | TaskState::Starting | TaskState::Running
                )
        }) {
            blockers.push("Wait for the Captain's queued or running tasks to settle.".into());
        }
        blockers
    }

    /// Read-only confirmation data for supported lifecycle actions.
    pub fn captain_lifecycle_preview(&self, captain: usize, action: &str) -> Result<Value, String> {
        if action == "delete" {
            return self.captain_delete_preview(captain);
        }
        self.require_captain(captain)?;
        let mut blockers = Vec::new();
        match (action, self.captain_phase(captain)) {
            ("archive", CaptainPhase::Active) => {
                blockers.extend(self.captain_archive_blockers(captain))
            }
            ("demote", CaptainPhase::Active) => {
                blockers.extend(self.captain_retirement_blockers(captain))
            }
            ("unarchive", CaptainPhase::Archived) => {
                if let Err(error) = self
                    .validate_captain_archives()
                    .and_then(|_| self.validate_captain_archive_routines())
                {
                    blockers.push(error);
                }
            }
            ("archive" | "unarchive" | "demote", _) => {
                blockers.push("The Captain is not in the required lifecycle state.".into())
            }
            _ => return Err("Unsupported Captain lifecycle action.".into()),
        }
        let group = if action == "unarchive" {
            self.captain_lifecycles
                .get(&captain)
                .map(|record| record.archived_threads.keys().copied().collect())
                .unwrap_or_default()
        } else {
            self.captain_group_ids(captain)
        };
        let crew: Vec<_> = self
            .threads
            .iter()
            .filter(|thread| thread.crew_captain == Some(captain))
            .map(|thread| thread.id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(
            json!({"captain":captain,"action":action,"captainVersion":self.captain_version(captain)?,
            "crewSeen":crew,"affectedThreads":group,"blockers":blockers}),
        )
    }

    pub fn captain_archive_command(
        &mut self,
        captain: usize,
        action: &str,
        expected_version: &str,
        crew_seen: &[usize],
    ) -> Result<Value, String> {
        if !matches!(action, "archive" | "unarchive") {
            return Err("Unsupported Captain archive action.".into());
        }
        let preview = self.captain_lifecycle_preview(captain, action)?;
        if preview["captainVersion"] != expected_version || preview["crewSeen"] != json!(crew_seen)
        {
            return Err(
                "Captain changed since version. Review a fresh preview before confirming.".into(),
            );
        }
        if let Some(blocker) = preview["blockers"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(Value::as_str)
        {
            return Err(blocker.into());
        }
        let database = crate::database::for_path(&self.data_dir.join("index.json"))
            .ok_or("This Captain change requires the durable server.")?;
        database.require_batch()?;
        if action == "archive" {
            let group = self.captain_group_ids(captain);
            let notifications = self.captain_archive_notifications(&group);
            let paused_routines = self.background.routines.archive_captain(captain)?;
            let at = lifecycle::now();
            let archived_threads = self
                .threads
                .iter()
                .filter(|thread| group.contains(&thread.id))
                .map(|thread| (thread.id, thread.life.archived_at))
                .collect();
            // Hold exact notification records before disconnecting any process.
            for report in &mut self.captain_reports {
                if report.captain == captain && !report.delivered {
                    report.hold_delivery = true;
                }
            }
            for answer in &mut self.captain_question_answers {
                if group.contains(&answer.thread_id) && !answer.delivered {
                    answer.hold_delivery = true;
                }
            }
            let task_indices: Vec<_> = self
                .tasks
                .iter()
                .enumerate()
                .filter(|(_, task)| {
                    self.task_home(task)
                        .is_some_and(|home| group.contains(&home))
                })
                .map(|(index, _)| index)
                .collect();
            for index in task_indices {
                self.tasks[index].hold_result_notifications();
            }
            for thread in &mut self.threads {
                if !group.contains(&thread.id)
                    && !(thread.captain_home && thread.captain == Some(captain))
                {
                    continue;
                }
                for report in &mut thread.crew_report_outbox {
                    if !report.delivered {
                        report.hold_delivery = true;
                    }
                }
                thread.pending_injections.retain(|injection| {
                    !notifications
                        .get(&thread.id)
                        .is_some_and(|known| known.contains(&injection.text))
                });
                thread.crew_report_run = None;
                if group.contains(&thread.id) {
                    thread.life.archived_at.get_or_insert(at);
                }
                thread.disconnect();
                thread.prewarm_attempted = true;
            }
            self.captain_lifecycles.insert(
                captain,
                CaptainLifecycle {
                    phase: CaptainPhase::Archived,
                    archived_at: Some(at),
                    archived_threads,
                    paused_routines,
                    ..Default::default()
                },
            );
            for id in &group {
                self.invalidate_undo(*id);
            }
        } else {
            let record = self
                .captain_lifecycles
                .get(&captain)
                .cloned()
                .ok_or("Missing Captain archive record.")?;
            self.background.routines.unarchive_captain(
                captain,
                &record.paused_routines,
                chrono::Local::now(),
            )?;
            for thread in &mut self.threads {
                if let Some(previous) = record.archived_threads.get(&thread.id) {
                    thread.life.archived_at = *previous;
                    thread.prewarm_attempted = true;
                } else if thread.captain_home && thread.captain == Some(captain) {
                    thread.prewarm_attempted = true;
                }
            }
            self.captain_lifecycles.remove(&captain);
            // No release_task_results, release_crew_reports or connect here.
        }
        self.persistence_scope = None;
        Ok(
            json!({"captain":captain,"lifecycle":self.captain_phase(captain),
            "captainVersion":self.captain_version(captain)?}),
        )
    }

    pub fn captain_phase(&self, captain: usize) -> CaptainPhase {
        if captain >= self.captains.len() {
            return CaptainPhase::Deleted;
        }
        self.captain_lifecycles
            .get(&captain)
            .map_or(CaptainPhase::Active, |state| state.phase)
    }

    pub(super) fn require_active_captain(&self, captain: usize) -> Result<(), String> {
        self.require_captain(captain)?;
        match self.captain_phase(captain) {
            CaptainPhase::Active => Ok(()),
            CaptainPhase::Archived => Err("This Captain is archived. Unarchive it first.".into()),
            CaptainPhase::Retired => {
                Err("This Captain was demoted and is a read-only record.".into())
            }
            CaptainPhase::Deleted => Err("This Captain was deleted.".into()),
        }
    }

    /// Persona selection alone does not make an ordinary thread a coordinator.
    /// Earlier homes and canonical crew still belong to their inactive owner.
    pub(super) fn require_active_thread_owner(&self, id: usize) -> Result<(), String> {
        if let Some(thread) = self.threads.iter().find(|thread| thread.id == id)
            && let Some(captain) = thread
                .crew_captain
                .or_else(|| thread.captain_home.then_some(thread.captain).flatten())
        {
            self.require_active_captain(captain)?;
        }
        for (&captain, record) in &self.captain_lifecycles {
            if record.phase == CaptainPhase::Archived && record.archived_threads.contains_key(&id) {
                return self.require_active_captain(captain);
            }
        }
        Ok(())
    }

    /// Canonical crew and currently owned task children, never historical
    /// parent links belonging to released or moved work.
    pub(super) fn captain_group_ids(&self, captain: usize) -> BTreeSet<usize> {
        let mut group: BTreeSet<_> = self
            .threads
            .iter()
            .filter(|thread| thread.crew_captain == Some(captain))
            .map(|thread| thread.id)
            .chain(self.captain_home(captain))
            .collect();
        loop {
            let before = group.len();
            let children: Vec<_> = self
                .tasks
                .iter()
                .filter(|task| {
                    self.task_home(task)
                        .is_some_and(|home| group.contains(&home))
                })
                .map(|task| task.child_thread_id)
                .collect();
            group.extend(children);
            if before == group.len() {
                return group;
            }
        }
    }

    /// The token changes with confirmation-relevant state, not streaming text,
    /// elapsed clocks or catalog refreshes. It carries no authority by itself.
    pub fn captain_version(&self, captain: usize) -> Result<String, String> {
        self.require_captain(captain)?;
        let group = self.captain_group_ids(captain);
        let homes: BTreeSet<_> = self
            .threads
            .iter()
            .filter(|thread| thread.captain_home && thread.captain == Some(captain))
            .map(|thread| thread.id)
            .collect();
        let threads: Vec<_> = self
            .threads
            .iter()
            .filter(|thread| group.contains(&thread.id) || homes.contains(&thread.id) || thread.captain == Some(captain))
            .map(|thread| {
                json!({"id":thread.id,"title":thread.title,"project":thread.project,
                    "parent":thread.parent,"crew":thread.crew_captain,
                    "home":thread.captain_home,"archivedAt":thread.life.archived_at,
                    "harness":thread.harness,"machine":thread.machine,
                    "session":thread.session,"access":thread.native.access.id(),"options":thread.model_options,
                    "model":thread.selected_model,"thinking":thread.selected_thinking,
                    "worktree":thread.worktree_path,"scratchDirectory":thread.scratch_directory})
            })
            .collect();
        let documents: Vec<_> = captain_state::DOCUMENT_NAMES
            .iter()
            .map(|name| {
                let meta = self.captain_document_view(captain, name);
                json!({"name":name,"revision":meta.revision})
            })
            .collect();
        let tasks: Vec<_> = self
            .tasks
            .iter()
            .filter(|task| {
                group.contains(&task.child_thread_id)
                    || homes.contains(&task.parent_thread_id)
                    || homes.contains(&task.child_thread_id)
            })
            .map(|task| {
                json!({"id":task.id,"parent":task.parent_thread_id,
                    "child":task.child_thread_id,"state":task.state,
                    "delivered":task.delivered,"deliveryId":task.delivery_id,
                    "pendingResults":task.pending_deliveries()})
            })
            .collect();
        let reports: Vec<_> = self
            .captain_reports
            .iter()
            .chain(
                self.threads
                    .iter()
                    .flat_map(|thread| &thread.crew_report_outbox),
            )
            .filter(|report| report.captain == captain)
            .map(|report| json!({"id":report.id,"delivered":report.delivered}))
            .collect();
        let answers: Vec<_> = self
            .captain_question_answers
            .iter()
            .filter(|answer| answer.captain == captain)
            .map(|answer| json!({"id":answer.id,"delivered":answer.delivered}))
            .collect();
        let routines: Vec<_> = self
            .background
            .routines
            .entries()
            .iter()
            .filter(|routine| routine.captain == captain)
            .collect();
        Ok(crate::protocol::revision(&json!({
            "captain":captain,"profile":self.captains[captain].to_value(),
            "lifecycle":self.captain_lifecycles.get(&captain).cloned().unwrap_or_default(),
            "home":self.captain_home(captain),"threads":threads,
            "scope":self.captain_scope(captain),"remoteScope":self.remote_captain_scope(captain),
            "documents":documents,"memory":self.memories(captain),
            "tasks":tasks,"reports":reports,"answers":answers,"routines":routines
        })))
    }

    pub(super) fn restore_captain_lifecycles(&mut self, index: &Value) -> Result<(), String> {
        let mut records = BTreeMap::new();
        if let Some(value) = index.get("captainLifecycles") {
            let values = value
                .as_object()
                .ok_or("Invalid Captain lifecycle records.")?;
            for (key, value) in values {
                let captain: usize = key.parse().map_err(|_| "Invalid Captain lifecycle id.")?;
                if key != &captain.to_string() || captain >= self.captains.len() {
                    return Err("Invalid Captain lifecycle id.".into());
                }
                let record: CaptainLifecycle = serde_json::from_value(value.clone())
                    .map_err(|_| "Invalid Captain lifecycle record.")?;
                if value.get("archivedThreads").is_some_and(|threads| {
                    threads.as_object().is_none_or(|threads| {
                        threads.keys().any(|key| {
                            key.parse::<usize>()
                                .ok()
                                .is_none_or(|id| id == 0 || key != &id.to_string())
                        })
                    })
                }) {
                    return Err("Invalid Captain archive thread id.".into());
                }
                let valid = match record.phase {
                    CaptainPhase::Active | CaptainPhase::Deleted => {
                        record.archived_at.is_none()
                            && record.retired_at.is_none()
                            && record.former_home.is_none()
                            && record.archived_threads.is_empty()
                            && record.paused_routines.is_empty()
                    }
                    CaptainPhase::Retired => {
                        record.retired_at.is_some_and(|time| time > 0)
                            && record.archived_at.is_none()
                            && record.archived_threads.is_empty()
                            && record.paused_routines.is_empty()
                    }
                    CaptainPhase::Archived => {
                        record.archived_at.is_some_and(|time| time > 0)
                            && record.retired_at.is_none()
                            && record.former_home.is_none()
                            && self
                                .captain_home(captain)
                                .is_some_and(|home| record.archived_threads.contains_key(&home))
                            && record.archived_threads.keys().all(|id| {
                                self.threads.iter().any(|thread| {
                                    thread.id == *id
                                        && thread.life.archived_at
                                            == record.archived_threads[id].or(record.archived_at)
                                })
                            })
                            && record
                                .archived_threads
                                .values()
                                .all(|at| at.is_none_or(|at| at > 0))
                            && record.paused_routines.iter().all(|id| *id > 0)
                            && record.paused_routines.iter().collect::<BTreeSet<_>>().len()
                                == record.paused_routines.len()
                    }
                };
                if record.version != 1 || !valid {
                    return Err("Inconsistent Captain lifecycle record.".into());
                }
                records.insert(captain, record);
            }
        }
        self.captain_lifecycles = records;
        Ok(())
    }

    pub(super) fn validate_captain_archives(&self) -> Result<(), String> {
        for (&captain, record) in &self.captain_lifecycles {
            if record.phase == CaptainPhase::Archived
                && self.captain_group_ids(captain)
                    != record.archived_threads.keys().copied().collect()
            {
                return Err(
                    "Captain archive membership changed without restoring the group.".into(),
                );
            }
            if matches!(record.phase, CaptainPhase::Retired | CaptainPhase::Deleted) {
                if self.threads.iter().any(|thread| {
                    thread.crew_captain == Some(captain)
                        || thread.captain_home && thread.captain == Some(captain)
                }) || self.captain_homes.contains_key(&captain)
                    || self
                        .projects
                        .entries
                        .iter()
                        .any(|project| project.captain == Some(captain))
                {
                    return Err("An inactive Captain still owns active role records.".into());
                }
                if record.phase == CaptainPhase::Retired
                    && !record.former_home.is_some_and(|home| home > 0)
                {
                    return Err("A retired Captain has no former home identity.".into());
                }
            }
            if record.phase == CaptainPhase::Deleted
                && (self.captains[captain].name != format!("Deleted Captain {captain}")
                    || !self.captains[captain].role.is_empty()
                    || !self.captains[captain].instructions.is_empty()
                    || self.captains[captain].portrait != 0
                    || self.captain_books.contains_key(&captain)
                    || self.captain_scopes.contains_key(&captain)
                    || self.captain_remote_scopes.contains_key(&captain)
                    || self
                        .threads
                        .iter()
                        .any(|thread| thread.captain == Some(captain)))
            {
                return Err("A deleted Captain still has private or assigned records.".into());
            }
        }
        Ok(())
    }

    pub(super) fn validate_captain_archive_routines(&self) -> Result<(), String> {
        for (&captain, record) in &self.captain_lifecycles {
            if record.phase != CaptainPhase::Archived {
                continue;
            }
            self.background.routines.captain_idle(captain)?;
            let routines = self.background.routines.entries();
            if routines
                .iter()
                .any(|routine| routine.captain == captain && !routine.paused)
                || record.paused_routines.iter().any(|id| {
                    !routines.iter().any(|routine| {
                        routine.id == *id && routine.captain == captain && routine.paused
                    })
                })
            {
                return Err("Captain archive routine records are inconsistent.".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> State {
        let mut state =
            State::headless(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()));
        state.demo = true;
        state.captains = Captain::presets();
        state.open_captain(0);
        state
    }

    fn worker(state: &mut State) -> usize {
        let id = state.next_id;
        state.next_id += 1;
        let mut thread = Thread::new(id);
        thread.crew_captain = Some(0);
        thread.captain = Some(0);
        state.threads.push(thread);
        id
    }

    fn archive_record(state: &mut State, group: BTreeSet<usize>) {
        let previous = state
            .threads
            .iter()
            .filter(|thread| group.contains(&thread.id))
            .map(|thread| (thread.id, thread.life.archived_at))
            .collect();
        for thread in &mut state.threads {
            if group.contains(&thread.id) {
                thread.life.archived_at.get_or_insert(42);
            }
        }
        state.captain_lifecycles.insert(
            0,
            CaptainLifecycle {
                phase: CaptainPhase::Archived,
                archived_at: Some(42),
                archived_threads: previous,
                ..Default::default()
            },
        );
    }

    #[test]
    fn old_profiles_default_active_and_lifecycle_records_fail_closed() {
        let state = fixture();
        let snapshot = state.snapshot();
        let mut old = snapshot.clone();
        old.as_object_mut().unwrap().remove("captainLifecycles");
        let mut restored = fixture();
        restored.restore_global(&old).unwrap();
        assert_eq!(restored.captain_phase(0), CaptainPhase::Active);
        for invalid in [
            Value::Null,
            json!([]),
            json!({"00":{}}),
            json!({"999":{}}),
            json!({"0":null}),
            json!({"0":{}}),
            json!({"0":{"phase":"unknown"}}),
            json!({"0":{"version":1,"phase":"archived"}}),
            json!({"0":{"version":1,"phase":"active","archivedThreads":{"1":null}}}),
        ] {
            let mut broken = snapshot.clone();
            broken["captainLifecycles"] = invalid.clone();
            assert!(fixture().restore_global(&broken).is_err(), "{invalid}");
        }
    }

    #[test]
    fn confirmation_versions_change_on_documents_and_membership_not_streaming_text() {
        let mut state = fixture();
        let before = state.captain_version(0).unwrap();
        let home = state.captain_home(0).unwrap();
        let thread = state
            .threads
            .iter_mut()
            .find(|thread| thread.id == home)
            .unwrap();
        thread.phase = "streaming another token".into();
        assert_eq!(state.captain_version(0).unwrap(), before);
        let worker = state.next_id;
        state.next_id += 1;
        let mut thread = Thread::new(worker);
        thread.crew_captain = Some(0);
        thread.captain = Some(0);
        state.threads.push(thread);
        assert_ne!(state.captain_version(0).unwrap(), before);
        let with_crew = state.captain_version(0).unwrap();
        state.captain_books.entry(0).or_default().documents.insert(
            "NOTES.md".into(),
            CaptainDocument {
                text: "A different plan".into(),
                revision: 1,
                updated_at: 1,
            },
        );
        assert_ne!(state.captain_version(0).unwrap(), with_crew);
    }

    #[test]
    fn opening_an_archived_home_never_restores_it_and_mutations_stay_blocked() {
        let mut state = fixture();
        let home = state.captain_home(0).unwrap();
        let worker = worker(&mut state);
        let group = state.captain_group_ids(0);
        archive_record(&mut state, group);
        state.open_captain(0);
        assert_eq!(state.current().id, home);
        assert_eq!(state.current().life.archived_at, Some(42));
        assert!(state.captain_document_value(0, "NOTES.md").is_ok());
        assert!(
            state
                .captain_document_write(0, "NOTES.md", "edit".into(), 0)
                .is_err()
        );
        assert!(state.set_captain_scope(0, vec![]).is_err());
        assert!(state.reset_captain_conversation(0).is_err());
        assert!(state.captain_crew_release(0, worker).is_err());
        assert!(
            state
                .mcp_call(home, "delegate_task", &json!({"prompt":"Work"}))
                .is_err()
        );
        for method in [
            "thread.send",
            "thread.resume",
            "thread.delete",
            "thread.lifecycle",
        ] {
            assert!(
                state
                    .execute_command(&crate::protocol::Command::new(
                        method,
                        json!({"threadId":worker,"action":"unarchive","text":"Work"})
                    ))
                    .is_err(),
                "{method}"
            );
        }
        state.archive_confirmed(worker);
        assert_eq!(
            state
                .threads
                .iter()
                .find(|thread| thread.id == worker)
                .unwrap()
                .life
                .archived_at,
            Some(42)
        );
        assert!(state.require_active_thread_owner(worker).is_err());
        let snapshot = state.snapshot();
        let mut restored = fixture();
        restored.restore_global(&snapshot).unwrap();
        assert_eq!(restored.captain_phase(0), CaptainPhase::Archived);
        assert!(restored.require_active_thread_owner(worker).is_err());
    }

    #[test]
    fn archive_preflight_ignores_only_exact_notification_text_and_checks_other_queues() {
        let mut state = fixture();
        let home = state.captain_home(0).unwrap();
        let source = worker(&mut state);
        let report: crew_reports::CrewReport = serde_json::from_value(json!({
            "id":format!("crew-{}", uuid::Uuid::new_v4()),"captain":0,
            "threadId":source,"runId":"run-a","title":"Worker","outcome":"completed",
            "summary":"Done","error":"","createdAt":1,"delivered":false
        }))
        .unwrap();
        let exact = report.message();
        state.captain_reports.push(report);
        let index = state
            .threads
            .iter()
            .position(|thread| thread.id == home)
            .unwrap();
        state.threads[index]
            .pending_injections
            .push_back(Injection {
                text: exact.clone(),
                shown: false,
            });
        assert!(state.captain_archive_blockers(0).is_empty());
        state.threads[index].pending_injections[0]
            .text
            .push_str("\nUser-directed extra work");
        assert!(!state.captain_archive_blockers(0).is_empty());
        state.threads[index].pending_injections[0].text = exact;
        state.threads[index].native.projected_queued = true;
        assert!(!state.captain_archive_blockers(0).is_empty());
        state.threads[index].native.projected_queued = false;
        state.threads[index].injection_id = Some("in-flight".into());
        assert!(!state.captain_archive_blockers(0).is_empty());
        state.threads[index].injection_id = None;
        state.threads[index].request = Some(json!({"method":"confirm"}));
        assert!(!state.captain_archive_blockers(0).is_empty());
        state.threads[index].request = None;
        state.threads[index].native.pending_command = Some(json!({"name":"compact"}));
        assert!(!state.captain_archive_blockers(0).is_empty());
        state.threads[index].native.pending_command = None;
        let id = state.next_id;
        state.next_id += 1;
        let mut earlier = Thread::new(id);
        earlier.captain_home = true;
        earlier.captain = Some(0);
        earlier.status = Status::Running;
        state.threads.push(earlier);
        assert!(
            !state.captain_archive_blockers(0).is_empty(),
            "earlier homes must be idle too"
        );
    }

    #[test]
    fn archive_refuses_stale_confirmation_and_nontransactional_execution_without_mutating() {
        let mut state = fixture();
        let preview = state.captain_lifecycle_preview(0, "archive").unwrap();
        worker(&mut state);
        assert!(
            state
                .captain_archive_command(
                    0,
                    "archive",
                    preview["captainVersion"].as_str().unwrap(),
                    &[]
                )
                .unwrap_err()
                .starts_with("Captain changed since version")
        );
        let preview = state.captain_lifecycle_preview(0, "archive").unwrap();
        let crew = serde_json::from_value::<Vec<usize>>(preview["crewSeen"].clone()).unwrap();
        assert!(
            state
                .captain_archive_command(
                    0,
                    "archive",
                    preview["captainVersion"].as_str().unwrap(),
                    &crew
                )
                .unwrap_err()
                .contains("durable server")
        );
        assert_eq!(state.captain_phase(0), CaptainPhase::Active);
        assert!(state.threads.iter().all(|thread| !thread.archived()));
    }

    #[test]
    fn exact_task_results_can_be_archived_and_stay_held_after_unarchive() {
        let mut state = fixture();
        let home = state.captain_home(0).unwrap();
        let source = worker(&mut state);
        let child = state
            .threads
            .iter_mut()
            .find(|thread| thread.id == source)
            .unwrap();
        child.parent = Some(home);
        child.delegated = true;
        let mut task = tasks::Task::new(child, "Work", None);
        task.finish(TaskState::Completed, "Result", "");
        let mut record = serde_json::to_value(&task).unwrap();
        let prior_text = format!("{INJECTION_PREFIX}[delivery earlier-result]\nEarlier result");
        record["prior_deliveries"] = json!([{"id":"earlier-result","text":prior_text}]);
        state.tasks.push(serde_json::from_value(record).unwrap());
        let index = state
            .threads
            .iter()
            .position(|thread| thread.id == home)
            .unwrap();
        for text in state.tasks[0].result_notifications() {
            state.threads[index]
                .pending_injections
                .push_back(Injection { text, shown: false });
        }
        assert_eq!(state.threads[index].pending_injections.len(), 2);
        assert!(state.captain_archive_blockers(0).is_empty());
        let database = crate::database::Database::open(&state.data_dir).unwrap();
        // Merely having a database is insufficient without the runtime batch.
        let version = state.captain_version(0).unwrap();
        assert!(
            state
                .captain_archive_command(0, "archive", &version, &[source])
                .is_err()
        );
        database.begin().unwrap();
        state
            .captain_archive_command(0, "archive", &version, &[source])
            .unwrap();
        assert!(state.tasks[0].hold_delivery);
        assert!(state.threads[index].pending_injections.is_empty());
        let version = state.captain_version(0).unwrap();
        state
            .captain_archive_command(0, "unarchive", &version, &[source])
            .unwrap();
        state.poll_tasks(Instant::now());
        assert!(state.tasks[0].hold_delivery);
        assert!(!state.tasks[0].delivered);
        assert!(state.threads[index].pending_injections.is_empty());
    }

    #[test]
    fn restored_archives_reject_foreign_threads_missing_members_and_changed_markers() {
        let mut state = fixture();
        let source = worker(&mut state);
        let group = state.captain_group_ids(0);
        archive_record(&mut state, group);
        let snapshot = state.snapshot();
        for field in ["missing", "marker", "foreign"] {
            let mut broken = snapshot.clone();
            match field {
                "missing" => {
                    broken["captainLifecycles"]["0"]["archivedThreads"]
                        .as_object_mut()
                        .unwrap()
                        .remove(&source.to_string());
                }
                "marker" => {
                    broken["captainLifecycles"]["0"]["archivedThreads"][source.to_string()] =
                        json!(7);
                }
                _ => {
                    broken["captainLifecycles"]["0"]["archivedThreads"]["999"] = Value::Null;
                }
            }
            assert!(fixture().restore_global(&broken).is_err(), "{field}");
        }
    }
}
