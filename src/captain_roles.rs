//! Role changes keep thread, session, task and execution-placement identities.
//! They never allocate replacement conversations or grant provider permissions.
use super::*;
use std::collections::BTreeSet;

impl State {
    fn role_thread_idle(&self, index: usize, allow_archived: bool) -> Result<(), String> {
        let thread = &self.threads[index];
        if thread.archived() && !allow_archived
            || thread.status.busy()
            || thread.configuring
            || thread.initializing
            || thread.session_started.is_some()
            || thread.queued()
            || thread.has_recoverable_native_inputs()
            || thread.native.operating()
            || thread.worktree_bootstrapping()
            || thread.request.is_some()
            || thread.native.questionnaire.is_some()
            || thread.injection_id.is_some()
            || thread.operation_deadline.is_some()
            || thread.staged_handoff.is_some()
            || thread.inflight_handoff.is_some()
        {
            return Err(format!(
                "Restore thread #{} and finish its pending work before changing its role.",
                thread.id
            ));
        }
        Ok(())
    }

    fn promotion_children(&self, home: usize) -> BTreeSet<usize> {
        self.tasks
            .iter()
            .filter(|task| self.task_home(task) == Some(home))
            .map(|task| task.child_thread_id)
            .chain(
                self.threads
                    .iter()
                    .filter(|thread| {
                        thread.parent == Some(home)
                            && thread.crew_captain.is_none()
                            && !self
                                .tasks
                                .iter()
                                .any(|task| task.child_thread_id == thread.id)
                    })
                    .map(|thread| thread.id),
            )
            .collect()
    }

    fn promotion_gate(&self, index: usize, restore: Option<usize>) -> Result<(), String> {
        let thread = &self.threads[index];
        self.require_active_thread_owner(thread.id)?;
        if thread.machine != "local" {
            return Err("Promotion must run on the thread's owning server.".into());
        }
        if thread.captain_home
            || thread.crew_captain.is_some()
            || thread.delegated
            || thread.peer_origin.is_some()
            || thread.parent.is_some()
        {
            return Err("Only an ordinary independent thread can become a Captain.".into());
        }
        self.role_thread_idle(index, false)?;
        let children = self.promotion_children(thread.id);
        for child in self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, child)| children.contains(&child.id))
        {
            self.role_thread_idle(child.0, true)?;
        }
        if self.tasks.iter().any(|task| {
            children.contains(&task.child_thread_id)
                && matches!(
                    task.state,
                    TaskState::Queued | TaskState::Starting | TaskState::Running
                )
        }) {
            return Err("Stop the thread's active descendants before promotion.".into());
        }
        if let Some(captain) = restore {
            self.require_captain(captain)?;
            if self.captain_phase(captain) != CaptainPhase::Retired
                || self.captain_former_home(captain) != Some(thread.id)
            {
                return Err(
                    "This thread is not the surviving former home of that retired Captain.".into(),
                );
            }
            self.background.routines.captain_idle(captain)?;
            if self.captain_scope(captain).first().copied() != thread.project {
                return Err("The former home changed workspace. Restore its original workspace before restoring this Captain profile.".into());
            }
        }
        Ok(())
    }

    pub fn captain_former_home(&self, captain: usize) -> Option<usize> {
        self.captain_lifecycles
            .get(&captain)
            .and_then(|record| record.former_home)
    }

    fn promotion_version(&self, index: usize, restore: Option<usize>) -> Result<String, String> {
        let thread = &self.threads[index];
        let children = self.promotion_children(thread.id);
        let tasks: Vec<_> = self
            .tasks
            .iter()
            .filter(|task| children.contains(&task.child_thread_id))
            .collect();
        Ok(crate::protocol::revision(&json!({
            "thread":thread.id,"title":thread.title,"machine":thread.machine,"project":thread.project,
            "harness":thread.harness,"model":thread.selected_model,"thinking":thread.selected_thinking,
            "options":thread.model_options,"session":thread.session,"access":thread.native.access.id(),
            "worktree":thread.worktree_path,"scratchDirectory":thread.scratch_directory,
            "captain":thread.captain,"home":thread.captain_home,"crew":thread.crew_captain,
            "archivedAt":thread.life.archived_at,"children":children,"tasks":tasks,
            "restore":restore.map(|captain| self.captain_version(captain)).transpose()?
        })))
    }

    pub fn captain_promote_preview(
        &self,
        thread_id: usize,
        restore: Option<usize>,
    ) -> Result<Value, String> {
        let index = self
            .threads
            .iter()
            .position(|thread| thread.id == thread_id)
            .ok_or("Unknown thread.")?;
        let blockers: Vec<_> = self
            .promotion_gate(index, restore)
            .err()
            .into_iter()
            .collect();
        Ok(json!({"threadId":thread_id,"restoreCaptain":restore,
            "threadVersion":self.promotion_version(index, restore)?,"blockers":blockers,
            "permissionsChanged":false}))
    }

    /// Resolve relative Pi session references before changing the role's default
    /// scratch directory. Git-worktree metadata is never used as a scratch alias.
    pub(super) fn prepare_role_directory(
        &self,
        index: usize,
    ) -> Result<(Option<String>, Option<String>), String> {
        let thread = &self.threads[index];
        let directory = self.thread_directory(index)?;
        let scratch = if thread.project.is_none() && thread.worktree_path.is_none() {
            Some(
                directory
                    .to_str()
                    .ok_or("The working directory is not valid UTF-8.")?
                    .to_owned(),
            )
        } else {
            thread.scratch_directory.clone()
        };
        let session = if thread.harness == "pi" {
            thread.session.as_ref().map(|session| {
                if std::path::Path::new(session).is_absolute() {
                    session.clone()
                } else {
                    directory.join(session).to_string_lossy().into_owned()
                }
            })
        } else {
            thread.session.clone()
        };
        Ok((scratch, session))
    }

    pub fn captain_promote(
        &mut self,
        thread_id: usize,
        profile: Captain,
        restore: Option<usize>,
        expected_version: &str,
    ) -> Result<Value, String> {
        let index = self
            .threads
            .iter()
            .position(|thread| thread.id == thread_id)
            .ok_or("Unknown thread.")?;
        if self.promotion_version(index, restore)? != expected_version {
            return Err("Thread changed since version. Review a fresh promotion preview.".into());
        }
        self.promotion_gate(index, restore)?;
        profile.validate()?;
        if let Some(captain) = restore {
            if self.captains[captain] != profile {
                return Err("Restore uses the retired Captain's unchanged profile.".into());
            }
        } else if self.captains.iter().enumerate().any(|(captain, existing)| {
            self.captain_phase(captain) != CaptainPhase::Deleted
                && existing.name.to_lowercase() == profile.name.to_lowercase()
        }) {
            return Err("That Captain name is already in the roster.".into());
        }
        if restore.is_none() && !self.memories(self.captains.len()).is_empty() {
            return Err("Unclaimed memory records occupy the next Captain identity. Repair the profile before promotion.".into());
        }
        let (scratch, session) = self.prepare_role_directory(index)?;
        let database = crate::database::for_path(&self.data_dir.join("index.json"))
            .ok_or("This Captain change requires the durable server.")?;
        database.require_batch()?;
        let captain = restore.unwrap_or(self.captains.len());
        if restore.is_none() {
            self.captains.push(profile);
            self.captain_scopes
                .insert(captain, self.threads[index].project.into_iter().collect());
        } else {
            self.captain_lifecycles.remove(&captain);
        }
        let thread = &mut self.threads[index];
        thread.scratch_directory = scratch;
        thread.session = session;
        thread.disconnect();
        thread.prewarm_attempted = true;
        thread.captain = Some(captain);
        thread.captain_home = true;
        thread.scope_context_pending = true;
        self.captain_homes.insert(captain, thread_id);
        self.invalidate_undo(thread_id);
        self.sync_memory_links();
        self.refresh_captain_contexts();
        self.persistence_scope = None;
        Ok(
            json!({"captain":captain,"threadId":thread_id,"captainVersion":self.captain_version(captain)?,
            "permissionsChanged":false}),
        )
    }

    pub(super) fn captain_retirement_blockers(&self, captain: usize) -> Vec<String> {
        if let Err(error) = self.require_active_captain(captain) {
            return vec![error];
        }
        let mut blockers = self.captain_settlement_blockers(captain);
        if self.captain_home(captain).is_none() {
            blockers.push("This Captain has no primary home.".into());
        }
        blockers
    }

    pub(super) fn captain_settlement_blockers(&self, captain: usize) -> Vec<String> {
        let mut blockers = Vec::new();
        if let Err(error) = self.background.routines.captain_idle(captain) {
            blockers.push(error);
        }
        let group = self.captain_group_ids(captain);
        for (index, _) in self.threads.iter().enumerate().filter(|(_, thread)| {
            group.contains(&thread.id) || thread.captain_home && thread.captain == Some(captain)
        }) {
            if let Err(error) = self.role_thread_idle(index, true) {
                blockers.push(error);
            }
        }
        if self.tasks.iter().any(|task| {
            group.contains(&task.child_thread_id)
                && (!task.state.finished() || !task.delivered || task.pending_deliveries())
        }) {
            blockers.push(
                "Every owned task result must be accepted before changing the Captain.".into(),
            );
        }
        if self
            .captain_reports
            .iter()
            .chain(
                self.threads
                    .iter()
                    .flat_map(|thread| &thread.crew_report_outbox),
            )
            .any(|report| {
                report.captain == captain
                    && (report.needs_delivery()
                        || report.question.is_some()
                            && !self
                                .captain_question_answers
                                .iter()
                                .any(|answer| answer.report_id == report.id && answer.delivered))
            })
        {
            blockers.push("Deliver every crew report and settle every question and answer before changing the Captain.".into());
        }
        blockers
    }

    pub fn captain_demote(
        &mut self,
        captain: usize,
        expected_version: &str,
        crew_seen: &[usize],
    ) -> Result<Value, String> {
        let preview = self.captain_lifecycle_preview(captain, "demote")?;
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
        let home = self
            .captain_home(captain)
            .ok_or("This Captain has no primary home.")?;
        let homes: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, thread)| thread.captain_home && thread.captain == Some(captain))
            .map(|(index, _)| index)
            .collect();
        let directories = homes
            .iter()
            .map(|index| self.prepare_role_directory(*index))
            .collect::<Result<Vec<_>, _>>()?;
        let database = crate::database::for_path(&self.data_dir.join("index.json"))
            .ok_or("This Captain change requires the durable server.")?;
        database.require_batch()?;
        self.background.routines.archive_captain(captain)?;
        let scope = self.captain_scope(captain);
        self.captain_scopes.insert(captain, scope);
        let at = lifecycle::now();
        for (index, (scratch, session)) in homes.into_iter().zip(directories) {
            let thread = &mut self.threads[index];
            thread.scratch_directory = scratch;
            thread.session = session;
            thread.disconnect();
            thread.prewarm_attempted = true;
            thread.captain_home = false;
            thread.captain = None;
            if thread.id != home {
                thread.life.archived_at.get_or_insert(at);
            }
            self.invalidate_undo(self.threads[index].id);
        }
        let crew: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, thread)| thread.crew_captain == Some(captain))
            .map(|(index, _)| index)
            .collect();
        for index in crew {
            self.threads[index].crew_report_run = None;
            self.threads[index].disconnect();
            self.threads[index].prewarm_attempted = true;
            self.threads[index].crew_captain = None;
            self.threads[index].captain = None;
            self.invalidate_undo(self.threads[index].id);
        }
        for project in &mut self.projects.entries {
            if project.captain == Some(captain) {
                project.captain = None;
                project.captain_thread = None;
            }
        }
        self.captain_homes.remove(&captain);
        self.captain_lifecycles.insert(
            captain,
            captain_lifecycle::CaptainLifecycle {
                phase: CaptainPhase::Retired,
                retired_at: Some(at),
                former_home: Some(home),
                ..Default::default()
            },
        );
        self.sync_memory_links();
        self.refresh_captain_contexts();
        self.persistence_scope = None;
        Ok(
            json!({"captain":captain,"threadId":home,"lifecycle":"retired",
            "captainVersion":self.captain_version(captain)?}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (State, crate::database::Database) {
        let directory = std::env::temp_dir().join(format!("shina-roles-{}", uuid::Uuid::new_v4()));
        let database = crate::database::Database::open(&directory).unwrap();
        let mut state = State::headless(directory);
        state.demo = true;
        state.captains = Captain::presets();
        database.begin().unwrap();
        (state, database)
    }

    fn profile() -> Captain {
        let mut profile = Captain::presets().remove(0);
        profile.name = "New Captain".into();
        profile
    }

    fn promote(state: &mut State, thread: usize, restore: Option<usize>) -> usize {
        let preview = state.captain_promote_preview(thread, restore).unwrap();
        let profile = restore
            .map(|captain| state.captains[captain].clone())
            .unwrap_or_else(profile);
        state
            .captain_promote(
                thread,
                profile,
                restore,
                preview["threadVersion"].as_str().unwrap(),
            )
            .unwrap()["captain"]
            .as_u64()
            .unwrap() as usize
    }

    #[test]
    fn promotion_and_demotion_preserve_scratch_session_access_tasks_and_stable_profile_ids() {
        let (mut state, _database) = fixture();
        let home = state.current().id;
        let directory = state.thread_directory(state.active).unwrap();
        state.current_mut().session = Some("saved.jsonl".into());
        state.current_mut().native.access = AccessMode::Full;
        state.current_mut().selected_model = Some(("cpa".into(), "model-a".into()));
        let child_id = state.next_id;
        state.next_id += 1;
        let mut child = Thread::new(child_id);
        child.parent = Some(home);
        child.delegated = true;
        let mut task = tasks::Task::new(&child, "Existing task", None);
        task.finish(TaskState::Completed, "Existing result", "");
        task.delivered = true;
        state.tasks.push(task);
        state.threads.push(child);
        let original_task = serde_json::to_value(&state.tasks[0]).unwrap();
        let captain = promote(&mut state, home, None);
        assert_eq!(
            captain, 4,
            "promotion appends, never reuses another profile"
        );
        assert_eq!(state.current().id, home);
        assert_eq!(state.current().native.access, AccessMode::Full);
        assert_eq!(
            state.current().selected_model,
            Some(("cpa".into(), "model-a".into()))
        );
        assert_eq!(
            state.current().session.as_deref(),
            directory.join("saved.jsonl").to_str()
        );
        assert_eq!(state.thread_directory(state.active).unwrap(), directory);
        assert!(state.current().worktree_path.is_none());
        assert_eq!(
            serde_json::to_value(&state.tasks[0]).unwrap(),
            original_task
        );
        let preview = state.captain_lifecycle_preview(captain, "demote").unwrap();
        state
            .captain_demote(captain, preview["captainVersion"].as_str().unwrap(), &[])
            .unwrap();
        assert_eq!(state.captain_phase(captain), CaptainPhase::Retired);
        assert!(!state.current().captain_home);
        assert_eq!(state.current().captain, None);
        assert_eq!(state.thread_directory(state.active).unwrap(), directory);
        assert_eq!(state.current().native.access, AccessMode::Full);
        assert_eq!(
            serde_json::to_value(&state.tasks[0]).unwrap(),
            original_task
        );
        let snapshot = state.snapshot();
        let mut restored = State::headless(state.data_dir.clone());
        restored.demo = true;
        restored.restore_global(&snapshot).unwrap();
        assert_eq!(restored.captain_phase(captain), CaptainPhase::Retired);
        assert_eq!(
            restored.thread_directory(restored.active).unwrap(),
            directory
        );
        assert_eq!(promote(&mut restored, home, Some(captain)), captain);
        assert_eq!(restored.captains.len(), 5);
        assert_eq!(
            serde_json::to_value(&restored.tasks[0]).unwrap(),
            original_task
        );
        assert_eq!(restored.current().native.access, AccessMode::Full);
    }

    #[test]
    fn promotion_rejects_stale_config_and_active_descendants_without_allocating_a_profile() {
        let (mut state, _database) = fixture();
        let home = state.current().id;
        let preview = state.captain_promote_preview(home, None).unwrap();
        state.current_mut().native.access = AccessMode::ReadOnly;
        assert!(
            state
                .captain_promote(
                    home,
                    profile(),
                    None,
                    preview["threadVersion"].as_str().unwrap()
                )
                .unwrap_err()
                .starts_with("Thread changed since version")
        );
        assert_eq!(state.captains.len(), 4);
        let mut child = Thread::new(state.next_id);
        state.next_id += 1;
        child.parent = Some(home);
        child.delegated = true;
        child.status = Status::Running;
        state.threads.push(child);
        let preview = state.captain_promote_preview(home, None).unwrap();
        assert!(!preview["blockers"].as_array().unwrap().is_empty());
        assert!(
            state
                .captain_promote(
                    home,
                    profile(),
                    None,
                    preview["threadVersion"].as_str().unwrap()
                )
                .is_err()
        );
        assert_eq!(state.captains.len(), 4);
        assert!(!state.current().captain_home);
    }

    #[test]
    fn demotion_keeps_cross_workspace_task_provenance_and_earlier_home_directories() {
        let (mut state, _database) = fixture();
        state.projects.entries = vec![
            Project::new(1, state.data_dir.join("first-project")),
            Project::new(2, state.data_dir.join("second-project")),
        ];
        state.projects.next_id = 3;
        state.current_mut().project = Some(1);
        let home = state.current().id;
        let captain = promote(&mut state, home, None);
        state.captain_scopes.insert(captain, vec![1, 2]);
        let earlier_id = state.next_id;
        state.next_id += 1;
        let mut earlier = Thread::new(earlier_id);
        earlier.captain = Some(captain);
        earlier.captain_home = true;
        earlier.session = Some("earlier.jsonl".into());
        state.threads.push(earlier);
        let earlier_index = state.threads.len() - 1;
        let earlier_directory = state.thread_directory(earlier_index).unwrap();
        let child_id = state.next_id;
        state.next_id += 1;
        let mut child = Thread::new(child_id);
        child.project = Some(2);
        child.parent = Some(home);
        child.captain = Some(captain);
        child.crew_captain = Some(captain);
        child.delegated = true;
        child.worktree_path = Some(
            state
                .data_dir
                .join("worker-worktree")
                .to_string_lossy()
                .into_owned(),
        );
        let mut task = tasks::Task::new(&child, "Existing cross-project task", None);
        task.finish(TaskState::Completed, "Saved result", "");
        task.delivered = true;
        state.tasks.push(task);
        state.threads.push(child);
        let task_record = serde_json::to_value(&state.tasks[0]).unwrap();
        let preview = state.captain_lifecycle_preview(captain, "demote").unwrap();
        state
            .captain_demote(
                captain,
                preview["captainVersion"].as_str().unwrap(),
                &[child_id],
            )
            .unwrap();
        assert_eq!(
            state.thread_directory(earlier_index).unwrap(),
            earlier_directory
        );
        assert_eq!(
            state.threads[earlier_index].session.as_deref(),
            earlier_directory.join("earlier.jsonl").to_str()
        );
        assert!(state.threads[earlier_index].archived());
        assert_eq!(serde_json::to_value(&state.tasks[0]).unwrap(), task_record);
        let mut restored = State::headless(state.data_dir.clone());
        restored.demo = true;
        restored.restore_global(&state.snapshot()).unwrap();
        assert_eq!(
            serde_json::to_value(&restored.tasks[0]).unwrap(),
            task_record
        );
        let worker = restored
            .threads
            .iter()
            .find(|thread| thread.id == child_id)
            .unwrap();
        assert_eq!(worker.parent, Some(home));
        assert!(worker.worktree_path.is_some());
        assert_eq!(worker.crew_captain, None);
    }

    #[test]
    fn demotion_refuses_unaccepted_reports_and_questions_then_releases_only_canonical_members() {
        let (mut state, _database) = fixture();
        let home = state.current().id;
        let captain = promote(&mut state, home, None);
        let source = state.next_id;
        state.next_id += 1;
        let mut worker = Thread::new(source);
        worker.crew_captain = Some(captain);
        worker.captain = Some(captain);
        state.threads.push(worker);
        let report: crew_reports::CrewReport = serde_json::from_value(json!({
            "id":format!("crew-{}", uuid::Uuid::new_v4()),"captain":captain,
            "threadId":source,"runId":"run-a","title":"Worker","outcome":"completed",
            "summary":"Claim","error":"","createdAt":1,"delivered":false,
            "requestId":uuid::Uuid::new_v4(),"question":{"text":"Proceed?","recipient":"user"}
        }))
        .unwrap();
        state.captain_reports.push(report);
        assert!(!state.captain_retirement_blockers(captain).is_empty());
        state.captain_reports[0].delivered = true;
        assert!(
            !state.captain_retirement_blockers(captain).is_empty(),
            "delivery is not a human answer"
        );
        state
            .captain_question_answers
            .push(crew_questions::CrewAnswer {
                id: format!("crew-answer-{}", uuid::Uuid::new_v4()),
                report_id: state.captain_reports[0].id.clone(),
                captain,
                thread_id: source,
                request_id: uuid::Uuid::new_v4().to_string(),
                answer: "Proceed".into(),
                actor: crew_questions::AnswerActor::User,
                created_at: 2,
                delivered: false,
                hold_delivery: true,
            });
        assert!(
            !state.captain_retirement_blockers(captain).is_empty(),
            "saving is not source acceptance"
        );
        state.captain_question_answers[0].delivered = true;
        let preview = state.captain_lifecycle_preview(captain, "demote").unwrap();
        state
            .captain_demote(
                captain,
                preview["captainVersion"].as_str().unwrap(),
                &[source],
            )
            .unwrap();
        assert!(
            state
                .threads
                .iter()
                .all(|thread| thread.crew_captain.is_none())
        );
        assert_eq!(state.captain_reports[0].captain, captain);
        assert_eq!(state.captain_question_answers[0].thread_id, source);
        assert!(state.captain_read_value(captain, true).is_ok());
        assert!(
            state
                .captain_document_write(captain, "NOTES.md", "forbidden".into(), 0)
                .is_err()
        );
    }

    #[test]
    fn scratch_metadata_rejects_local_relative_paths_but_keeps_remote_paths_opaque() {
        let (state, _database) = fixture();
        let mut snapshot = state.snapshot();
        snapshot["threads"][0]["scratchDirectory"] = json!("relative/path");
        let mut restored = State::headless(state.data_dir.clone());
        assert!(restored.restore_global(&snapshot).is_err());
        snapshot["threads"][0]["scratchDirectory"] = json!(r"D:\Profiles\Shina\chats\1");
        restored.client = Some(crate::client::Client::new(state.data_dir.clone()));
        restored.restore_records(&snapshot, false).unwrap();
        assert_eq!(
            restored.threads[0].scratch_directory.as_deref(),
            Some(r"D:\Profiles\Shina\chats\1")
        );
        for value in [Value::Null, json!(17), json!("bad\npath")] {
            snapshot["threads"][0]["scratchDirectory"] = value;
            assert!(restored.restore_records(&snapshot, false).is_err());
        }
    }

    #[test]
    fn an_ordinary_new_draft_never_reuses_a_blank_independent_crew_root() {
        let (mut state, _database) = fixture();
        let crew = state.current().id;
        state.current_mut().crew_captain = Some(0);
        state.current_mut().captain = Some(0);
        assert!(state.current().blank_draft());
        state.create_thread_in(None, &[], &Value::Null);
        assert_ne!(state.current().id, crew);
        assert!(state.current().crew_captain.is_none());
        let preserved = state
            .threads
            .iter()
            .find(|thread| thread.id == crew)
            .unwrap();
        assert_eq!(preserved.crew_captain, Some(0));
        assert_eq!(preserved.captain, Some(0));
    }
}
