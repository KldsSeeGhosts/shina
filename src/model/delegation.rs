use super::*;

impl State {
    /// Models Shina can validate against, preferring the live catalog over the caller's last
    /// handshake metadata. The string records which source was used.
    pub(super) fn delegation_models(&self, caller: &Thread) -> (Vec<AgentModel>, &'static str) {
        if let Some(models) = self.workspace_delegation_models(caller) {
            return (models, "workspace");
        }
        if caller.harness != "pi" {
            let models = self
                .threads
                .iter()
                .position(|t| t.id == caller.id)
                .map(|index| self.harness_models_for_thread(index, &caller.harness))
                .unwrap_or_default();
            if !models.is_empty() {
                return (models, "caller");
            }
            if !caller.models.is_empty() {
                return (caller.models.clone(), "caller");
            }
            return (models, "caller");
        }
        let mut models = self.catalog.models.clone();
        if let Some(default) = &self.catalog.default_model
            && !models
                .iter()
                .any(|model| model.provider == default.provider && model.id == default.id)
        {
            models.push(default.clone());
        }
        if !models.is_empty() {
            return (models, "catalog");
        }
        if !caller.models.is_empty() {
            return (caller.models.clone(), "caller");
        }
        (models, "none")
    }

    /// KSG `withWorkspaceModels`: the caller folder's own provider config can
    /// add models that only its workspace snapshot lists. A recorded snapshot
    /// for that folder replaces the machine-wide list, never merges into it.
    pub(super) fn workspace_delegation_models(&self, caller: &Thread) -> Option<Vec<AgentModel>> {
        let index = self.threads.iter().position(|t| t.id == caller.id)?;
        self.workspace_models(index, &caller.harness)
    }

    /// The workspace snapshot for `harness` under the caller's folder, when one
    /// was recorded.
    pub(super) fn workspace_models(
        &self,
        caller_index: usize,
        harness: &str,
    ) -> Option<Vec<AgentModel>> {
        if harness == "pi" {
            return None;
        }
        let cwd = self.thread_directory(caller_index).ok()?;
        self.native_catalog(harness)
            .and_then(|catalog| catalog.workspace_snapshots.get(&cwd).cloned())
    }

    /// KSG `scanWorkspace`: probe the caller folder's provider config in the
    /// background when no workspace snapshot exists yet. A failed or stale scan
    /// retries after the catalog's own backoff, never twice in flight.
    pub(super) fn scan_workspace_catalog(&mut self, harness: &str, cwd: &Path) {
        if self.demo || !self.runtime_enabled || self.client.is_some() || !cwd.is_dir() {
            return;
        }
        let key = (harness.to_owned(), cwd.to_path_buf());
        if let Some(scan) = self.workspace_model_scans.get(&key) {
            if scan.loading() || !scan.retry_ready() {
                return;
            }
        }
        let wake = self.wake.clone();
        let mut catalog = crate::session::NativeCatalog::default();
        let configured = self
            .harnesses
            .agents
            .iter()
            .find(|agent| agent.id == harness);
        let started = if let Some(agent) = configured {
            catalog.refresh_acp(agent, cwd, wake, false, false)
        } else {
            match harness {
                "codex" | "claude" | "antigravity" | "cursor" => {
                    self.builtin_cli_available(harness)
                        && catalog.refresh(harness, cwd, wake, false, false)
                }
                _ => false,
            }
        };
        if started {
            self.workspace_model_scans.insert(key, catalog);
        }
    }

    /// KSG `OrchestratorMcpService.capabilities`/`delegate` unscanned-provider
    /// pass: kick a background scan for the caller's folder when its snapshot
    /// is missing.
    pub(super) fn scan_caller_workspace(&mut self, caller_index: usize, harness: &str) {
        if harness == "pi" {
            return;
        }
        let Ok(cwd) = self.thread_directory(caller_index) else {
            return;
        };
        let missing = self
            .native_catalog(harness)
            .is_none_or(|catalog| !catalog.workspace_snapshots.contains_key(&cwd));
        if missing {
            self.scan_workspace_catalog(harness, &cwd);
        }
    }

    pub(super) fn default_selection(&self) -> Option<Selection> {
        let model = self.catalog.default_model.as_ref()?;
        let reasoning = (!self.catalog.default_thinking.trim().is_empty()
            && model
                .thinking_levels
                .iter()
                .any(|level| level == &self.catalog.default_thinking))
        .then(|| self.catalog.default_thinking.clone());
        Some(Selection {
            provider: model.provider.clone(),
            model: model.id.clone(),
            reasoning,
        })
    }

    pub(super) fn inherited_selection(&self, caller_index: usize) -> Option<Selection> {
        let caller = self.threads.get(caller_index)?;
        caller.inherited_selection().or_else(|| {
            (caller.harness == "pi")
                .then(|| self.default_selection())
                .flatten()
        })
    }

    pub(super) fn discovery_state(&self, source: &str) -> &'static str {
        if source == "caller" {
            return "caller_metadata";
        }
        let has_catalog = !self.catalog.models.is_empty() || self.catalog.default_model.is_some();
        if self.catalog.error.is_some() {
            return if has_catalog { "stale" } else { "failed" };
        }
        if self.catalog.remaining().is_some() {
            return if has_catalog { "refreshing" } else { "loading" };
        }
        if source == "none" {
            "not_started"
        } else {
            "ready"
        }
    }

    pub(super) fn validate_delegation_selection(
        &self,
        models: &[AgentModel],
        selection: &Selection,
        explicit: bool,
    ) -> Result<(), String> {
        let provider_name = if selection.provider == "cpa" {
            "CPA"
        } else {
            &selection.provider
        };
        if models.is_empty() {
            return if explicit {
                Err("Model discovery is not ready, so Shina cannot validate the requested target. Call orchestrator_capabilities and retry when ready is true.".into())
            } else {
                Ok(())
            };
        }
        let Some(model) =
            advertised_delegation_model(models, selection.provider.as_str(), &selection.model)
        else {
            let available = models
                .iter()
                .take(6)
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(if explicit {
                format!(
                    "The requested {provider_name} model {} is not in the current catalog. Available model ids include: {available}. Call orchestrator_capabilities for the full list.",
                    selection.model
                )
            } else {
                format!(
                    "The inherited {provider_name} model {} is not in the current catalog. Pass target with an available model from orchestrator_capabilities.",
                    selection.model
                )
            });
        };
        if let Some(reasoning) = &selection.reasoning
            && !model.thinking_levels.iter().any(|level| level == reasoning)
        {
            let supported = if model.thinking_levels.is_empty() {
                "none".to_owned()
            } else {
                model.thinking_levels.join(", ")
            };
            return Err(format!(
                "Reasoning level {reasoning} is not supported by {provider_name} model {}. Supported levels: {supported}. Pass an exact level; Shina will not substitute or clamp one.",
                model.id
            ));
        }
        Ok(())
    }

    pub(super) fn orchestrator_capabilities(&mut self, caller_index: usize) -> Value {
        // KSG `capabilities` scans unscanned providers for the caller's folder in
        // the background; the next call or delegation retry sees their models.
        let harness = self.threads[caller_index].harness.clone();
        self.scan_caller_workspace(caller_index, &harness);
        let caller = &self.threads[caller_index];
        let (models, source) = self.delegation_models(caller);
        let default_selection = self.default_selection();
        let caller_selection = caller.inherited_selection();
        let inherit_selection = self.inherited_selection(caller_index);
        let model_values: Vec<Value> = models
            .iter()
            .map(|model| {
                json!({
                    "provider": model.provider,
                    "id": model.id,
                    "name": model.name,
                    "reasoning_levels": model.thinking_levels,
                })
            })
            .collect();
        let mut capabilities = json!({
            "ready": !models.is_empty(),
            "scope": self.scope_value(caller.captain),
            "workspace_required": caller.captain.is_some_and(|id| self.captain_scope(id).len() > 1),
            "discovery": {
                "state": self.discovery_state(source),
                "source": source,
                "error": self.catalog.error,
            },
            "runtime": {
                "harness": caller.harness,
                "providers": ["cpa"],
                "hosts": ["local"],
                "machine": caller.machine,
                "host_supported": caller.machine == "local",
            },
            "constraints": {
                "running_children_per_coordinator": self.scheduler.limits.coordinator_running,
                "unfinished_tasks_per_coordinator": self.scheduler.limits.coordinator_unfinished,
                "direct_providers": false,
                "remote_hosts": true,
                "task_plans": true,
                "model_ids": "exact ids only",
                "reasoning_levels": "exact levels only, never clamped",
            },
            "default_selection": default_selection.map_or(Value::Null, |selection| selection.to_value()),
            "caller_selection": caller_selection.map_or(Value::Null, |selection| selection.to_value()),
            "inherit_selection": inherit_selection.map_or(Value::Null, |selection| selection.to_value()),
            "models": model_values,
            "repositories": self.repository_list_value()["repositories"],
            "notes": [
                "Pi uses CPA. Codex and ACP use their own configured providers and login.",
                "Linked peers run workers on their own projects and credentials. Workers cannot delegate further.",
                "Pass exact model ids and reasoning levels. Unknown or unsupported choices are rejected before the child is created and are never silently substituted or clamped.",
                "Omit target to inherit inherit_selection. Given target fields override the inherited selection.",
                "When ready is false, discovery has not produced a usable catalog. Do not invent model ids or levels."
            ],
        });
        self.add_harness_capabilities(Some(caller_index), &mut capabilities);
        let peers = crate::database::for_path(&self.data_dir.join("index.json"))
            .and_then(|database| database.peers().ok())
            .unwrap_or_else(|| self.peers.clone());
        capabilities["peers"] = json!(
            peers
                .iter()
                .map(crate::peer::Peer::summary)
                .collect::<Vec<_>>()
        );
        // Effective coordination rights, identical for Pi, Claude, Codex and ACP callers:
        // the server resolves them from the calling thread, never model-supplied ids.
        // Each `may_*` mirrors the matching handler's ownership and lifecycle gates; it
        // does not promise call-specific validation (prompt, target or capacity) succeeds.
        let (captain, primary_home) = self.captain_actor(caller_index);
        // How the caller's harness treats `mcp__shina__` names before they reach this
        // server. Stated per adapter because admission policy is not uniform: Pi's bridge
        // and Claude's policy gate by name, Codex leaves it to its approval mode, and ACP
        // agents own their own prompts.
        let admission = match caller.harness.as_str() {
            "pi" => {
                "The Pi bridge admits this thread's own mcp__shina__ tools without a prompt in read-only and full-access; in other modes it auto-admits tools marked readOnlyHint and asks before the rest."
            }
            "claude" => {
                "In read-only, exact advertised mcp__shina__ names run without a prompt only after the session verifies its registered shina server is this thread's token-scoped endpoint; foreign or lookalike names stay denied. Other access modes run them through the normal permission flow."
            }
            "codex" => {
                "The thread's required shina MCP server runs under Codex's approval policy for this access mode; Shina adds no per-name gate."
            }
            _ => {
                "MCP admission is the agent's own permission policy on this harness; Shina cannot report whether a call prompts or is refused."
            }
        };
        // Authoritative admission state for this caller's coordinator: the
        // same limits the scheduler enforces at execution time, usage and
        // the caller-visible queue. Not a cached discovery record.
        capabilities["scheduling"] =
            self.scheduling_scoped_value(Some(self.effective_home(caller.id)));
        capabilities["caller"] = json!({
            "role": self.mcp_role(caller_index),
            "captain": captain,
            "primary_home": primary_home,
            "access": caller.native.access.id(),
            "harness": caller.harness,
            "machine": caller.machine,
            "host": if caller.peer_origin.is_some() { "remote" } else { "local" },
            "delegated": caller.delegated,
            "crew_member": caller.crew_captain.is_some(),
            "owner_status": captain.map(|id| match self.captain_phase(id) {
                CaptainPhase::Active => "active",
                CaptainPhase::Archived => "archived",
                CaptainPhase::Retired => "retired",
                CaptainPhase::Deleted => "deleted",
            }),
            // Legacy booleans remain for older clients, but derive from the
            // same effective gates the tool record and dispatch use.
            "may_delegate": self.denial(caller_index, "delegate_task").is_none(),
            "may_control_crew": self.denial(caller_index, "captain_crew_create").is_none(),
            "may_write_captain_documents": self
                .denial(caller_index, "captain_document_write")
                .is_none(),
            "may_answer_questions": self
                .denial(caller_index, "captain_question_answer")
                .is_none(),
            "may_manage_routines": self.denial(caller_index, "create_routine").is_none(),
            "shina_tools": {
                "endpoint": caller.host.is_some(),
                "admission": admission,
            },
        });
        // The effective record (#32): coordination rights with precise reasons,
        // interaction support, authorized reads, placement, live capacity and
        // the per-tool listing `tools/list` now returns. Dispatch revalidates
        // each gate again at call time.
        capabilities["contract"] = json!(capabilities::CONTRACT);
        capabilities["effective"] = json!({
            "coordination": self.coordination(caller_index),
            "interaction": self.interaction(caller_index),
            "reads": self.reads(caller_index),
            "execution": self.execution(caller_index),
            "placement": self.placement(caller_index),
            "capacity": self.capacity(caller_index),
        });
        capabilities["tools"] = self.tools_record(caller_index);
        capabilities
    }

    /// Connects one thread's Pi with its captain's instructions and its Shina MCP link.
    pub fn connect_thread(&mut self, index: usize) {
        if let Some(thread) = self.threads.get_mut(index) {
            thread.provider_update_held =
                self.provider_update_drivers.contains(&thread.harness) && thread.machine == "local";
            if thread.provider_update_held {
                return;
            }
            // A worker's provider never starts before its placement resolves.
            if thread
                .placement
                .as_ref()
                .is_some_and(|placement| placement.phase != placement::PlacementPhase::Ready)
            {
                return;
            }
        }
        if index < self.threads.len()
            && self
                .tasks
                .iter()
                .any(|task| task.child_thread_id == self.threads[index].id && task.remote.is_some())
        {
            return;
        }
        // A queued admission reservation gates every provider start, however
        // the call reached here. Held reservations and unmanaged interactive
        // threads never match.
        if index < self.threads.len() && self.admission_blocks_connect(self.threads[index].id) {
            return;
        }
        if self.demo
            || !self.runtime_enabled
            || self.client.is_some()
            || self.ownership_denied
            || index >= self.threads.len()
        {
            return;
        }
        if self
            .require_active_thread_owner(self.threads[index].id)
            .is_err()
            || self.threads[index].archived()
        {
            return;
        }
        self.threads[index].server_owned = self.headless || self.persist_transcripts;
        let path = match self.thread_directory(index) {
            Ok(path) => path,
            Err(error) => {
                self.threads[index].fail_session(error);
                return;
            }
        };
        #[cfg(test)]
        {
            let tid = self.threads[index].id;
            self.test_connect_workspaces.push((tid, path.clone()));
            self.test_connect_sessions.push((
                tid,
                self.threads[index].harness.clone(),
                self.threads[index].session.clone(),
            ));
            if let Some(error) = self.test_connect_error.clone() {
                self.threads[index].fail_session(error);
                return;
            }
            if let Some(hook) = self.test_connect_pi.as_mut()
                && let Some(pi) = hook(tid, &path)
            {
                let thread = &mut self.threads[index];
                thread.error = None;
                if !thread.pending_injections.is_empty() && !thread.status.busy() {
                    thread.arm_task();
                    thread.status = Status::Running;
                    thread.started = None;
                    thread.phase = "Working".into();
                } else if thread.status == Status::Failed {
                    thread.status = Status::Ready;
                }
                thread.initializing = false;
                thread.prewarm_attempted = true;
                thread.pi = Some(pi);
                thread.drive_session();
                return;
            }
        }
        if self.threads[index].project.is_none()
            && let Err(error) = std::fs::create_dir_all(&path)
        {
            self.threads[index].fail_session(format!("Could not create the chat folder: {error}"));
            return;
        }
        let wake = self.wake.clone();
        let thread_id = self.threads[index].id;
        self.sync_memory_links();
        self.refresh_captain_contexts();
        let role = self.mcp_role(index);
        self.threads[index].instruction_role = Some(role);
        self.threads[index].shina = Self::instructions_for_role(role);
        // The session start carries the current text, so a stale next-turn
        // update is not needed for this connection.
        self.threads[index].instruction_context_pending = false;
        let instructions = self.connection_instructions(index);
        if matches!(
            self.threads[index].harness.as_str(),
            "pi" | "codex" | "claude"
        ) || self.threads[index].session.is_none()
        {
            self.threads[index].memory_context = self.threads[index]
                .memory
                .as_ref()
                .map(|link| link.seen())
                .unwrap_or_default();
        }
        // Future project notes can augment this prompt using (thread.machine, thread.project).
        // Context storage is deliberately separate from persona and session persistence.
        // Shina's tools are an addition. If the MCP cannot start, Pi still works without them.
        let host = crate::mcp::server(wake.clone()).and_then(|server| {
            Ok(HostLink {
                url: server.url.clone(),
                token: server.token_for(thread_id),
                extension: if self.threads[index].harness == "pi" {
                    crate::mcp::extension_path()?
                } else {
                    PathBuf::new()
                },
                access_mode: self.threads[index].native.access.id().into(),
            })
        });
        let prompt_suggestions_enabled = self.thread_defaults.prompt_suggestions_enabled;
        let thread = &mut self.threads[index];
        thread.prompt_suggestions_enabled = prompt_suggestions_enabled && !thread.delegated;
        match host {
            Ok(host) => thread.host = Some(host),
            Err(error) => {
                thread.host = None;
                if thread.native.access != AccessMode::Full {
                    thread.fail_session(format!(
                        "The permission bridge could not start. Pi was not launched: {error}"
                    ));
                    return;
                }
                thread.diagnostic(&error);
            }
        }
        thread.connect_with_instructions(path, wake, instructions);
    }
    /// Answers a Shina MCP tool call on the UI thread. `caller` is the calling thread's id.
    pub(crate) fn mcp_call(
        &mut self,
        caller: usize,
        tool: &str,
        args: &Value,
    ) -> Result<Value, String> {
        if matches!(tool, "initialize" | "tools/list") {
            return self.mcp_meta_call(caller, tool, args);
        }
        if matches!(
            tool,
            "captain_crew_create" | "captain_crew_send" | "captain_crew_stop"
        ) {
            return self.captain_crew_tool(caller, tool, args);
        }
        if !matches!(
            tool,
            "list_routines"
                | "list_memories"
                | "search_memories"
                | "orchestrator_capabilities"
                | "list_threads"
                | "list_captains"
                | "read_thread"
                | "task_status"
                | "captain_read"
                | "captain_document_read"
                | "captain_crew_list"
                | "captain_reports_list"
                | "captain_crew_report"
                | "captain_question_answer"
                | "task_plan"
        ) {
            self.require_active_thread_owner(caller)?;
        }
        if self
            .threads
            .iter()
            .any(|t| t.id == caller && t.peer_origin.is_some())
            && !(matches!(
                tool,
                "propose_memory"
                    | "list_memories"
                    | "search_memories"
                    | "orchestrator_capabilities"
                    | "list_threads"
                    | "read_thread"
            ) || tool == "task_status" && args["thread_id"] == caller)
        {
            return Err("A remote worker may read threads and memories, but cannot delegate or control unrelated work.".into());
        }
        if matches!(
            tool,
            "create_routine" | "list_routines" | "pause_routine" | "delete_routine"
        ) {
            return self.routine_mcp(caller, tool, args);
        }
        if matches!(tool, "delegate_task" | "task_status") {
            wait_seconds(args)?;
        }
        let caller_index = self
            .threads
            .iter()
            .position(|t| t.id == caller)
            .ok_or("The calling thread no longer exists.")?;
        let find = |state: &Self| -> Result<usize, String> {
            let id = args["thread_id"]
                .as_u64()
                .ok_or("thread_id must be a thread id from list_threads.")?;
            state
                .threads
                .iter()
                .position(|t| t.id as u64 == id)
                .ok_or_else(|| format!("There is no thread {id}."))
        };
        let captain_name = |state: &Self, captain: Option<usize>| -> Value {
            captain
                .and_then(|id| state.captains.get(id))
                .map(|c| Value::from(c.name.clone()))
                .unwrap_or(Value::Null)
        };
        match tool {
            "remember" | "forget_memory" | "list_memories" | "search_memories"
            | "propose_memory" => self.memory_call(caller_index, tool, args),
            "orchestrator_capabilities" => Ok(self.orchestrator_capabilities(caller_index)),
            "list_threads" => self.readable_threads(Some(caller), args),
            "read_thread" => {
                let id = args["threadId"].as_u64().ok_or("threadId is required.")?;
                let thread = self
                    .threads
                    .iter()
                    .find(|t| t.id as u64 == id)
                    .ok_or("Unknown thread.")?;
                let mut page = remote_tasks::transcript_page(thread, args)?;
                page["thread"]["project"] = json!(self.task_project_value(thread));
                page["thread"]["machine"] = json!("local");
                Ok(page)
            }
            "list_captains" => {
                let captains: Vec<Value> = self
                    .captains
                    .iter()
                    .enumerate()
                    .filter(|(captain, _)| self.captain_phase(*captain) != CaptainPhase::Deleted)
                    .map(|(i, c)| {
                        json!({
                            "name": c.name,
                            "captain_id": i,
                            "home_thread_id": self.captain_home(i),
                            "scope": self.scope_value(Some(i)),
                            "earlier_conversations": self.earlier_conversations(i),
                            "role": c.role,
                            "lifecycle":self.captain_phase(i),
                            "status": status_word(self.captain_status(i)),
                            "assigned": self.threads.iter().any(|t| t.captain_home && t.captain == Some(i)),
                        })
                    })
                    .collect();
                Ok(
                    json!({ "captains": captains, "peers":self.peers.iter().map(crate::peer::Peer::summary).collect::<Vec<_>>() }),
                )
            }
            "task_prepare" => self.task_plan_prepare(caller_index, args),
            "task_update" => self.task_plan_update(caller_index, args),
            "task_start" => self.task_plan_start(caller_index, args),
            "task_plan" => self.task_plan_read(caller_index, args),
            "delegate_task" => {
                if self.threads[caller_index].delegated {
                    return Err(
                        "Delegated threads cannot delegate further. Do the work in this thread."
                            .into(),
                    );
                }
                // Earlier Captain homes are records, not coordinators; the remote path
                // already refuses them, and local delegation is no different.
                if self.threads[caller_index].captain_home
                    && self.effective_home(self.threads[caller_index].id)
                        != self.threads[caller_index].id
                {
                    return Err(
                        "Open the captain's primary home to delegate new work. This is an earlier conversation."
                            .into(),
                    );
                }
                parse_delegation_target(args)?;
                if remote_tasks::target_machine(args)?.is_some_and(|machine| machine != "local") {
                    return self.delegate_remote(caller_index, args);
                }
                let request_key = tasks::delegate_request_key(args)?;
                if let Some(key) = request_key
                    && let Some(replayed) = self.delegate_receipt_replay(caller_index, key, args)?
                {
                    return Ok(replayed);
                }
                if request_key.is_some() {
                    self.delegate_receipt_ready()?;
                }
                self.delegate_local(caller_index, args, request_key)
            }
            "task_status" => {
                let thread_index = find(self)?;
                self.reconcile_task_operations();
                let thread = &self.threads[thread_index];
                let task = self
                    .tasks
                    .iter()
                    .find(|task| task.child_thread_id == thread.id);
                let reply = thread
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "Pi")
                    .map(|m| truncate(&m.text, 6000));
                let status = thread.task_result.unwrap_or_else(|| {
                    if !thread.status.busy() && !thread.pending_injections.is_empty() {
                        "queued"
                    } else {
                        status_word(thread.status)
                    }
                });
                let mut operations: Vec<_> = task
                    .map(|task| {
                        self.task_operations
                            .iter()
                            .filter(|operation| {
                                operation.task_id.as_deref() == Some(task.id.as_str())
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                if operations.len() > 16 {
                    operations.drain(..operations.len() - 16);
                }
                Ok(json!({
                    "remote":task.and_then(|task|task.remote.as_ref()),
                    "remote_state":task.and_then(|task|task.remote.as_ref()).map(|remote|remote.state.word()),
                    "needs_you_on":task.and_then(|task|task.remote.as_ref()).filter(|remote|remote.needs_you).map(|remote|&remote.label),
                    "thread_id": thread.id,
                    "title": thread.title,
                    "project": self.task_project_value(thread),
                    "machine":task.and_then(|task|task.remote.as_ref()).map(|remote|remote.server_id.as_str()).unwrap_or(&thread.machine),
                    "parent": thread.parent,
                    "captain": captain_name(self, thread.captain),
                    "status": task.map(|task| task.state.id()).unwrap_or(status),
                    "task_id": task.map(|task| &task.id),
                    // Stable task identity, the attempt a keyed control call
                    // fences against, and the live run/runtime generation a
                    // steering send or question answer names.
                    "attempt_id": task.map(|task| task.attempt_id.as_str()),
                    "run_id": task.and_then(|task| task.remote.as_ref())
                        .and_then(|remote| remote.execution.as_ref())
                        .map(|lease| lease.run_id.as_str())
                        .or(thread.run_id.as_deref()),
                    "execution": task.and_then(|task| task.remote.as_ref())
                        .and_then(|remote| remote.execution.as_ref())
                        .map(crate::execution::Lease::value),
                    "runtime_generation": thread.request_generation,
                    // One request shape for local and remote children; the
                    // answer fence reads requestId and generation from here.
                    "request": task
                        .and_then(|task| task.remote.as_ref())
                        .and_then(|remote| remote.request.as_ref())
                        .map(|request| json!({
                            "requestId": request["requestId"],
                            "generation": request["generation"],
                            "kind": request["kind"],
                        }))
                        .or_else(|| thread.request.as_ref().map(|request| json!({
                            "requestId": request["id"],
                            "generation": thread.request_generation,
                            "kind": request["method"],
                        }))),
                    "plan": task
                        .and_then(|task| task.plan_node.as_ref())
                        .map(|plan| json!({"id": plan.plan, "node": plan.node}))
                        .unwrap_or(Value::Null),
                    "queue_position": task.and_then(|task| self.task_queue_position(task)),
                    "admission": task.map(|task| self.admission_value_for_task(task)).unwrap_or(Value::Null),
                    "selection": selection_value(thread),
                    "placement": task.and_then(|task|task.remote.as_ref()).and_then(|remote|remote.placement.clone())
                        .or_else(|| thread.placement.as_ref().map(placement::ThreadPlacement::value)),
                    "finished": task.map(|task| task.state.finished()).unwrap_or(thread.task_result.is_some() || (thread.completion_notified && matches!(thread.status, Status::Completed | Status::Failed))),
                    "phase": thread.phase,
                    "error": task.map(|task| task.error_excerpt.as_str()).filter(|s| !s.is_empty()).or(thread.error.as_deref()),
                    "remote_error": task.and_then(|task|task.remote.as_ref()).and_then(|remote|remote.last_error.as_deref()),
                    "operations": operations,
                    "latest_reply": task.map(|task| task.result_excerpt.as_str()).filter(|s| !s.is_empty()).map(str::to_owned).or(reply),
                }))
            }
            "task_cancel" | "task_send" => {
                let request_key = args["client_request_id"]
                    .as_str()
                    .map(str::trim)
                    .filter(|key| !key.is_empty() && key.len() <= 256)
                    .ok_or("task_send and task_cancel require client_request_id: a nonempty caller-chosen string of at most 256 characters. Reuse it verbatim to retry; never reuse it for different arguments.")?;
                if let Some(replayed) =
                    self.task_control_receipt_replay(caller_index, tool, request_key, args)?
                {
                    return Ok(replayed);
                }
                self.delegate_receipt_ready()?;
                let index = find(self)?;
                if self.prepare_task_control(index, caller, tool, args)? {
                    let status = self.mcp_call(
                        caller,
                        "task_status",
                        &json!({"thread_id":self.threads[index].id}),
                    )?;
                    let result = self.record_task_control_receipt(
                        caller_index,
                        tool,
                        request_key,
                        args,
                        &status,
                    )?;
                    self.start_queued_tasks();
                    return Ok(result);
                }
                if !self.owns_child(caller, index) {
                    return Err(format!(
                        "Only the parent that delegated thread {} may cancel or send to it.",
                        self.threads[index].id
                    ));
                }
                if tool == "task_cancel" {
                    self.threads[index].stop_task()?;
                } else {
                    let message = args["message"]
                        .as_str()
                        .map(str::trim)
                        .filter(|m| !m.is_empty())
                        .ok_or("message must contain a correction or follow-up brief.")?;
                    let request = self.threads[index].request.clone().filter(|r| {
                        matches!(r["method"].as_str(), Some("input" | "editor" | "select"))
                    });
                    if let Some(request) = request {
                        if request["method"] == "select"
                            && !request["options"].as_array().is_some_and(|choices| {
                                choices
                                    .iter()
                                    .any(|choice| choice.as_str() == Some(message))
                            })
                        {
                            return Err("Choose one of the crew question's listed options.".into());
                        }
                        self.threads[index].respond(json!({"value":message}));
                        if self.threads[index].request.is_some() {
                            let error = self.threads[index]
                                .error
                                .clone()
                                .unwrap_or_else(|| "Could not answer the crew question.".into());
                            self.reject_caller_operation(caller_index, request_key, &error);
                            return Err(error);
                        }
                        self.mark_caller_operation_dispatched(caller_index, request_key, "answer");
                        // The local adapter can prove it accepted the write, not
                        // that the provider consumed the answer.
                        self.mark_caller_operation_input_unknown(
                            caller_index,
                            request_key,
                            "adapter_write",
                        );
                        let status = self.mcp_call(
                            caller,
                            "task_status",
                            &json!({"thread_id":self.threads[index].id}),
                        )?;
                        return self.record_task_control_receipt(
                            caller_index,
                            tool,
                            request_key,
                            args,
                            &status,
                        );
                    }
                    if !self.threads[index].status.busy() {
                        self.threads[index].arm_task();
                        if self.threads[index].status == Status::Failed
                            && self.threads[index].pi.is_some()
                        {
                            self.threads[index].status = Status::Ready;
                            self.threads[index].error = None;
                        }
                    }
                    let delivery = self.threads[index].enqueue_injection(format!(
                        "Shina: message from parent #{caller}\n\n{message}"
                    ));
                    self.attach_caller_operation_delivery(caller_index, request_key, &delivery);
                    if self.threads[index].pi.is_none() {
                        self.connect_thread(index);
                    }
                }
                let status = self.mcp_call(
                    caller,
                    "task_status",
                    &json!({"thread_id":self.threads[index].id}),
                )?;
                self.record_task_control_receipt(caller_index, tool, request_key, args, &status)
            }
            "captain_read"
            | "captain_document_read"
            | "captain_document_write"
            | "captain_crew_list"
            | "captain_crew_join"
            | "captain_crew_release"
            | "captain_reports_list"
            | "captain_crew_report"
            | "captain_question_answer" => self.captain_mcp(caller_index, tool, args),
            _ => Err(format!("Shina has no tool named {tool}.")),
        }
    }

    pub(crate) fn mcp_request(&mut self, request: crate::mcp::Request) {
        if request.tool == "delegate_task"
            && let Some(preparation) = self.prepare_remote_delegation(request.thread, &request.args)
        {
            self.defer_remote_mcp(request, preparation);
            return;
        }
        self.mcp_request_ready(request);
    }

    pub(super) fn mcp_request_ready(&mut self, request: crate::mcp::Request) {
        if self.relay_captain_tool(&request) {
            return;
        }
        if matches!(request.tool.as_str(), "read_thread" | "list_threads")
            && self.remote_transcript_request(&request)
        {
            return;
        }
        if request.tool == "captain_document_read" && self.queue_peer_document_read(&request) {
            return;
        }
        if matches!(request.tool.as_str(), "html_render" | "html_preview") {
            let Some(thread) = self.threads.iter().find(|t| t.id == request.thread) else {
                let _ = request
                    .reply
                    .send(Err("The calling thread no longer exists.".into()));
                return;
            };
            crate::html_render::request(
                request,
                self.data_dir.join("attachments"),
                thread.native.access == AccessMode::Full,
                self.html_lane_width,
                self.wake.clone(),
            );
            return;
        }
        if self.learned_memory_request(&request) {
            return;
        }
        let wait = if matches!(request.tool.as_str(), "delegate_task" | "task_status") {
            wait_seconds(&request.args)
        } else {
            Ok(0)
        };
        let result = wait.and_then(|seconds| {
            self.mcp_call(request.thread, &request.tool, &request.args)
                .map(|value| (seconds, value))
        });
        match result {
            Ok((seconds, value))
                if seconds > 0
                    && value["finished"] == false
                    && value["status"] != "interrupted" =>
            {
                let deadline = Instant::now() + Duration::from_secs(seconds);
                let (timer, cancel) = mpsc::channel();
                let wake = self.wake.clone();
                std::thread::spawn(move || {
                    if cancel.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        == Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        wake();
                    }
                });
                self.task_waiters.push(TaskWaiter {
                    thread: value["thread_id"].as_u64().expect("task thread id") as usize,
                    deadline,
                    reply: request.reply,
                    _timer: timer,
                });
            }
            Ok((_, value)) => {
                let _ = crate::outbox::reply(&request.reply, Ok(value));
            }
            Err(error) => {
                let _ = crate::outbox::reply(&request.reply, Err(error));
            }
        }
    }

    /// Called after all threads' events are drained, so simultaneous results cannot overwrite
    /// each other. Each finished run contributes one message to its parent's separate queue.
    pub(super) fn poll_legacy_tasks(&mut self, now: Instant) -> bool {
        let mut changed = false;
        let mut completions = vec![];
        for thread in &mut self.threads {
            if self
                .tasks
                .iter()
                .any(|task| task.child_thread_id == thread.id)
            {
                continue;
            }
            if !thread.completion_notified
                && let Some(result) = thread.task_result
            {
                thread.completion_notified = true;
                if let Some(parent) = thread.parent {
                    let reply = thread
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.role == "Pi")
                        .map(|m| truncate(&m.text, 6000))
                        .unwrap_or_else(|| "(no reply)".into());
                    completions.push((parent, format!(
                        "Shina: task #{} finished\nTitle: {}\nStatus: {result}\nLast reply:\n{reply}\nError: {}",
                        thread.id, thread.title, thread.error.as_deref().map(|e| truncate(e, 2000)).unwrap_or_else(|| "none".into())
                    )));
                }
            }
        }
        for (parent, text) in completions {
            if let Some(index) = self.threads.iter().position(|t| t.id == parent) {
                self.threads[index].enqueue_injection(text);
                if self.threads[index].pi.is_none() {
                    self.connect_thread(index);
                }
                changed = true;
            }
        }
        let waiters = std::mem::take(&mut self.task_waiters);
        for waiter in waiters {
            // Read through the same formatter, including cancellation's idle UI status.
            let result = self.mcp_call(
                waiter.thread,
                "task_status",
                &json!({"thread_id":waiter.thread}),
            );
            if result.as_ref().is_err()
                || result.as_ref().is_ok_and(|value| value["finished"] == true)
                || result
                    .as_ref()
                    .is_ok_and(|value| value["status"] == "interrupted")
                || now >= waiter.deadline
            {
                let _ = crate::outbox::reply(&waiter.reply, result);
            } else {
                self.task_waiters.push(waiter);
            }
        }
        for thread in &mut self.threads {
            changed |= thread.flush_injection();
        }
        changed |= self.reconcile_task_operations();
        changed |= self.poll_context_imports();
        changed
    }
}

pub(crate) fn parse_delegation_target(args: &Value) -> Result<Option<DelegationTarget>, String> {
    let Some(target) = args.get("target") else {
        return Ok(None);
    };
    let object = target.as_object().ok_or(
        "target must be an object with harness or model, and optional provider and options.",
    )?;
    if let Some(key) = object.keys().find(|key| {
        !matches!(
            key.as_str(),
            "machine"
                | "environmentId"
                | "projectId"
                | "repository"
                | "harness"
                | "provider"
                | "model"
                | "options"
        )
    }) {
        return Err(format!(
            "Unknown target setting {key}. Use harness, provider, model and options."
        ));
    }
    let harness = object
        .get("harness")
        .map(|value| {
            value
                .as_str()
                .filter(|id| crate::harness::valid_id(id))
                .map(str::to_owned)
                .ok_or("target.harness must be pi, codex, claude, or an installed ACP id.")
        })
        .transpose()?;
    remote_tasks::target_machine(args)?;
    if let Some(project) = object.get("projectId") {
        if project
            .as_u64()
            .is_none_or(|id| id == 0 || id >= usize::MAX as u64)
        {
            return Err(
                "target.projectId must be a positive project id on the named machine.".into(),
            );
        }
        if remote_tasks::target_machine(args)?.is_none_or(|machine| machine == "local") {
            return Err("target.projectId requires target.environmentId or target.machine.".into());
        }
        if args.get("workspace").is_some() {
            return Err("Pass target.projectId or workspace, not both.".into());
        }
        if object.get("repository").is_some() {
            return Err("Pass target.projectId or target.repository, not both.".into());
        }
    }
    if let Some(repository) = object.get("repository") {
        if repository
            .as_str()
            .is_none_or(|selector| selector.trim().is_empty())
        {
            return Err("target.repository must name a catalog id or repository URL.".into());
        }
        if remote_tasks::target_machine(args)?.is_none_or(|machine| machine == "local") {
            return Err("target.repository requires a linked target.machine.".into());
        }
        if args.get("workspace").is_some() {
            return Err("Pass target.repository or workspace, not both.".into());
        }
    }
    if !object.contains_key("model")
        && harness.is_none()
        && remote_tasks::target_machine(args)?.is_none()
    {
        return Err("target.model is required when target is set without a harness.".into());
    }
    let model = object
        .get("model")
        .map(|value| {
            value.as_str()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .ok_or(
            "target.model must be a non-empty exact model id from orchestrator_capabilities.",
        )
        })
        .transpose()?;
    let provider = object
        .get("provider")
        .map(|value| {
            value
                .as_str()
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .ok_or("target.provider must be a nonempty string.")
        })
        .transpose()?;
    let reasoning = match object.get("options") {
        None => None,
        Some(options) => {
            let options = options
                .as_object()
                .ok_or("target.options must be an object with reasoning, or its alias thinking.")?;
            let mut reasoning: Option<String> = None;
            for (key, value) in options {
                let level = match key.as_str() {
                    "reasoning" | "thinking" => value
                        .as_str()
                        .map(str::trim)
                        .filter(|level| !level.is_empty())
                        .ok_or_else(|| {
                            format!(
                                "target.options.{key} must be a non-empty reasoning level from orchestrator_capabilities."
                            )
                        })?,
                    _ => {
                        return Err(format!(
                            "Unknown target.options setting {key}. Use reasoning, or its alias thinking."
                        ));
                    }
                };
                if let Some(existing) = &reasoning
                    && existing != level
                {
                    return Err(
                        "target.options.reasoning and target.options.thinking disagree. Pass one level."
                            .into(),
                    );
                }
                reasoning = Some(level.to_owned());
            }
            reasoning
        }
    };
    Ok(Some(DelegationTarget {
        harness,
        provider,
        model,
        reasoning,
    }))
}

fn wait_seconds(args: &Value) -> Result<u64, String> {
    match args.get("wait_seconds") {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|&seconds| seconds <= 600)
            .ok_or_else(|| "wait_seconds must be an integer from 0 to 600.".into()),
    }
}
