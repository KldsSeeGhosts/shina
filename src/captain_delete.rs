//! Confirmed profile deletion removes Shina-owned private records atomically.
//! Conversation files, provider data, worktrees and backups are not erased.
use super::*;
use std::collections::BTreeSet;

impl State {
    pub(super) fn validate_captain_deleted_storage(&self) -> Result<(), String> {
        for (&captain, record) in &self.captain_lifecycles {
            if record.phase == CaptainPhase::Deleted
                && (!self.memories(captain).is_empty()
                    || self
                        .background
                        .routines
                        .entries()
                        .iter()
                        .any(|routine| routine.captain == captain))
            {
                return Err(
                    "A deleted Captain still has curated memory or routine records.".into(),
                );
            }
        }
        Ok(())
    }

    fn captain_delete_homes(&self, captain: usize) -> BTreeSet<usize> {
        self.threads
            .iter()
            .filter(|thread| thread.captain_home && thread.captain == Some(captain))
            .map(|thread| thread.id)
            .collect()
    }

    fn captain_home_retention_reason(&self, homes: &BTreeSet<usize>) -> Option<&'static str> {
        if self.tasks.iter().any(|task| {
            homes.contains(&task.parent_thread_id) || homes.contains(&task.child_thread_id)
        }) || self.threads.iter().any(|thread| {
            !homes.contains(&thread.id)
                && thread.parent.is_some_and(|parent| homes.contains(&parent))
        }) {
            return Some("Keep conversations to preserve existing Task and parent references.");
        }
        if self
            .captain_reports
            .iter()
            .chain(
                self.threads
                    .iter()
                    .flat_map(|thread| &thread.crew_report_outbox),
            )
            .any(|report| homes.contains(&report.thread_id))
            || self
                .captain_question_answers
                .iter()
                .any(|answer| homes.contains(&answer.thread_id))
        {
            return Some(
                "Keep conversations to preserve historical crew report and answer references.",
            );
        }
        None
    }

    pub(super) fn captain_delete_preview(&self, captain: usize) -> Result<Value, String> {
        self.require_captain(captain)?;
        let homes = self.captain_delete_homes(captain);
        let mut affected = self.captain_group_ids(captain);
        affected.extend(&homes);
        affected.extend(
            self.threads
                .iter()
                .filter(|thread| thread.captain == Some(captain))
                .map(|thread| thread.id),
        );
        let mut blockers = self.captain_settlement_blockers(captain);
        if let Err(error) = self.captain_memory_delete_ready() {
            blockers.push(error);
        }
        if self.captain_phase(captain) == CaptainPhase::Archived
            && let Err(error) = self
                .validate_captain_archives()
                .and_then(|_| self.validate_captain_archive_routines())
        {
            blockers.push(error);
        }
        let crew: BTreeSet<_> = self
            .threads
            .iter()
            .filter(|thread| thread.crew_captain == Some(captain))
            .map(|thread| thread.id)
            .collect();
        let reason = if self.captain_phase(captain) == CaptainPhase::Retired {
            Some(
                "A retired Captain no longer owns its former conversations. Delete those ordinary threads separately.",
            )
        } else {
            self.captain_home_retention_reason(&homes)
        };
        Ok(
            json!({"captain":captain,"action":"delete","captainVersion":self.captain_version(captain)?,
            "crewSeen":crew,"homeThreads":homes,"affectedThreads":affected,"blockers":blockers,
            "canDeleteConversations":reason.is_none(),"keepConversationsRequiredReason":reason,
            "externalMemoryManagedSeparately":self.memory_backend_status() != MemoryBackendStatus::Disabled}),
        )
    }

    pub fn captain_delete(
        &mut self,
        captain: usize,
        expected_version: &str,
        crew_seen: &[usize],
        confirm_name: &str,
        keep_conversations: bool,
    ) -> Result<Value, String> {
        let preview = self.captain_delete_preview(captain)?;
        if preview["captainVersion"] != expected_version || preview["crewSeen"] != json!(crew_seen)
        {
            return Err(
                "Captain changed since version. Review a fresh preview before confirming.".into(),
            );
        }
        if confirm_name != self.captains[captain].name {
            return Err("Type the Captain's exact current name to confirm deletion.".into());
        }
        if let Some(blocker) = preview["blockers"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(Value::as_str)
        {
            return Err(blocker.into());
        }
        if !keep_conversations
            && let Some(reason) = preview["keepConversationsRequiredReason"].as_str()
        {
            return Err(reason.into());
        }
        let homes = self.captain_delete_homes(captain);
        let indices: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, thread)| homes.contains(&thread.id))
            .map(|(index, _)| index)
            .collect();
        let directories = indices
            .iter()
            .map(|index| self.prepare_role_directory(*index))
            .collect::<Result<Vec<_>, _>>()?;
        let database = crate::database::for_path(&self.data_dir.join("index.json"))
            .ok_or("This Captain change requires the durable server.")?;
        database.require_batch()?;
        // If either staged store fails, poison the batch. A rejected receipt
        // must never commit only half of a multi-record deletion.
        if let Err(error) = self
            .background
            .routines
            .remove_captain(captain)
            .and_then(|_| self.remove_captain_curated_memory(captain))
        {
            database.fail(error.clone());
            self.storage_error = Some(error.clone());
            return Err(error);
        }
        let archived = self
            .captain_lifecycles
            .get(&captain)
            .filter(|record| record.phase == CaptainPhase::Archived)
            .cloned();
        if let Some(record) = archived {
            for thread in &mut self.threads {
                if let Some(previous) = record.archived_threads.get(&thread.id) {
                    thread.life.archived_at = *previous;
                    thread.prewarm_attempted = true;
                }
            }
        }
        let at = lifecycle::now();
        for (index, (scratch, session)) in indices.into_iter().zip(directories) {
            let thread = &mut self.threads[index];
            thread.scratch_directory = scratch;
            thread.session = session;
            thread.crew_report_run = None;
            thread.disconnect();
            thread.prewarm_attempted = true;
            thread.captain_home = false;
            thread.life.archived_at.get_or_insert(at);
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
            self.invalidate_undo(self.threads[index].id);
        }
        for thread in &mut self.threads {
            if thread.captain == Some(captain) {
                thread.captain = None;
            }
        }
        for project in &mut self.projects.entries {
            if project.captain == Some(captain) {
                project.captain = None;
                project.captain_thread = None;
            }
        }
        self.captain_homes.remove(&captain);
        self.captain_books.remove(&captain);
        self.captain_scopes.remove(&captain);
        self.captain_remote_scopes.remove(&captain);
        self.captain_report_policies.remove(&captain);
        self.captain_recovery_modes.remove(&captain);
        self.close_captain_inbox(captain);
        self.captains[captain] = Captain {
            name: format!("Deleted Captain {captain}"),
            role: String::new(),
            instructions: String::new(),
            portrait: 0,
        };
        self.captain_lifecycles.insert(
            captain,
            captain_lifecycle::CaptainLifecycle {
                phase: CaptainPhase::Deleted,
                ..Default::default()
            },
        );
        self.sync_memory_links();
        self.refresh_captain_contexts();
        if !keep_conversations {
            // This exact set has no surviving Task, parent, report or answer
            // references. Never call recursive deletion_ids here.
            self.remove_threads(&homes.iter().copied().collect());
        }
        self.persistence_scope = None;
        Ok(
            json!({"captain":captain,"lifecycle":"deleted","keptConversations":keep_conversations,
            "captainVersion":crate::protocol::revision(&json!({"captain":captain,"lifecycle":"deleted"}))}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Command;
    use crate::runtime::RuntimeState;

    fn fixture() -> RuntimeState {
        let directory = std::env::temp_dir().join(format!("shina-delete-{}", uuid::Uuid::new_v4()));
        let mut runtime = RuntimeState::open(directory, Arc::new(|| {})).unwrap();
        runtime
            .database
            .as_ref()
            .unwrap()
            .issue_device("maintainer", "fixture-token")
            .unwrap();
        runtime.state.captains = Captain::presets();
        runtime.state.open_captain(0);
        runtime.state.test_connect_pi = Some(Box::new(|_, _| {
            let (session, _, _) = crate::harness::Session::test_connection();
            Some(session)
        }));
        for (name, text) in [("AGENTS.md", "Private rules"), ("NOTES.md", "Private plan")] {
            let receipt = runtime
                .command(
                    "maintainer",
                    Command::new(
                        "captain.document.write",
                        json!({
                            "captain":0,"name":name,"text":text,"expectedRevision":0
                        }),
                    ),
                )
                .unwrap();
            assert_eq!(receipt["status"], "accepted", "{receipt}");
        }
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "memory.put",
                    json!({
                        "captain":0,"text":"Private curated memory","essential":true
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let home = runtime.state.captain_home(0).unwrap();
        runtime
            .state
            .background
            .routines
            .create(
                0,
                home,
                "Review".into(),
                "Read status".into(),
                crate::routines::Schedule::Every { minutes: 15 },
                crate::routines::MissedPolicy::Skip,
                chrono::Local::now(),
            )
            .unwrap();
        runtime
    }

    fn worker(runtime: &mut RuntimeState, archived: Option<u64>) -> usize {
        let id = runtime.state.next_id;
        runtime.state.next_id += 1;
        let mut worker = Thread::new(id);
        worker.crew_captain = Some(0);
        worker.captain = Some(0);
        worker.life.archived_at = archived;
        runtime.state.threads.push(worker);
        id
    }

    fn deletion(state: &State, keep: bool) -> Command {
        let preview = state.captain_lifecycle_preview(0, "delete").unwrap();
        Command::new(
            "captain.delete",
            json!({
                "captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"],
                "confirmName":state.captains[0].name,"keepConversations":keep
            }),
        )
    }

    #[test]
    fn captain_delete_keeps_history_ids_and_removes_only_its_private_records_across_restart() {
        let mut runtime = fixture();
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "memory.put",
                    json!({
                        "captain":1,"text":"Another Captain's memory","essential":true
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted");
        let home = runtime.state.captain_home(0).unwrap();
        let source = worker(&mut runtime, None);
        let child_id = runtime.state.next_id;
        runtime.state.next_id += 1;
        let mut child = Thread::new(child_id);
        child.parent = Some(home);
        child.delegated = true;
        child.crew_captain = Some(0);
        child.captain = Some(0);
        let mut task = tasks::Task::new(&child, "Existing task", None);
        task.finish(TaskState::Completed, "Existing result", "");
        task.delivered = true;
        runtime.state.tasks.push(task);
        runtime.state.threads.push(child);
        let report: crew_reports::CrewReport = serde_json::from_value(json!({
            "id":format!("crew-{}", uuid::Uuid::new_v4()),"captain":0,
            "threadId":source,"runId":"run-a","title":"Worker","outcome":"completed",
            "summary":"Result","error":"","createdAt":1,"delivered":true
        }))
        .unwrap();
        runtime.state.captain_reports.push(report);
        let task_record = serde_json::to_value(&runtime.state.tasks[0]).unwrap();
        let report_record = serde_json::to_value(&runtime.state.captain_reports[0]).unwrap();
        let sibling = runtime.state.captains[1].clone();
        let rejected = runtime
            .command("maintainer", deletion(&runtime.state, false))
            .unwrap();
        assert_eq!(rejected["status"], "rejected");
        assert_eq!(runtime.state.captain_phase(0), CaptainPhase::Active);
        let mut command = deletion(&runtime.state, true);
        command
            .params
            .as_object_mut()
            .unwrap()
            .remove("keepConversations");
        let receipt = runtime.command("maintainer", command.clone()).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert_eq!(runtime.state.captains.len(), 4);
        assert_eq!(runtime.state.captains[1], sibling);
        assert_eq!(runtime.state.captain_phase(0), CaptainPhase::Deleted);
        assert!(runtime.state.memories(0).is_empty());
        assert_eq!(runtime.state.memories(1).len(), 1);
        assert!(runtime.state.background.routines.entries().is_empty());
        let home_thread = runtime
            .state
            .threads
            .iter()
            .find(|thread| thread.id == home)
            .unwrap();
        assert!(home_thread.archived());
        assert!(!home_thread.captain_home);
        assert!(
            runtime
                .state
                .threads
                .iter()
                .all(|thread| thread.crew_captain.is_none() && thread.captain != Some(0))
        );
        assert_eq!(
            serde_json::to_value(&runtime.state.tasks[0]).unwrap(),
            task_record
        );
        assert_eq!(
            serde_json::to_value(&runtime.state.captain_reports[0]).unwrap(),
            report_record
        );
        let list = runtime
            .state
            .mcp_call(home, "list_captains", &json!({}))
            .unwrap();
        assert!(
            list["captains"]
                .as_array()
                .unwrap()
                .iter()
                .all(|captain| captain["captain_id"] != 0)
        );
        assert!(runtime.state.captain_read_value(0, true).is_err());
        let database = runtime.database.clone().unwrap();
        let index = database.read("index.json").unwrap().unwrap();
        assert!(index["captainBooks"].get("0").is_none());
        assert!(index["captainScopes"].get("0").is_none());
        assert_eq!(index["captains"][0]["instructions"], "");
        assert_eq!(
            runtime.command("maintainer", command.clone()).unwrap(),
            receipt
        );
        let directory = runtime.state.data_dir.clone();
        drop(runtime);
        drop(database);
        let mut reopened = RuntimeState::open(directory, Arc::new(|| {})).unwrap();
        assert_eq!(reopened.state.captain_phase(0), CaptainPhase::Deleted);
        assert_eq!(
            serde_json::to_value(&reopened.state.tasks[0]).unwrap(),
            task_record
        );
        assert_eq!(
            serde_json::to_value(&reopened.state.captain_reports[0]).unwrap(),
            report_record
        );
        assert_eq!(reopened.command("maintainer", command).unwrap(), receipt);
    }

    #[test]
    fn captain_delete_from_archive_restores_crew_markers_and_never_removes_worktree_files() {
        let mut runtime = fixture();
        let home = runtime.state.captain_home(0).unwrap();
        let live = worker(&mut runtime, None);
        let prior = worker(&mut runtime, Some(17));
        let path = runtime.state.data_dir.join("crew-worktree");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("keep.txt"), "Keep this file").unwrap();
        runtime
            .state
            .threads
            .iter_mut()
            .find(|thread| thread.id == live)
            .unwrap()
            .worktree_path = Some(path.to_string_lossy().into_owned());
        let preview = runtime
            .state
            .captain_lifecycle_preview(0, "archive")
            .unwrap();
        let receipt = runtime.command("maintainer", Command::new("captain.archive", json!({
            "captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":preview["crewSeen"]
        }))).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let command = deletion(&runtime.state, false);
        let receipt = runtime.command("maintainer", command).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert!(!runtime.state.threads.iter().any(|thread| thread.id == home));
        assert!(
            !runtime
                .state
                .threads
                .iter()
                .find(|thread| thread.id == live)
                .unwrap()
                .archived()
        );
        assert_eq!(
            runtime
                .state
                .threads
                .iter()
                .find(|thread| thread.id == prior)
                .unwrap()
                .life
                .archived_at,
            Some(17)
        );
        assert_eq!(
            std::fs::read_to_string(path.join("keep.txt")).unwrap(),
            "Keep this file"
        );
        assert!(
            runtime
                .state
                .threads
                .iter()
                .all(|thread| thread.pi.is_none())
        );
    }

    #[test]
    fn captain_delete_checks_exact_name_current_version_and_unanswered_human_questions() {
        let mut runtime = fixture();
        let mut command = deletion(&runtime.state, true);
        command.params["confirmName"] = json!(format!("{} ", runtime.state.captains[0].name));
        assert_eq!(
            runtime.command("maintainer", command).unwrap()["status"],
            "rejected"
        );
        let command = deletion(&runtime.state, true);
        worker(&mut runtime, None);
        assert_eq!(
            runtime.command("maintainer", command).unwrap()["status"],
            "rejected"
        );
        assert_eq!(runtime.state.captain_phase(0), CaptainPhase::Active);
        assert_eq!(runtime.state.memories(0).len(), 1);
        let source = runtime.state.threads.last().unwrap().id;
        runtime.state.captain_reports.push(
            serde_json::from_value(json!({
                "id":format!("crew-{}", uuid::Uuid::new_v4()),"captain":0,
                "threadId":source,"runId":"run-a","title":"Worker","outcome":"blocked",
                "summary":"Need a choice","error":"","createdAt":1,"delivered":true,
                "requestId":uuid::Uuid::new_v4(),"question":{"text":"Proceed?","recipient":"user"}
            }))
            .unwrap(),
        );
        let preview = runtime
            .state
            .captain_lifecycle_preview(0, "delete")
            .unwrap();
        assert!(!preview["blockers"].as_array().unwrap().is_empty());
        let receipt = runtime
            .command("maintainer", deletion(&runtime.state, true))
            .unwrap();
        assert_eq!(receipt["status"], "rejected");
        assert_eq!(runtime.state.captain_phase(0), CaptainPhase::Active);
    }

    #[test]
    fn captain_delete_can_remove_an_unopened_profile_without_changing_the_current_home() {
        let mut runtime = fixture();
        let home = runtime.state.current().id;
        assert!(runtime.state.captain_home(1).is_none());
        let preview = runtime
            .state
            .captain_lifecycle_preview(1, "delete")
            .unwrap();
        assert_eq!(preview["blockers"], json!([]));
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "captain.delete",
                    json!({
                        "captain":1,"expectedVersion":preview["captainVersion"],"crewSeen":[],
                        "confirmName":runtime.state.captains[1].name
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert_eq!(runtime.state.current().id, home);
        assert_eq!(runtime.state.captain_home(0), Some(home));
        assert_eq!(runtime.state.captain_phase(0), CaptainPhase::Active);
        assert_eq!(runtime.state.captain_phase(1), CaptainPhase::Deleted);
        assert_eq!(runtime.state.memories(0).len(), 1);
        assert_eq!(runtime.state.background.routines.entries().len(), 1);
    }

    #[test]
    fn captain_delete_prevents_old_thread_undo_from_reattaching_deleted_crew() {
        let mut runtime = fixture();
        let source = worker(&mut runtime, None);
        runtime
            .state
            .delete_with_undo(std::collections::HashSet::from([source]), Action::Delete);
        assert!(!runtime.state.deleted_threads.is_empty());
        let receipt = runtime
            .command("maintainer", deletion(&runtime.state, true))
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "thread.undo",
                    json!({
                        "expected":runtime.state.undo
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "rejected", "{receipt}");
        assert!(
            !runtime
                .state
                .threads
                .iter()
                .any(|thread| thread.id == source)
        );
        assert_eq!(runtime.state.captain_phase(0), CaptainPhase::Deleted);
    }

    #[test]
    fn captain_delete_of_a_retired_profile_does_not_stop_or_remove_its_former_home() {
        let mut runtime = fixture();
        let home = runtime.state.captain_home(0).unwrap();
        let preview = runtime
            .state
            .captain_lifecycle_preview(0, "demote")
            .unwrap();
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "captain.demote",
                    json!({
                        "captain":0,"expectedVersion":preview["captainVersion"],"crewSeen":[]
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == home)
            .unwrap();
        let (session, _commands, _events) = crate::harness::Session::test_connection();
        runtime.state.threads[index].pi = Some(session);
        runtime.state.threads[index].status = Status::Running;
        let receipt = runtime
            .command("maintainer", deletion(&runtime.state, false))
            .unwrap();
        assert_eq!(receipt["status"], "rejected");
        let receipt = runtime
            .command("maintainer", deletion(&runtime.state, true))
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert_eq!(runtime.state.threads[index].status, Status::Running);
        assert!(runtime.state.threads[index].pi.is_some());
        assert!(!runtime.state.threads[index].archived());
        assert_eq!(runtime.state.captain_phase(0), CaptainPhase::Deleted);
    }

    #[test]
    fn captain_delete_sql_failure_rolls_back_index_memory_and_routines_and_quarantines_runtime() {
        let mut runtime = fixture();
        let database = runtime.database.clone().unwrap();
        let before_index = database.read("index.json").unwrap().unwrap();
        let before_memory = database.read("memory.json").unwrap().unwrap();
        let before_routines = database.read("routines.json").unwrap().unwrap();
        database.test_connection(|connection| connection.execute_batch(
            "CREATE TRIGGER fail_delete_memory BEFORE INSERT ON documents
             WHEN NEW.name='memory.json' BEGIN SELECT RAISE(ABORT,'injected deletion failure'); END;"
        ).unwrap());
        let command = deletion(&runtime.state, true);
        assert!(runtime.command("maintainer", command).is_err());
        assert!(
            database.read("index.json").is_err(),
            "failure must remain quarantined"
        );
        database.test_connection(|connection| {
            assert_eq!(
                crate::projection_store::read(connection, "index.json")
                    .unwrap()
                    .unwrap(),
                before_index
            );
            for (name, expected) in [
                ("memory.json", before_memory),
                ("routines.json", before_routines),
            ] {
                let payload: String = connection
                    .query_row(
                        "SELECT payload FROM documents WHERE name=?1",
                        [name],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&payload).unwrap(),
                    expected,
                    "{name}"
                );
            }
        });
    }
}
