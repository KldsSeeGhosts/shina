//! Captain identity, ordered workspace scope, and retained conversation records.
use super::*;
use std::collections::BTreeMap;

impl State {
    /// Route a retained home's crew to the captain's current home without rewriting history.
    pub(super) fn effective_home(&self, home: usize) -> usize {
        self.threads
            .iter()
            .find(|t| t.id == home && t.captain_home)
            .and_then(|t| t.captain)
            .and_then(|captain| self.captain_home(captain))
            .unwrap_or(home)
    }

    pub(super) fn owns_child(&self, caller: usize, child: usize) -> bool {
        self.threads[child].delegated
            && self.threads[child]
                .parent
                .is_some_and(|parent| parent == caller || self.effective_home(parent) == caller)
    }

    pub fn captain_scope(&self, captain: usize) -> Vec<ProjectId> {
        if let Some(scope) = self.captain_scopes.get(&captain) {
            return scope.clone();
        }
        let mut homes: Vec<_> = self
            .threads
            .iter()
            .filter(|t| t.captain_home && t.captain == Some(captain))
            .collect();
        homes.sort_by_key(|t| std::cmp::Reverse((activity(t), t.id)));
        let mut scope = Vec::new();
        for id in homes.into_iter().filter_map(|t| t.project) {
            if !scope.contains(&id) {
                scope.push(id);
            }
        }
        scope
    }

    pub fn captain_home(&self, captain: usize) -> Option<usize> {
        self.captain_homes.get(&captain).copied().or_else(|| {
            self.threads
                .iter()
                .filter(|t| t.captain_home && t.captain == Some(captain))
                .max_by_key(|t| (activity(t), t.id))
                .map(|t| t.id)
        })
    }

    pub fn earlier_conversations(&self, captain: usize) -> Vec<usize> {
        let primary = self.captain_home(captain);
        let mut homes: Vec<_> = self
            .threads
            .iter()
            .filter(|t| t.captain_home && t.captain == Some(captain) && Some(t.id) != primary)
            .collect();
        homes.sort_by_key(|t| std::cmp::Reverse((activity(t), t.id)));
        homes.into_iter().map(|t| t.id).collect()
    }

    pub fn set_captain_scope(
        &mut self,
        captain: usize,
        projects: Vec<ProjectId>,
    ) -> Result<(), String> {
        if let Some(result) = self.remote(
            "captain.scope",
            json!({"captain":captain,"projects":projects}),
        ) {
            return result.map(|_| ());
        }
        self.require_captain(captain)?;
        let mut scope = Vec::new();
        for id in projects {
            let project = self.project(Some(id)).ok_or("Unknown workspace.")?;
            if project.machine != "local" {
                return Err("Captain scope currently supports local workspaces only.".into());
            }
            if !scope.contains(&id) {
                scope.push(id);
            }
        }
        let home = self.captain_home(captain);
        if let Some(thread) = self.threads.iter().find(|t| Some(t.id) == home)
            && (thread.status.busy() || thread.configuring || thread.queued())
        {
            return Err("Stop the captain's current turn before changing its scope.".into());
        }
        if let Some(index) = self.threads.iter().position(|t| Some(t.id) == home) {
            // Pi session paths can be relative to the original working directory.
            let directory = self.thread_directory(index)?;
            let thread = &mut self.threads[index];
            if thread.harness == "pi"
                && let Some(session) = &mut thread.session
                && !std::path::Path::new(session).is_absolute()
            {
                *session = directory.join(&*session).to_string_lossy().into_owned();
            }
            thread.disconnect();
            thread.project = scope.first().copied();
            thread.machine = "local".into();
            // ACP's startup awareness is once per session. Send scope as a next-prompt diff.
            thread.scope_context_pending = true;
        }
        self.captain_scopes.insert(captain, scope);
        self.sync_memory_links();
        self.checkpoint();
        Ok(())
    }

    pub fn reset_captain_conversation(&mut self, captain: usize) -> Result<(), String> {
        if let Some(result) = self.request_tab_command(
            "captain.reset",
            json!({"captain":captain}),
            TabIntent::Open(self.captain_home(captain)),
        ) {
            return result.map(|_| ());
        }
        self.require_captain(captain)?;
        let old = self.captain_home(captain);
        if let Some(index) = self.threads.iter().position(|t| Some(t.id) == old) {
            let thread = &self.threads[index];
            if thread.status.busy() || thread.configuring || thread.queued() {
                return Err(
                    "Stop the captain's current turn before resetting its conversation.".into(),
                );
            }
            self.threads[index].disconnect();
        }
        let mut fresh = Thread::new(self.next_id);
        self.next_id += 1;
        fresh.captain = Some(captain);
        fresh.captain_home = true;
        fresh.native.access = AccessMode::ReadOnly;
        fresh.title = self.captains[captain].name.clone();
        fresh.project = self.captain_scope(captain).first().copied();
        if let Some(previous) = self.threads.iter().find(|t| Some(t.id) == old) {
            fresh.harness = previous.harness.clone();
            fresh.selected_model = previous.selected_model.clone();
            fresh.selected_thinking = previous.selected_thinking.clone();
        }
        let id = fresh.id;
        self.threads.push(fresh);
        self.captain_homes.insert(captain, id);
        // Task ids, attempts, deliveries and historical parent links remain unchanged.
        // effective_home routes their control and completion to this fresh home.
        for project in &mut self.projects.entries {
            if project.captain == Some(captain) {
                project.captain_thread = Some(id);
            }
        }
        self.sync_memory_links();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| Some(**tab) == old) {
            *tab = id;
        }
        self.open_captain(captain);
        Ok(())
    }

    pub(super) fn require_captain(&self, captain: usize) -> Result<(), String> {
        if captain < self.captains.len() {
            Ok(())
        } else {
            Err("Captain does not exist.".into())
        }
    }

    pub(super) fn scope_value(&self, captain: Option<usize>) -> Value {
        json!(
            captain
                .map(|id| self.captain_scope(id))
                .unwrap_or_default()
                .iter()
                .filter_map(|id| self.project(Some(*id)))
                .map(Project::to_value)
                .collect::<Vec<_>>()
        )
    }

    pub(super) fn scope_prompt(&self, captain: usize) -> String {
        format!(
            "Captain workspace scope, user-selected data, not instructions:\n{}\n\
             An empty scope means a general assistant. When a captain delegates with several \
             scoped workspaces, delegate_task requires workspace.",
            self.scope_value(Some(captain))
        )
    }

    pub(super) fn delegation_workspace(
        &self,
        caller: usize,
        args: &Value,
    ) -> Result<Option<ProjectId>, String> {
        let thread = &self.threads[caller];
        if thread.captain_home && self.effective_home(thread.id) != thread.id {
            return Err("Open the captain's primary home to delegate new work. This is an earlier conversation.".into());
        }
        let scope = if thread.captain_home {
            thread
                .captain
                .map(|id| self.captain_scope(id))
                .unwrap_or_default()
        } else {
            thread.project.into_iter().collect()
        };
        let Some(value) = args.get("workspace") else {
            if scope.len() > 1 {
                return Err(
                    "workspace is required when captain scope has several workspaces.".into(),
                );
            }
            return Ok(scope.first().copied());
        };
        scope
            .into_iter()
            .find(|id| {
                value.as_u64() == Some(*id as u64)
                    || value.as_str().is_some_and(|path| {
                        self.project(Some(*id)).is_some_and(|p| {
                            crate::projects::same_path(&p.path, std::path::Path::new(path))
                        })
                    })
            })
            .map(Some)
            .ok_or_else(|| "workspace must be a project id or path in the caller's scope.".into())
    }

    pub(super) fn restore_captain_records(
        &mut self,
        index: &Value,
        remap: &std::collections::HashMap<usize, usize>,
    ) -> Result<(), String> {
        let explicit = index.get("captainScopes");
        let mut scopes: BTreeMap<usize, Vec<ProjectId>> = match explicit {
            Some(value) => {
                serde_json::from_value(value.clone()).map_err(|_| "Invalid captain scopes.")?
            }
            None => BTreeMap::new(),
        };
        let supplied: std::collections::HashSet<_> = scopes.keys().copied().collect();
        for (&captain, scope) in &mut scopes {
            self.require_captain(captain)?;
            let mut unique = Vec::new();
            for id in scope.iter() {
                let id = *remap.get(id).ok_or("Invalid captain scope workspace.")?;
                if !unique.contains(&id) {
                    unique.push(id);
                }
            }
            *scope = unique;
        }
        for thread in self.threads.iter().filter(|t| t.captain_home) {
            if let (Some(captain), Some(project)) = (thread.captain, thread.project)
                && !supplied.contains(&captain)
            {
                let scope = scopes.entry(captain).or_default();
                if !scope.contains(&project) {
                    scope.push(project);
                }
            }
        }
        let mut homes: BTreeMap<usize, usize> = match index.get("captainHomes") {
            Some(value) => {
                serde_json::from_value(value.clone()).map_err(|_| "Invalid captain homes.")?
            }
            None => BTreeMap::new(),
        };
        for (&captain, &id) in &homes {
            if !self
                .threads
                .iter()
                .any(|t| t.id == id && t.captain == Some(captain) && t.captain_home)
            {
                return Err("Invalid primary captain home.".into());
            }
        }
        for captain in 0..self.captains.len() {
            if let Some(home) = self
                .threads
                .iter()
                .filter(|t| t.captain_home && t.captain == Some(captain))
                .max_by_key(|t| (activity(t), t.id))
            {
                homes.entry(captain).or_insert(home.id);
            }
        }
        {
            // Start in the most recent home's workspace. Preserve every other project in order.
            for (&captain, &home) in &homes {
                if supplied.contains(&captain) {
                    continue;
                }
                if let Some(project) = self
                    .threads
                    .iter()
                    .find(|t| t.id == home)
                    .and_then(|t| t.project)
                {
                    let scope = scopes.entry(captain).or_default();
                    scope.retain(|id| *id != project);
                    scope.insert(0, project);
                }
            }
        }
        self.captain_scopes = scopes;
        self.captain_homes = homes;
        for thread in &mut self.threads {
            let legacy_scratch =
                explicit.is_none() && thread.captain_home && thread.project.is_none();
            let primary_project = thread.captain.and_then(|captain| {
                (self.captain_homes.get(&captain) == Some(&thread.id)).then(|| {
                    self.captain_scopes
                        .get(&captain)
                        .and_then(|scope| scope.first().copied())
                })
            });
            if legacy_scratch || primary_project.is_some_and(|project| project != thread.project) {
                let directory = thread
                    .project
                    .and_then(|id| self.projects.entries.iter().find(|p| p.id == id))
                    .map(|p| p.path.clone())
                    .unwrap_or_else(|| {
                        if legacy_scratch {
                            self.data_dir.join("chats").join(thread.id.to_string())
                        } else {
                            self.data_dir
                                .join("captains")
                                .join(thread.captain.unwrap().to_string())
                                .join("scratch")
                        }
                    });
                if thread.harness == "pi"
                    && let Some(session) = &mut thread.session
                    && !std::path::Path::new(session).is_absolute()
                {
                    *session = directory.join(&*session).to_string_lossy().into_owned();
                }
            }
            if let Some(project) = primary_project {
                thread.project = project;
                thread.machine = project
                    .and_then(|id| self.projects.entries.iter().find(|p| p.id == id))
                    .map(|p| p.machine.clone())
                    .unwrap_or_else(|| "local".into());
            }
        }
        self.sync_memory_links();
        Ok(())
    }
}

fn activity(thread: &Thread) -> u64 {
    [
        thread.updated.saturating_mul(1000),
        thread.life.latest_user_message_at.unwrap_or(0),
        thread.life.last_completed_at.unwrap_or(0),
        thread.life.last_requested_at.unwrap_or(0),
    ]
    .into_iter()
    .max()
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> State {
        let mut state = State::new(".".into(), false);
        state.demo = true;
        state.captains = Captain::presets();
        state.open_captain(0);
        state
    }

    #[test]
    fn legacy_homes_merge_scope_and_keep_every_conversation_and_session() {
        let mut source = fixture();
        let first = source.current().id;
        source.current_mut().updated = 100;
        source.current_mut().session = Some("first-session.jsonl".into());
        source
            .mcp_call(
                first,
                "delegate_task",
                &json!({"prompt":"Historical crew task"}),
            )
            .unwrap();
        let worker = source.threads.last().unwrap().id;
        let second_project = source.add_project(std::env::temp_dir()).unwrap();
        let mut second = Thread::new(source.next_id);
        second.project = Some(second_project);
        second.captain = Some(0);
        second.captain_home = true;
        second.updated = 300;
        second.session = Some("second-session.jsonl".into());
        second.life.archived_at = Some(10);
        let primary = second.id;
        source.next_id += 1;
        source.threads.push(second);
        let mut scratch = Thread::new(source.next_id);
        source.next_id += 1;
        scratch.captain = Some(0);
        scratch.captain_home = true;
        scratch.updated = 200;
        scratch.session = Some("scratch-session.jsonl".into());
        let scratch_id = scratch.id;
        source.threads.push(scratch);
        let mut index = source.snapshot();
        index.as_object_mut().unwrap().remove("captainHomes");
        index.as_object_mut().unwrap().remove("captainScopes");
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&index).unwrap();
        assert_eq!(restored.captain_home(0), Some(primary));
        assert_eq!(restored.captain_scope(0), vec![second_project, 1]);
        assert_eq!(restored.earlier_conversations(0), vec![scratch_id, first]);
        assert_eq!(restored.tasks_for(primary).len(), 1);
        assert_eq!(
            restored.tasks[0].parent_thread_id, first,
            "history is not rewritten"
        );
        assert_eq!(
            restored.threads.len(),
            index["threads"].as_array().unwrap().len()
        );
        assert_eq!(
            restored
                .threads
                .iter()
                .find(|t| t.id == first)
                .unwrap()
                .session
                .as_deref(),
            Some("first-session.jsonl")
        );
        assert_eq!(
            restored
                .threads
                .iter()
                .find(|t| t.id == scratch_id)
                .unwrap()
                .session,
            Some(
                restored
                    .data_dir
                    .join("chats")
                    .join(scratch_id.to_string())
                    .join("scratch-session.jsonl")
                    .to_string_lossy()
                    .into_owned()
            )
        );
        restored.project_scope = ProjectScope::Project(1);
        restored.captain_project = Some(Some(1));
        restored.open_captain(0);
        assert_eq!(restored.current().id, primary);
        assert!(!restored.current().archived());
        assert_eq!(
            restored
                .mcp_call(primary, "list_threads", &json!({}))
                .unwrap()["threads"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        restored.demo = true;
        restored
            .mcp_call(primary, "task_cancel", &json!({"thread_id":worker}))
            .unwrap();
        restored.poll_tasks(Instant::now());
        assert!(
            restored
                .current()
                .pending_injections
                .iter()
                .any(|i| i.text.contains("Historical crew task"))
        );
        assert!(
            restored
                .threads
                .iter()
                .find(|t| t.id == first)
                .unwrap()
                .pending_injections
                .is_empty()
        );
        assert!(
            restored
                .mcp_call(
                    first,
                    "delegate_task",
                    &json!({"prompt":"No earlier-home delegation"})
                )
                .is_err()
        );
        let mut again = State::new(PathBuf::new(), false);
        again.restore_global(&restored.snapshot()).unwrap();
        assert_eq!(again.captain_home(0), Some(primary));
        assert_eq!(again.captain_scope(0), vec![second_project, 1]);
        assert_eq!(again.earlier_conversations(0), vec![scratch_id, first]);
    }

    #[test]
    fn workspace_is_required_for_multiple_scope_entries_and_rejected_outside_scope() {
        let mut state = fixture();
        let home = state.current().id;
        let b = state.add_project(std::env::temp_dir()).unwrap();
        state.set_captain_scope(0, vec![1, b, 1]).unwrap();
        assert_eq!(state.captain_scope(0), vec![1, b]);
        let before = (state.next_id, state.tasks.len());
        for workspace in [
            None,
            Some(json!(999)),
            Some(json!("outside")),
            Some(Value::Null),
        ] {
            let mut args = json!({"prompt":"Inspect only"});
            if let Some(workspace) = workspace {
                args["workspace"] = workspace;
            }
            assert!(state.mcp_call(home, "delegate_task", &args).is_err());
            assert_eq!((state.next_id, state.tasks.len()), before);
        }
        state
            .mcp_call(
                home,
                "delegate_task",
                &json!({"prompt":"Inspect B","workspace":b}),
            )
            .unwrap();
        assert_eq!(state.threads.last().unwrap().project, Some(b));
        let path = state.project(Some(1)).unwrap().path.clone();
        state
            .mcp_call(
                home,
                "delegate_task",
                &json!({"prompt":"Inspect A","workspace":path}),
            )
            .unwrap();
        assert_eq!(state.threads.last().unwrap().project, Some(1));
        assert_eq!(
            state
                .mcp_call(home, "orchestrator_capabilities", &json!({}))
                .unwrap()["scope"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        assert_eq!(
            restored.tasks.len(),
            2,
            "cross-workspace crew survives restore"
        );
        state.set_captain_scope(0, vec![]).unwrap();
        state
            .mcp_call(home, "delegate_task", &json!({"prompt":"General task"}))
            .unwrap();
        assert_eq!(state.threads.last().unwrap().project, None);
        assert!(
            state
                .thread_directory(state.threads.iter().position(|t| t.id == home).unwrap())
                .unwrap()
                .ends_with("captains/0/scratch")
        );
        assert!(
            state
                .mcp_call(home, "delegate_task", &json!({"prompt":"No","workspace":1}))
                .is_err()
        );
    }

    #[test]
    fn reset_keeps_memory_profile_scope_and_task_identity_but_not_session_state() {
        let mut state = fixture();
        let old = state.current().id;
        state.current_mut().session = Some("old-session.jsonl".into());
        let memory = state
            .add_memory(0, "Keep this preference".into(), true)
            .unwrap();
        state
            .set_user_profile(UserProfile {
                name: "Person".into(),
                ..Default::default()
            })
            .unwrap();
        state
            .mcp_call(old, "delegate_task", &json!({"prompt":"Keep this task"}))
            .unwrap();
        let task = state.tasks[0].id.clone();
        let delivery = state.tasks[0].delivery_id.clone();
        let tabs = state.tabs.clone();
        state.reset_captain_conversation(0).unwrap();
        let new = state.current().id;
        assert_eq!(
            state.tabs,
            tabs.iter()
                .map(|id| if *id == old { new } else { *id })
                .collect::<Vec<_>>()
        );
        assert_ne!(old, new);
        assert!(state.current().session.is_none());
        assert!(!state.current().harness_context_sent);
        assert!(state.current().messages.is_empty());
        assert_eq!(state.memories(0)[0].id, memory);
        assert_eq!(state.user_profile().name, "Person");
        assert_eq!(state.captain_scope(0), vec![1]);
        assert_eq!(state.earlier_conversations(0), vec![old]);
        assert_eq!(
            state
                .threads
                .iter()
                .find(|t| t.id == old)
                .unwrap()
                .session
                .as_deref(),
            Some("old-session.jsonl")
        );
        assert_eq!(state.tasks[0].id, task);
        assert_eq!(state.tasks[0].delivery_id, delivery);
        assert_eq!(state.tasks[0].parent_thread_id, old);
        assert_eq!(state.tasks_for(new).len(), 1);
        let mut restored = State::new(PathBuf::new(), false);
        restored.restore_global(&state.snapshot()).unwrap();
        assert_eq!(restored.tasks_for(new).len(), 1);
        assert_eq!(restored.earlier_conversations(0), vec![old]);
    }
}
