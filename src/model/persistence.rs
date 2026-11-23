use super::*;

impl State {
    pub fn add_project(&mut self, path: PathBuf) -> Result<Option<ProjectId>, String> {
        let (purpose, captain) = self.project_selection_purpose();
        if let Some(result) = self.remote(
            "project.put",
            json!({"path":path,"selectFor":purpose,"captain":captain}),
        ) {
            result?;
            return Ok(None);
        }
        if !path.is_absolute() {
            return Err("A workspace needs an absolute path on this server.".into());
        }
        let path = match path.canonicalize() {
            Ok(path) if path.is_dir() => crate::projects::clean_path(&path),
            _ => return Err("That project folder could not be opened.".into()),
        };
        let id = if let Some(project) = self
            .projects
            .entries
            .iter_mut()
            .find(|p| crate::projects::same_path(&p.path, &path) && p.machine == "local")
        {
            project.hidden = false;
            project.updated = crate::projects::now();
            project.id
        } else {
            let id = self.projects.next_id.max(1);
            self.projects.next_id = id + 1;
            self.projects.entries.push(Project::new(id, path));
            id
        };
        if let Some(project) = self.project(Some(id)) {
            remote_tasks::repository(&project.path);
        }
        if let Some(index) = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.project == Some(id) && !t.archived())
            .max_by_key(|(_, t)| (t.updated, t.id))
            .map(|(i, _)| i)
        {
            self.show_thread(index);
        } else {
            self.new_thread_in(Some(id));
        }
        self.show_archive = false;
        self.checkpoint();
        if purpose != "project" {
            self.machines.project_selection.insert(
                self.machines.selected.clone(),
                (id, purpose.into(), captain),
            );
            self.finish_project_selection();
        }
        Ok(Some(id))
    }
    pub fn setup(&mut self, wake: Waker) {
        self.wake = wake.clone();
        let image_wake = wake.clone();
        self.attachment_cache =
            crate::runtime::UiResource::new(move || rune::ImageCache::new(image_wake));
        if self.demo {
            return;
        }
        // Migrated profiles always attach/supervise a server, including plain launches.
        // Keep load_global's refusal as a defense against direct legacy access.
        if self.client.is_none() && self.data_dir.join("state.sqlite").exists() {
            self.client = Some(crate::client::Client::new(self.data_dir.clone()));
        }
        if self.client.is_some() {
            self.setup_client(wake);
            return;
        }
        if self.owner.is_none() {
            match crate::server_lock::Owner::acquire(&self.data_dir) {
                Ok(owner) => self.owner = Some(owner),
                Err(error) => {
                    self.ownership_denied = true;
                    self.storage_error = Some(error);
                    return;
                }
            }
        }
        if let Err(error) = std::fs::create_dir_all(&self.data_dir) {
            self.storage_error = Some(format!("Could not create Shina's data folder: {error}"));
            return;
        }
        self.load_global(wake.clone());
        if self.ownership_denied {
            return;
        }
        self.init_keybindings_file(wake.clone());
        self.setup_background();
        self.runtime_enabled = true;
        self.start_task_recovery();
        self.refresh_catalog();
        self.updater.start(true, Instant::now(), wake.clone());
        self.prewarm_current();
    }
    pub fn leave_demo(&mut self) {
        if !self.demo {
            return;
        }
        let mut live = Self::new(std::env::current_dir().unwrap_or_default(), false);
        live.setup(self.wake.clone());
        *self = live;
    }

    pub(super) fn load_global(&mut self, wake: Waker) {
        if !self.headless && self.data_dir.join("state.sqlite").exists() {
            self.ownership_denied = true;
            self.storage_error = Some("This profile uses the durable server database. Launch with --server-client. Legacy JSON mode cannot write a migrated profile.".into());
            return;
        }
        if self.run_marker.is_none() {
            match crate::store::RunMarker::start(&self.data_dir) {
                Ok(marker) => self.run_marker = Some(marker),
                Err(error) => self.storage_error = Some(error),
            }
        }
        let path = self.data_dir.join("index.json");
        let mut runtime_inputs = None;
        match crate::store::load_global(&self.data_dir, self.launch_project.as_deref()) {
            Ok(Some(index))
                if index["version"] == 3
                    || index["version"] == 4
                    || index["version"] == crate::identity::INDEX_VERSION =>
            {
                if let Err(error) = self.restore_global(&index) {
                    self.storage_error =
                        Some(format!("{error} The index has not been overwritten."));
                    return;
                }
                if let Some(inputs) = index.get("runtimeInputs") {
                    if !inputs.is_object() {
                        self.storage_error = Some(
                            "Invalid durable runtime inputs. The index has not been overwritten."
                                .into(),
                        );
                        return;
                    }
                    runtime_inputs = Some(inputs.clone());
                }
            }
            Ok(Some(_)) => {
                self.storage_error = Some("The saved thread index uses an unsupported version. It has not been overwritten.".into());
                return;
            }
            Ok(None) => {}
            Err(error) => {
                self.storage_error = Some(error);
                return;
            }
        }
        let ui_path = self.data_dir.join(if self.headless {
            "server-inputs.json"
        } else {
            "ui-state.json"
        });
        if let Err(error) = self.load_memory(true) {
            self.storage_error = Some(error);
            return;
        }
        let saved_ui = crate::store::load(&ui_path).and_then(|ui| {
            if self.headless && ui.is_none() {
                crate::store::load(&self.data_dir.join("ui-state.json"))
            } else {
                Ok(ui)
            }
        });
        match saved_ui {
            Ok(Some(mut ui)) => {
                if self.headless {
                    let mut defaults = self.ui_snapshot();
                    for key in ["pendingMessages", "background"] {
                        if let Some(value) = ui.get(key) {
                            defaults[key] = value.clone();
                        }
                    }
                    ui = defaults;
                }
                if let Some(inputs) = &runtime_inputs {
                    ui["pendingMessages"] = inputs.clone();
                }
                if let Err(error) = self.restore_ui(&ui) {
                    self.storage_error = Some(error);
                    return;
                }
            }
            Ok(None) => {
                if let Some(inputs) = &runtime_inputs {
                    for thread in &mut self.threads {
                        if let Err(error) =
                            thread.restore_pending_inputs(&inputs[thread.id.to_string()])
                        {
                            self.storage_error = Some(error);
                            return;
                        }
                    }
                }
            }
            Err(error) => {
                self.storage_error = Some(error);
                return;
            }
        }
        if self.headless {
            for thread in &mut self.threads {
                thread.draft.replace_text("");
                thread.draft_images.clear();
            }
        }
        self.ui_store = Some(crate::store::Store::new(ui_path, wake.clone()));
        self.store = Some(crate::store::Store::new(path, wake));
        // The stash sits in its own file so a corrupt one never breaks UI state loading.
        let (stash, stash_warning) = crate::ui::composer_stash::load(&self.data_dir);
        self.stash = stash;
        if let Some(warning) = stash_warning {
            self.composer_notices.push(warning);
        }
        self.prepare_task_recovery();
        if let Some(path) = self.launch_project.take()
            && let Err(error) = self.add_project(path)
        {
            self.current_mut().error = Some(error);
        }
        if !self.headless {
            self.checkpoint();
        }
    }

    pub(super) fn restore_global(&mut self, index: &Value) -> Result<(), String> {
        self.restore_records(index, true)
    }
    /// Projections share record validation with session restore, but never restore navigation.
    pub(super) fn restore_records(
        &mut self,
        index: &Value,
        restore_session: bool,
    ) -> Result<(), String> {
        self.identity = match crate::identity::Catalog::from_index(index)? {
            Some(catalog) => catalog,
            None => crate::identity::catalog_from_legacy(index, &self.identity.controller_id)?,
        };
        self.execution = match index.get("execution") {
            None | Some(Value::Null) => crate::execution::Book::default(),
            Some(value) => {
                let book: crate::execution::Book = serde_json::from_value(value.clone())
                    .map_err(|_| "Invalid saved execution lease book.")?;
                book.validate()?;
                book
            }
        };
        self.default_harness = match index.get("defaultHarness") {
            None => "codex".into(),
            Some(value) => value
                .as_str()
                .filter(|id| crate::harness::valid_id(id))
                .ok_or("Invalid default harness.")?
                .into(),
        };
        self.thread_defaults =
            serde_json::from_value(index.get("threadDefaults").cloned().unwrap_or(json!({})))
                .map_err(|_| "Invalid thread defaults.")?;
        if AccessMode::parse(&self.thread_defaults.access).is_none() {
            self.thread_defaults.access = AccessMode::Supervised.id().into();
        }
        self.thread_defaults.validate()?;
        self.model_favorites = match index.get("modelFavorites") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                serde_json::from_value(value.clone()).map_err(|_| "Invalid model favorites.")?,
            ),
        };
        self.persist_transcripts = self.headless || index["threadViews"].is_array();
        self.server_seq = index["serverSeq"].as_u64().unwrap_or(0);
        self.command_receipts =
            serde_json::from_value(index.get("commandReceipts").cloned().unwrap_or(json!([])))
                .map_err(|e| format!("Invalid command receipts: {e}"))?;
        self.command_intents =
            serde_json::from_value(index.get("commandIntents").cloned().unwrap_or(json!({})))
                .map_err(|e| format!("Invalid command intents: {e}"))?;
        self.recovered_inputs =
            serde_json::from_value(index.get("recoveredInputs").cloned().unwrap_or(json!({})))
                .map_err(|e| format!("Invalid recovered inputs: {e}"))?;
        if let Some(epoch) = index["serverEpoch"].as_str() {
            self.server_epoch = epoch.into();
        }
        let Some(profiles) = index["captains"].as_array() else {
            return Err("The saved captain profiles are invalid.".into());
        };
        let captains: Option<Vec<Captain>> = profiles.iter().map(Captain::from_value).collect();
        let Some(captains) = captains else {
            return Err("A saved captain profile is invalid.".into());
        };
        let projects: Option<Vec<_>> = index["projects"]
            .as_array()
            .ok_or("Invalid project list.")?
            .iter()
            .map(|value| Project::from_record(value, self.client.is_none()))
            .collect();
        let mut canonical_projects: Vec<Project> = Vec::new();
        let mut project_remap = std::collections::HashMap::new();
        let mut maximum_project_id = 0;
        for project in projects.ok_or("Invalid project record.")? {
            maximum_project_id = maximum_project_id.max(project.id);
            if project_remap.contains_key(&project.id) {
                return Err("Duplicate project ids.".into());
            }
            if let Some(existing) = canonical_projects.iter_mut().find(|existing| {
                existing.machine == project.machine
                    && if self.client.is_some() {
                        existing.path == project.path
                    } else {
                        crate::projects::same_path(&existing.path, &project.path)
                    }
            }) {
                project_remap.insert(project.id, existing.id);
                existing.hidden &= project.hidden;
                existing.updated = existing.updated.max(project.updated);
                if existing.captain.is_none() {
                    existing.captain = project.captain;
                    existing.captain_thread = project.captain_thread;
                }
            } else {
                project_remap.insert(project.id, project.id);
                canonical_projects.push(project);
            }
        }
        let projects = canonical_projects;
        let project_ids: std::collections::HashSet<_> = projects.iter().map(|p| p.id).collect();
        if project_ids.len() != projects.len() {
            return Err("Duplicate project ids.".into());
        }
        if let Some(threads) = index["threads"].as_array() {
            let missing_crew: std::collections::HashSet<usize> = threads
                .iter()
                .filter(|record| record.get("crewCaptain").is_none())
                .filter_map(|record| record["id"].as_u64())
                .filter_map(|id| usize::try_from(id).ok())
                .collect();
            let restored: Vec<Thread> = threads
                .iter()
                .filter_map(|record| {
                    let id = usize::try_from(record["id"].as_u64()?).ok()?;
                    if id == 0 || id == usize::MAX {
                        return None;
                    }
                    let mut thread = Thread::new(id);
                    if let Some(harness) = record.get("harness") {
                        let harness = harness.as_str()?;
                        if !crate::harness::valid_id(harness) {
                            return None;
                        }
                        thread.harness = harness.into();
                    } else {
                        // Indexes saved before per-thread harnesses have no
                        // harness field; those threads are all Pi sessions.
                        thread.harness = "pi".into();
                    }
                    if let Some(sent) = record.get("harnessContextSent") {
                        thread.harness_context_sent = sent.as_bool()?;
                    }
                    if let Some(context) = record.get("memoryContext") {
                        thread.memory_context = serde_json::from_value(context.clone()).ok()?;
                        if thread.memory_context.len() > 257
                            || thread.memory_context.iter().any(|(id, hash)| {
                                id != "profile" && !id.starts_with("memory-")
                                    || hash.len() != 64
                                    || !hash.bytes().all(|b| b.is_ascii_hexdigit())
                            })
                        {
                            return None;
                        }
                    }
                    if let Some(pending) = record.get("scopeContextPending") {
                        thread.scope_context_pending = pending.as_bool()?;
                    }
                    thread.captain_return_context = record["captainReturnContext"] == true;
                    thread.never_sent = record["neverSent"] == true;
                    // Handoff records are optional thread data: absent means none, and
                    // older files decode with an empty list.
                    thread.handoffs = serde_json::from_value(record["handoffs"].clone())
                        .unwrap_or_default();
                    thread.handoff_sessions =
                        serde_json::from_value(record["handoffSessions"].clone())
                            .unwrap_or_default();
                    thread.native.fork_source = record["forkSource"].as_str().map(str::to_owned);
                    thread.native.fork_entry = record["forkEntry"].as_str().map(str::to_owned);
                    thread.native.previous_session =
                        record["previousSession"].as_str().map(str::to_owned);
                    if let Some(access) = record.get("accessMode") {
                        thread.native.access =
                            AccessMode::parse(access.as_str()?).unwrap_or(AccessMode::Supervised);
                    }
                    if index["version"] == 4 && record.get("lifecycle").is_none() {
                        return None;
                    }
                    for field in ["renamed", "captainHome", "delegated"] {
                        if record
                            .get(field)
                            .is_some_and(|v| !v.is_null() && !v.is_boolean())
                        {
                            return None;
                        }
                    }
                    for field in ["session", "model", "thinking"] {
                        if record
                            .get(field)
                            .is_some_and(|v| !v.is_null() && !v.is_string())
                        {
                            return None;
                        }
                    }
                    if record
                        .get("updated")
                        .is_some_and(|v| !v.is_null() && !v.is_u64())
                    {
                        return None;
                    }
                    thread.project = if record["project"].is_null() {
                        None
                    } else {
                        let original = usize::try_from(record["project"].as_u64()?).ok()?;
                        Some(*project_remap.get(&original)?)
                    };
                    thread.machine = record["machine"].as_str()?.into();
                    thread.updated = record["updated"].as_u64().unwrap_or(0);
                    if thread.machine.is_empty()
                        || thread.project.is_some_and(|id| !project_ids.contains(&id))
                    {
                        return None;
                    }
                    thread.title = record["title"].as_str()?.into();
                    thread.renamed = record["renamed"] == true;
                    thread.session = record["session"].as_str().map(str::to_owned);
                    thread.run_id = record["runId"].as_str().map(str::to_owned);
                    thread.execution = record
                        .get("execution")
                        .filter(|value| !value.is_null())
                        .map(|value| serde_json::from_value(value.clone()))
                        .transpose()
                        .ok()?;
                    thread.next_execution = record
                        .get("nextExecution")
                        .filter(|value| !value.is_null())
                        .map(|value| serde_json::from_value(value.clone()))
                        .transpose()
                        .ok()?;
                    thread.peer_input_executions = record
                        .get("peerInputExecutions")
                        .filter(|value| !value.is_null())
                        .map(|value| serde_json::from_value(value.clone()))
                        .transpose()
                        .ok()?
                        .unwrap_or_default();
                    thread.peer_provider_inputs = record
                        .get("peerProviderInputs").filter(|value| !value.is_null())
                        .map(|value| serde_json::from_value(value.clone())).transpose().ok()?
                        .unwrap_or_default();
                    if thread.peer_provider_inputs.iter().any(|(input, binding)| {
                        thread.peer_input_executions.get(input) != Some(binding)
                    }) {
                        return None;
                    }
                    if thread.peer_input_executions.iter().any(|(input, binding)| {
                        uuid::Uuid::parse_str(input).is_err()
                            || self.execution.get(&binding.lease_id)
                                .is_none_or(|lease| lease.binding() != *binding)
                    }) {
                        return None;
                    }
                    thread.provider_accepted_run =
                        record["providerAcceptedRun"].as_str().map(str::to_owned);
                    thread.provider_running_run =
                        record["providerRunningRun"].as_str().map(str::to_owned);
                    thread.provider_terminal_run =
                        record["providerTerminalRun"].as_str().map(str::to_owned);
                    thread.provider_terminal_status = record["providerTerminalStatus"]
                        .as_str()
                        .map(str::to_owned);
                    thread.resumable_run_id = record["resumableRunId"].as_str().map(str::to_owned);
                    thread.continuation_of_run_id =
                        record["continuationOfRunId"].as_str().map(str::to_owned);
                    if let Some(model) = record["model"].as_str() {
                        // Pi restores only CPA choices. ACP selections belong to its agent id.
                        let provider = if thread.harness == "pi" {
                            "cpa"
                        } else {
                            &thread.harness
                        };
                        thread.selected_model = Some((provider.into(), model.into()));
                        thread.model_provider = provider.into();
                        thread.model_id = model.into();
                        thread.model = model.into();
                    }
                    thread.selected_thinking = record["thinking"].as_str().map(str::to_owned);
                    if let Some(options) = record.get("options")
                        && let Ok(parsed) = serde_json::from_value::<
                            Vec<crate::model_selection::ProviderOptionSelection>,
                        >(options.clone())
                    {
                        thread.model_options = parsed;
                    }
                    if let Some(breakdown) = record.get("contextBreakdown")
                        && let Ok(parsed) = serde_json::from_value::<
                            Option<Vec<crate::context_breakdown::ContextBreakdownRow>>,
                        >(breakdown.clone())
                    {
                        thread.context.usage.breakdown = parsed;
                    }
                    thread.worktree_path = record
                        .get("worktreePath")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    if !restore_session {
                        thread.projected_agent_counts = record["agentCount"].as_u64()
                            .zip(record["activeAgentCount"].as_u64())
                            .and_then(|(total, active)| Some((usize::try_from(active).ok()?, usize::try_from(total).ok()?)));
                    }
                    thread.scratch_directory = match record.get("scratchDirectory") {
                        None => None,
                        Some(value) => {
                            let path = value.as_str()?;
                            if path.is_empty() || path.len() > 32768 || path.chars().any(char::is_control)
                                || self.client.is_none() && thread.machine == "local"
                                    && !std::path::Path::new(path).is_absolute()
                            {
                                return None;
                            }
                            Some(path.to_owned())
                        }
                    };
                    thread.placement = record
                        .get("placement")
                        .filter(|v| !v.is_null())
                        .map(|v| serde_json::from_value(v.clone()))
                        .transpose()
                        .ok()?;
                    thread.branch = record
                        .get("branch")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    thread.base_branch = record
                        .get("baseBranch")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    thread.env_mode = record
                        .get("envMode")
                        .and_then(|v| serde_json::from_value::<ThreadEnvMode>(v.clone()).ok());
                    thread.start_from_origin =
                        record.get("startFromOrigin").and_then(Value::as_bool);
                    if let Some(setup_val) = record.get("worktreeSetup")
                        && let Ok(mut setup) =
                            serde_json::from_value::<Option<WorktreeSetupState>>(setup_val.clone())
                    {
                        if let Some(ref mut s) = setup
                            && s.phase == WorktreeSetupPhase::Running
                        {
                            s.phase = WorktreeSetupPhase::Failed;
                            s.error = Some(
                                "Worktree setup was interrupted before restart. Retry or skip to continue."
                                    .into(),
                            );
                            if let Some(placement) = thread.placement.as_mut() {
                                placement.phase = placement::PlacementPhase::Failed;
                                placement.error = s.error.clone();
                            }
                            thread.worktree_setup_output_open = true;
                        }
                        thread.worktree_setup = setup;
                    }
                    if let Some(suggestion) = record.get("promptSuggestion")
                        && let Ok(parsed) =
                            serde_json::from_value::<Option<PromptSuggestion>>(suggestion.clone())
                    {
                        thread.prompt_suggestion = parsed;
                    }
                    if thread.harness == "codex" {
                        thread.harness_state["serviceTier"] = record["serviceTier"].clone();
                    }
                    if let Some(level) = &thread.selected_thinking {
                        thread.thinking = level.clone();
                    }
                    thread.life = Lifecycle::from_record(record).ok()?;
                    thread.captain = if record["captain"].is_null() {
                        None
                    } else {
                        let id = usize::try_from(record["captain"].as_u64()?).ok()?;
                        if id >= captains.len() {
                            return None;
                        }
                        Some(id)
                    };
                    thread.captain_home = record["captainHome"] == true;
                    thread.crew_captain = match record.get("crewCaptain") {
                        None => None,
                        Some(value) if value.is_null() => None,
                        Some(value) => Some(usize::try_from(value.as_u64()?).ok()?),
                    };
                    if let Some(outbox) = record.get("crewReportOutbox") {
                        thread.crew_report_outbox = serde_json::from_value(outbox.clone()).ok()?;
                    }
                    if let Some(last) = record.get("crewReportLastRun") {
                        thread.crew_report_last_run = if last.is_null() {
                            None
                        } else {
                            Some(last.as_str()?.to_owned())
                        };
                    }
                    thread.delegated = record["delegated"] == true;
                    thread.peer_origin = record
                        .get("peerOrigin")
                        .filter(|v| !v.is_null())
                        .map(|v| serde_json::from_value(v.clone()))
                        .transpose()
                        .ok()?;
                    thread.stop_serial = record["stopSerial"].as_u64().unwrap_or(0);
                    thread.admission = record["admission"].as_str().map(str::to_owned);
                    thread.admission_run = record["admissionRun"].as_u64().unwrap_or(0);
                    if let Some(saved) = record
                        .get("admissionPrompt")
                        .filter(|value| !value.is_null())
                    {
                        // A malformed queued prompt must not invalidate the
                        // thread record; the reservation it covers simply
                        // releases at reconcile instead.
                        let images: Vec<_> = saved["images"]
                            .as_array()
                            .map(|items| {
                                items
                                    .iter()
                                    .filter_map(Attachment::from_value)
                                    .collect()
                            })
                            .unwrap_or_default();
                        let context = saved
                            .get("context")
                            .filter(|v| !v.is_null())
                            .map(crate::composer_context::decode_message_context)
                            .transpose()
                            .ok()
                            .flatten();
                        if let Some(text) = saved["text"].as_str()
                            && let Ok(prompt) = crate::composer_tokens::received_prompt_with_context(
                                text.into(),
                                images,
                                saved.get("draft").cloned(),
                                context,
                            )
                        {
                            thread.queued_prompt = Some((prompt, thread.messages.len()));
                        }
                    }
                    thread.parent = if record["parent"].is_null() {
                        None
                    } else {
                        Some(usize::try_from(record["parent"].as_u64()?).ok()?)
                    };
                    Some(thread)
                })
                .collect();
            let ids: std::collections::HashSet<_> = restored.iter().map(|t| t.id).collect();
            let proven: std::collections::HashSet<(usize, usize)> = index["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|record| {
                    let captain = usize::try_from(record["coordinator_captain"].as_u64()?).ok()?;
                    let parent = usize::try_from(record["parent_thread_id"].as_u64()?).ok()?;
                    let child = usize::try_from(record["child_thread_id"].as_u64()?).ok()?;
                    (captain < captains.len()).then_some((parent, child))
                })
                .collect();
            let parents_valid = restored.iter().all(|thread| {
                (!thread.captain_home || thread.captain.is_some() && !thread.delegated)
                    && thread.delegated == (thread.parent.is_some() || thread.peer_origin.is_some())
                    && thread.parent.is_none_or(|parent| {
                        thread.delegated
                            && parent != thread.id
                            && restored.iter().any(|t| {
                                t.id == parent
                                    && !t.delegated
                                    && t.machine == thread.machine
                                    && (t.project == thread.project
                                        || proven.contains(&(parent, thread.id))
                                        || (t.captain_home
                                            && t.captain.is_some()
                                            && t.captain == thread.captain
                                            && missing_crew.contains(&thread.id)))
                            })
                    })
                    && thread.crew_captain.is_none_or(|captain| {
                        captain < captains.len()
                            && !thread.captain_home
                            && thread.peer_origin.is_none()
                            && thread.captain == Some(captain)
                    })
            });
            if restored.len() != threads.len() || ids.len() != restored.len() || !parents_valid {
                return Err("The saved index contains invalid or duplicate threads.".into());
            }
            for project in &projects {
                if project.captain.is_some_and(|id| id >= profiles.len())
                    || project.captain_thread.is_some_and(|id| {
                        !restored.iter().any(|t| {
                            t.id == id
                                && t.captain == project.captain
                                && t.captain_home
                                && !t.delegated
                        })
                    })
                    || restored
                        .iter()
                        .any(|t| t.project == Some(project.id) && t.machine != project.machine)
                {
                    return Err("Invalid project captain or machine.".into());
                }
            }
            let next_thread = next_counter(
                &index["nextId"],
                restored.iter().map(|t| t.id).max().unwrap_or(0),
            )?;
            let next_project = next_counter(&index["nextProjectId"], maximum_project_id)?;
            self.threads = restored;
            let inferred: std::collections::HashMap<usize, usize> = self
                .threads
                .iter()
                .filter(|thread| {
                    thread.delegated
                        && thread.peer_origin.is_none()
                        && missing_crew.contains(&thread.id)
                        && thread.parent.is_some_and(|parent| {
                            self.threads.iter().any(|t| {
                                t.id == parent
                                    && t.captain_home
                                    && t.captain == thread.captain
                                    && t.captain.is_some()
                            })
                        })
                })
                .filter_map(|thread| thread.captain.map(|captain| (thread.id, captain)))
                .collect();
            for thread in &mut self.threads {
                if let Some(captain) = inferred.get(&thread.id) {
                    thread.crew_captain = Some(*captain);
                }
            }
            // v0.8 kept home flags but hid all except the project's default. Every
            // flagged home is now an independent coordinator, with its original id.
            for thread in &mut self.threads {
                if thread.captain_home {
                    thread.life.settled_override = None;
                    thread.life.settled_at = None;
                    thread.life.snoozed_at = None;
                    thread.life.snoozed_until = None;
                    thread.life.pinned_at = None;
                    thread.life.pin_order_key = None;
                    thread.life.auto_settle_disabled_at = Some(thread.life.created_at);
                }
            }
            self.captains = captains;
            // A different machine's records must not consult the old lifecycle map.
            self.captain_lifecycles.clear();
            self.captain_report_policies.clear();
            self.captain_recovery_modes.clear();
            self.captain_inbox.clear();
            self.captain_waits.clear();
            self.projects.entries = projects;
            self.restore_repository_records(index)?;
            self.restore_captain_records(index, &project_remap)?;
            self.captain_books = self.restore_captain_books(index)?;
            self.restore_crew_reports(index)?;
            self.restore_crew_answers(index)?;
            self.restore_captain_lifecycles(index)?;
            self.restore_captain_report_policies(index)?;
            self.restore_captain_recovery_modes(index)?;
            self.restore_captain_inbox(index)?;
            self.projects.next_id = next_project;
            self.next_id = next_thread;
            if restore_session {
                self.tabs = index["tabs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_u64)
                    .filter_map(|id| usize::try_from(id).ok())
                    .filter(|id| self.threads.iter().any(|t| t.id == *id && !t.archived()))
                    .collect();
                let mut seen_tabs = std::collections::HashSet::new();
                self.tabs.retain(|id| seen_tabs.insert(*id));
                // Indexes saved before ghost tabs have no "ghostTab": keep the
                // active tab and any tab still working, and drop the rest, as
                // KSG Code's migration did when the strip learned ghosts.
                if index["tabs"].is_array() && index.get("ghostTab").is_none() {
                    let active_id = index["active"].as_u64().map(|id| id as usize);
                    // `runtimeInputs` restore after the index, so a held
                    // composer queue counts as work from the record itself.
                    let held_inputs = |id: usize| {
                        index["runtimeInputs"][id.to_string()]
                            .as_array()
                            .is_some_and(|inputs| !inputs.is_empty())
                    };
                    self.tabs.retain(|id| {
                        Some(*id) == active_id
                            || held_inputs(*id)
                            || self.threads.iter().any(|t| {
                                // Runtime status is not persisted; a run the
                                // session can resume, a queued prompt or a
                                // pending request is what "working" leaves
                                // behind in the index.
                                t.id == *id
                                    && (t.provider_running_run.is_some()
                                        || t.resumable_run_id.is_some()
                                        || t.queued_prompt.is_some()
                                        || t.pending_prompt.is_some())
                            })
                    });
                    self.ghost_tab = None;
                    self.ghost_anchor = None;
                } else {
                    self.ghost_tab = index["ghostTab"]
                        .as_u64()
                        .and_then(|id| usize::try_from(id).ok())
                        .filter(|id| self.tabs.contains(id));
                    self.ghost_anchor = index["ghostAnchor"]
                        .as_u64()
                        .and_then(|id| usize::try_from(id).ok())
                        .filter(|id| self.tabs.contains(id));
                    if self.ghost_tab.is_none() {
                        self.ghost_anchor = None;
                    }
                }
                // An archived thread is never reopened at launch. Without any live thread, land on
                // the most recent workspace's draft, as KSG Code's index route does.
                let active = self
                    .threads
                    .iter()
                    .position(|t| Some(t.id as u64) == index["active"].as_u64() && !t.archived())
                    .or_else(|| self.threads.iter().position(|t| !t.archived()));
                if let Some(active) = active {
                    self.active = active;
                    if !self.tabs.contains(&self.current().id) {
                        self.tabs.push(self.current().id);
                    }
                } else {
                    let project = self
                        .projects
                        .entries
                        .iter()
                        .filter(|p| !p.hidden)
                        .max_by_key(|p| (p.updated, p.id))
                        .map(|p| p.id);
                    self.active = self.active.min(self.threads.len().saturating_sub(1));
                    self.create_thread_in(project, &[], &Value::Null);
                }
            } else {
                self.active = 0;
                self.tabs.clear();
            }
        } else {
            return Err("The saved index has no valid thread list.".into());
        }
        self.dark = index["dark"] == true;
        self.tabs_enabled = index["tabsEnabled"] == true;
        self.auto_settle_days = index["autoSettleAfterDays"].as_u64().unwrap_or(3).min(3650) as u32;
        if let Some(zoom) = index["zoom"].as_f64() {
            self.zoom = snap_zoom(zoom as f32);
        }
        self.projects.recents.clone_parent = index["cloneParent"].as_str().map(PathBuf::from);
        self.project_scope = match index["projectScope"].as_str() {
            Some("none") => ProjectScope::NoProject,
            _ => index["projectScope"]
                .as_u64()
                .and_then(|id| usize::try_from(id).ok())
                .and_then(|id| project_remap.get(&id).copied())
                .filter(|id| project_ids.contains(id))
                .map(ProjectScope::Project)
                .unwrap_or(ProjectScope::All),
        };
        if restore_session {
            self.capture_opened_draft();
        }
        self.restore_tasks(index, &project_remap)?;
        if let Some(saved) = index.get("scheduler") {
            self.scheduler = serde_json::from_value(saved.clone())
                .map_err(|error| format!("Invalid saved scheduler: {error}."))?;
            self.scheduler.validate()?;
        }
        self.restore_task_plans(index)?;
        self.requeue_undelivered_managed_inputs();
        self.validate_captain_archives()?;
        self.sync_memory_links();
        self.refresh_captain_contexts();
        if self.persist_transcripts && !self.headless && self.client.is_none() {
            self.restore_persisted_history(&index["threadViews"])?;
        }
        self.repair_execution_bindings();
        Ok(())
    }
    /// Accepted managed input that never reached a provider survives shutdown
    /// as a recovered draft. A still-queued reservation proves the provider
    /// never received it, so the prompt requeues under the same admission
    /// instead of blocking on manual review while the line it owns releases.
    pub(super) fn requeue_undelivered_managed_inputs(&mut self) {
        let ids: Vec<usize> = self.recovered_inputs.keys().copied().collect();
        for id in ids {
            let Some(position) = self.threads.iter().position(|thread| thread.id == id) else {
                continue;
            };
            let covered = self
                .reservation_covering_thread(id)
                .is_some_and(|reservation| {
                    reservation.state == admission::ReservationState::Queued
                });
            if !covered {
                continue;
            }
            let value = self.recovered_inputs[&id].clone();
            let Some(text) = value["text"].as_str() else {
                continue;
            };
            let images: Vec<Attachment> =
                serde_json::from_value(value["attachments"].clone()).unwrap_or_default();
            let Ok(prompt) = crate::composer_tokens::received_prompt_with_context(
                text.into(),
                images,
                value.get("draft").cloned(),
                None,
            ) else {
                continue;
            };
            self.recovered_inputs.remove(&id);
            let at = self.threads[position].messages.len();
            self.threads[position].queued_prompt = Some((prompt, at));
        }
    }
    pub fn keybindings_path(&self) -> PathBuf {
        self.data_dir.join("keybindings.json")
    }
    pub fn ensure_keybindings_file(&mut self) {
        let path = self.keybindings_path();
        match crate::keybindings::sync_default_keybindings_on_startup(
            &path,
            &self.preferences.keybindings,
        ) {
            Ok(file_state) => {
                self.preferences.keybindings = file_state.rules;
                self.settings.keybindings_issues = file_state.issues;
                self.keybindings_watcher.record_stamp(&path);
            }
            Err(error) => {
                self.settings.keybindings_issues =
                    vec![crate::keybindings::KeybindingsIssue::MalformedConfig { message: error }];
            }
        }
    }
    pub fn init_keybindings_file(&mut self, wake: Waker) {
        self.ensure_keybindings_file();
        if !self.demo && !self.headless {
            self.keybindings_watcher
                .start(self.keybindings_path(), wake);
        }
    }
    pub fn poll_keybindings(&mut self) -> bool {
        if self.demo {
            return false;
        }
        let Some(file_state) = self.keybindings_watcher.poll(&self.keybindings_path()) else {
            return false;
        };
        self.preferences.keybindings = file_state.rules;
        self.settings.keybindings_issues = file_state.issues;
        true
    }
    pub fn save_keybinding_override(
        &mut self,
        command: &str,
        key: &str,
        when: Option<&str>,
        replace: Option<(&str, &str, Option<&str>)>,
    ) -> Result<(), String> {
        let path = self.keybindings_path();
        let file_state =
            crate::keybindings::upsert_keybinding_in_file(&path, command, key, when, replace)?;
        self.preferences.keybindings = file_state.rules;
        self.settings.keybindings_issues = file_state.issues;
        self.keybindings_watcher.record_stamp(&path);
        Ok(())
    }
    pub fn reset_keybinding_override(
        &mut self,
        command: &str,
        key: &str,
        when: Option<&str>,
        default_key: Option<&str>,
        default_when: &str,
    ) -> Result<(), String> {
        let path = self.keybindings_path();
        let file_state = crate::keybindings::reset_keybinding_in_file(
            &path,
            command,
            key,
            when,
            default_key,
            default_when,
        )?;
        self.preferences.keybindings = file_state.rules;
        self.settings.keybindings_issues = file_state.issues;
        self.keybindings_watcher.record_stamp(&path);
        Ok(())
    }

    pub(crate) fn ui_snapshot(&self) -> Value {
        let drafts = self
            .threads
            .iter()
            .map(|t| (t.id.to_string(), t.encode_device_draft()))
            .collect();
        let pending = self
            .threads
            .iter()
            .map(|t| (t.id.to_string(), t.pending_inputs_value()))
            .collect();
        self.ui_snapshot_parts(drafts, pending)
    }

    pub(super) fn ui_snapshot_parts(
        &self,
        drafts: serde_json::Map<String, Value>,
        pending: serde_json::Map<String, Value>,
    ) -> Value {
        let mut prefs_value = serde_json::to_value(&self.preferences).unwrap_or_else(|_| json!({}));
        if let Some(map) = prefs_value.as_object_mut() {
            map.remove("keybindings");
        }
        let draft_model_states: serde_json::Map<String, Value> = self
            .threads
            .iter()
            .filter(|t| {
                t.draft_model_state.model_selection_explicit
                    || !t.draft_model_state.additional_models.is_empty()
                    || t.has_draft_content()
            })
            .filter(|t| {
                !t.draft_model_state.model_selection_by_provider.is_empty()
                    || t.draft_model_state.active_provider.is_some()
                    || !t.draft_model_state.additional_models.is_empty()
            })
            .filter_map(|t| {
                serde_json::to_value(&t.draft_model_state)
                    .ok()
                    .map(|val| (t.id.to_string(), val))
            })
            .collect();
        let mut value = json!({
            "background": self.background.settings,
            "preferences": prefs_value,
            "draftModelStates": draft_model_states,
            "version":1, "shelves":self.shelves, "sidebar":self.sidebar, "sidebarWidth":self.sidebar_width,
            "sidebarWidthDefault":DEFAULT_SIDEBAR_WIDTH,
            "detailsCard":self.details.docked,
            "rightPanelWidth":self.right_panel_width,
            "scope": match self.project_scope { ProjectScope::All => json!("all"),
                ProjectScope::NoProject => json!("none"), ProjectScope::Project(id) => json!(id) },
            "scopeAdditional":self.scope_additional,
            "scopeServer":self.machines.scope_server,
            "scopeRemote":self.machines.scope_remote,
            "pendingMessages":pending,
            "drafts":drafts,
        });
        // Off is the default, so only the on state is saved.
        if self.diff_hide_tests {
            value["diffHideTests"] = json!(true);
        }
        // KSG's file tree defaults hidden; only a turn-on and a chosen lane
        // width are saved.
        if self.diff_tree_open {
            value["diffTreeOpen"] = json!(true);
        }
        if let Some(width) = self.diff_tree_width {
            value["diffTreeWidth"] = json!(width);
        }
        value["tabsEnabled"] = json!(self.tabs_enabled);
        value
    }
    pub(crate) fn runtime_input_snapshot(
        &self,
        scope: Option<&std::collections::HashSet<usize>>,
    ) -> Value {
        let pending = self
            .threads
            .iter()
            .filter(|t| scope.is_none_or(|s| s.contains(&t.id)))
            .map(|t| (t.id.to_string(), t.pending_inputs_value()))
            .collect::<serde_json::Map<_, _>>();
        if scope.is_some() {
            json!({"_partial":true,"pendingMessages":pending})
        } else {
            json!({"version":1,"pendingMessages":pending,"background":{"keep_awake":self.background_settings().keep_awake}})
        }
    }
    pub(crate) fn restore_ui(&mut self, ui: &Value) -> Result<(), String> {
        if ui["version"] != 1 {
            return Err("Unsupported UI state. Existing files have not been overwritten.".into());
        }
        self.background.settings = serde_json::from_value(
            ui.get("background").cloned().unwrap_or(json!({})),
        )
        .map_err(|_| "Invalid background settings. Existing UI state has not been overwritten.")?;
        self.preferences =
            serde_json::from_value(ui.get("preferences").cloned().unwrap_or(json!({})))
                .map_err(|_| "Invalid UI preferences.")?;
        self.shelves =
            serde_json::from_value(ui["shelves"].clone()).map_err(|_| "Invalid shelf state.")?;
        self.sidebar = ui["sidebar"].as_bool().ok_or("Invalid sidebar state.")?;
        self.sidebar_width = ui
            .get("sidebarWidth")
            .map(|v| {
                v.as_f64()
                    .filter(|n| n.is_finite())
                    .map(|n| n.clamp(SIDEBAR_MIN as f64, SIDEBAR_MAX as f64) as f32)
                    .ok_or("Invalid sidebar width.")
            })
            .transpose()?
            .unwrap_or(DEFAULT_SIDEBAR_WIDTH);
        // Profiles still on a default move to the new one. A width saved equal to the
        // layout's own default was never chosen; a width that differs from the saved
        // default was, and stays as the user left it.
        let still_on_default = match ui.get("sidebarWidthDefault").and_then(Value::as_f64) {
            Some(saved) => self.sidebar_width == saved as f32,
            None => {
                self.sidebar_width == PREVIOUS_DEFAULT_SIDEBAR_WIDTH || self.sidebar_width == 336.0
            }
        };
        if still_on_default {
            self.sidebar_width = DEFAULT_SIDEBAR_WIDTH;
        }
        self.details.docked = ui
            .get("detailsCard")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        // Panel state itself is session-only; the width is the one saved value.
        self.right_panel_width = ui
            .get("rightPanelWidth")
            .map(|v| {
                v.as_f64()
                    .filter(|n| n.is_finite())
                    .map(|n| n.max(crate::right_panel::MIN_WIDTH as f64) as f32)
                    .ok_or("Invalid right panel width.")
            })
            .transpose()?
            .unwrap_or(crate::right_panel::DEFAULT_WIDTH);
        self.diff_hide_tests = ui["diffHideTests"].as_bool().unwrap_or(false);
        self.diff_tree_open = ui["diffTreeOpen"].as_bool().unwrap_or(false);
        self.diff_tree_width = ui
            .get("diffTreeWidth")
            .and_then(|v| v.as_f64())
            .filter(|n| n.is_finite())
            .map(|n| {
                n.clamp(
                    crate::ui::diff_tree::MIN_W as f64,
                    crate::ui::diff_tree::MAX_W as f64,
                ) as f32
            });
        if let Some(enabled) = ui.get("tabsEnabled").and_then(Value::as_bool) {
            self.tabs_enabled = enabled;
        }
        self.project_scope = match ui["scope"].as_str() {
            Some("all") => ProjectScope::All,
            Some("none") => ProjectScope::NoProject,
            _ => ProjectScope::Project(
                ui["scope"]
                    .as_u64()
                    .and_then(|id| usize::try_from(id).ok())
                    .ok_or("Invalid project scope.")?,
            ),
        };
        self.scope_additional = serde_json::from_value(ui["scopeAdditional"].clone())
            .map_err(|_| "Invalid project scopes.")?;
        self.machines.scope_server = ui["scopeServer"].as_str().map(str::to_owned);
        self.machines.scope_remote =
            serde_json::from_value(ui.get("scopeRemote").cloned().unwrap_or(json!([])))
                .map_err(|_| "Invalid remote project scopes.")?;
        if let ProjectScope::Project(id) = self.project_scope
            && self
                .machines
                .scope_server
                .as_ref()
                .is_none_or(|owner| owner == &self.machines.selected)
            && self.project(Some(id)).is_none()
            && self
                .scope_additional
                .iter()
                .all(|id| self.project(Some(*id)).is_none())
        {
            self.project_scope = ProjectScope::All;
        }
        for (id, draft) in ui["drafts"].as_object().ok_or("Invalid saved drafts.")? {
            let (text, images, missing_images) = if let Some(text) = draft.as_str() {
                (text, Vec::new(), false)
            } else {
                let text = draft["text"].as_str().ok_or("Invalid draft text.")?;
                let saved_images = draft["images"]
                    .as_array()
                    .ok_or("Invalid saved draft images.")?;
                let images: Vec<_> = saved_images
                    .iter()
                    .filter_map(Attachment::from_value)
                    .collect();
                let missing = images.len() != saved_images.len();
                (text, images, missing)
            };
            if let Some(t) = id
                .parse::<usize>()
                .ok()
                .and_then(|id| self.threads.iter_mut().find(|t| t.id == id))
            {
                t.draft = crate::composer_rich::restore(text, draft["richHtml"].as_str());
                t.composer_source = Default::default();
                crate::composer_tokens::stamp_thread_owner(&mut t.draft, &t.machine);
                t.draft_images = images;
                t.draft_context = draft
                    .get("context")
                    .filter(|v| !v.is_null())
                    .and_then(|v| crate::composer_context::decode_message_context(v).ok())
                    .map(|c| c.records)
                    .unwrap_or_default();
                t.draft_reply_to = draft
                    .get("replyTo")
                    .filter(|v| !v.is_null())
                    .and_then(|v| {
                        serde_json::from_value::<crate::composer_context::ComposerContextRecord>(
                            v.clone(),
                        )
                        .ok()
                    })
                    .filter(|r| {
                        matches!(
                            r.data,
                            crate::composer_context::ComposerContextData::Response { .. }
                        )
                    });
                t.retained_context = Default::default();
                if draft["freshSession"] == true {
                    let arm = draft["freshSessionArm"].as_u64().unwrap_or_else(|| {
                        self.fresh_arm_seq = self.fresh_arm_seq.saturating_add(1).max(1);
                        self.fresh_arm_seq
                    });
                    self.fresh_arm_seq = self.fresh_arm_seq.max(arm);
                    t.fresh_session_arm = Some(arm);
                } else {
                    t.fresh_session_arm = None;
                }
                t.attachment_error = if missing_images {
                    Some("Some saved draft images are missing or invalid. Attach those images again.".into())
                } else {
                    attachments::validate_prompt_images(&t.draft_images).err()
                };
            }
        }
        if let Some(pending) = ui["pendingMessages"].as_object() {
            for (id, value) in pending {
                if let Some(thread) = id
                    .parse::<usize>()
                    .ok()
                    .and_then(|id| self.threads.iter_mut().find(|t| t.id == id))
                {
                    thread.restore_pending_inputs(value)?;
                }
            }
        }
        if let Some(draft_models) = ui.get("draftModelStates").and_then(Value::as_object) {
            for (id, value) in draft_models {
                if let Some(thread) = id
                    .parse::<usize>()
                    .ok()
                    .and_then(|id| self.threads.iter_mut().find(|t| t.id == id))
                    && let Ok(state) = serde_json::from_value(value.clone())
                {
                    thread.draft_model_state = state;
                }
            }
        }
        self.capture_opened_draft();
        self.sync_fresh_session_notice();
        Ok(())
    }
    pub fn checkpoint(&mut self) {
        if self.threads.iter().any(|t| t.answer_intake) {
            return;
        }
        if self.demo {
            return;
        }
        if self.client.is_some() {
            self.checkpoint_client();
            return;
        }
        // The runtime checkpoints once, after the entire reducer batch and before
        // dispatch. Reducer helpers must not serialize intermediate workspaces.
        if self.headless && crate::database::for_path(&self.data_dir.join("index.json")).is_some() {
            return;
        }
        let snapshot = self.snapshot();
        if let Some(store) = &mut self.store {
            store.save(snapshot);
        }
        let mut ui = self.ui_snapshot();
        if self.headless && crate::database::for_path(&self.data_dir.join("index.json")).is_some() {
            ui = json!({"version":1,"pendingMessages":ui["pendingMessages"],
                "background":{"keep_awake":ui["background"]["keep_awake"]}});
        }
        if let Some(store) = &mut self.ui_store {
            store.save(ui);
        }
    }
    /// Starts both stores against this state's data folder, as `setup` does, without Pi.
    #[cfg(test)]
    pub fn load_global_for_test(&mut self) {
        self.load_global(self.wake.clone());
    }

    /// Saves both stores and waits until the writes are on disk. A disabled or failed save
    /// blocks an update restart.
    pub fn checkpoint_and_flush(&mut self) -> Result<(), String> {
        self.checkpoint();
        self.flush_stores()
    }

    /// The commit an admission grant performs before its provider effect.
    /// Separate from `checkpoint_and_flush` so a test can fault exactly
    /// this boundary without breaking unrelated commits in the process.
    pub(crate) fn admission_commit(&mut self) -> Result<(), String> {
        #[cfg(debug_assertions)]
        if crate::outbox::commit_fault(&self.data_dir) {
            return Err("Injected commit failure for integration testing.".into());
        }
        self.checkpoint_and_flush()
    }

    #[cfg(test)]
    pub fn take_restart_request(&mut self) -> bool {
        std::mem::take(&mut self.restart_approved)
    }

    pub fn cancel_update_exit(&mut self) {
        self.restart_approved = false;
        self.restart_for_update = false;
        crate::update::take_exit();
    }

    /// Says why a restart waits, next to the update row rather than as a thread error.
    pub fn report_restart_blocker(&mut self, reason: String) {
        self.record_fleet_restart_result(&reason);
        self.restart_notice = Some(reason);
    }

    pub(super) fn flush_stores(&mut self) -> Result<(), String> {
        if self.client.is_some() {
            return self
                .ui_store
                .as_ref()
                .ok_or("Client drafts are not saving.")?
                .flush();
        }
        if self.demo {
            return Err("Demo runs do not save.".into());
        }
        let (Some(store), Some(ui)) = (self.store.as_ref(), self.ui_store.as_ref()) else {
            return Err("Shina is not saving this session.".into());
        };
        // Flush both even if the first one failed.
        let index = store.flush();
        let drafts = ui.flush();
        let memory = self.flush_memory();
        index.and(drafts).and(memory)
    }

    /// Display copy and available actions shared by the update control and Settings.
    pub fn update_summary(&self) -> crate::update_status::UpdateSummary {
        use crate::fleet_update::PillTone;
        use crate::update::Status;
        use crate::update_status::{MachineUpdate, UpdateAction, UpdatePhase::*, UpdateSummary};

        let last_check = self.updater.last_check_label().to_owned();
        if self.demo
            || !self.updater.self_update_enabled() && self.updater.status() == &Status::Disabled
        {
            return UpdateSummary::from_phase(
                Disabled,
                None,
                vec![],
                vec![],
                last_check,
                None,
                false,
            );
        }
        let mut raw: Vec<String> = self.updater.diagnostics().cloned().collect();
        let failure = self.updater.failed_release_detail();
        let notice = self.updater.notice();
        let fleet_failure = self
            .fleet
            .result_notice
            .as_ref()
            .filter(|notice| !notice.success);
        let pill_failure = self
            .fleet
            .pill
            .as_ref()
            .filter(|pill| matches!(pill.tone, PillTone::Warning | PillTone::Error));
        raw.extend(failure.iter().map(|(_, reason)| reason.clone()));
        if let Some(notice) = notice
            && !raw.iter().any(|line| line == notice)
        {
            raw.push(notice.into());
        }
        raw.extend(
            fleet_failure
                .into_iter()
                .flat_map(|notice| notice.lines.iter().cloned()),
        );
        raw.extend(pill_failure.into_iter().map(|pill| pill.title.clone()));
        for row in self
            .fleet
            .rows
            .iter()
            .filter(|row| row.result == Some(false))
        {
            raw.extend(row.provider_failure.iter().cloned());
            raw.extend(row.state.strip_prefix("failed: ").map(str::to_owned));
        }

        let running = self.fleet.running || self.fleet_run_visible();
        let provider_offers = self.all_provider_candidates();
        let server_offers = self.server_update_offers();
        let mut machines: Vec<_> = self
            .fleet
            .rows
            .iter()
            .map(|row| MachineUpdate {
                is_app: false,
                server: row.server.clone(),
                name: row.name.clone(),
                status: if running {
                    crate::fleet_update::status_word(self.run_status(row)).into()
                } else {
                    row.state.clone()
                },
                current_version: (!row.current.is_empty()).then(|| row.current.clone()),
                target_version: (!row.target.is_empty()).then(|| row.target.clone()),
                action: (!running && row.eligible)
                    .then(|| UpdateAction::UpdateMachine(row.server.clone())),
            })
            .collect();
        let known: std::collections::BTreeSet<&String> = self
            .client
            .iter()
            .flat_map(|client| client.linked.keys())
            .chain(server_offers.keys())
            .chain(provider_offers.iter().map(|(server, _, _, _)| server))
            .collect();
        for server in known {
            if machines.iter().any(|machine| machine.server == *server) {
                continue;
            }
            let target = server_offers.get(server).cloned();
            let can_update =
                target.is_some() || provider_offers.iter().any(|(host, _, _, _)| host == server);
            machines.push(MachineUpdate {
                is_app: false,
                name: self.machine_label(server),
                status: if can_update {
                    "Update available"
                } else if self.machine_connected(server) {
                    "Up to date"
                } else {
                    "Offline"
                }
                .into(),
                current_version: self
                    .machines
                    .views
                    .get(server)
                    .and_then(|view| view["updateStatus"]["runningVersion"].as_str())
                    .map(str::to_owned),
                target_version: target,
                action: (!running && can_update)
                    .then(|| UpdateAction::UpdateMachine(server.clone())),
                server: server.clone(),
            });
        }

        let mut detail = None;
        let phase = if running {
            let capsule = self.describe_capsule();
            detail = Some(if let Some(elapsed) = capsule.elapsed {
                format!("{} · {elapsed}", capsule.label)
            } else {
                capsule.label
            });
            let include_app = self.fleet.local_version.is_some()
                && !self.fleet.rows.iter().any(|row| row.server == "local");
            FleetRunning {
                done: self
                    .fleet
                    .rows
                    .iter()
                    .filter(|row| self.run_status(row) == "done")
                    .count(),
                total: self.fleet.rows.len() + usize::from(include_app),
            }
        } else {
            match self.updater.status() {
                Status::Ready { update } => Ready {
                    tag: update.tag.clone(),
                    blocker: self.update_restart_blocker().map(str::to_owned),
                },
                Status::Downloading { tag } => Downloading {
                    tag: tag.clone(),
                    progress: None,
                },
                Status::Checking => Checking,
                _ if failure.is_some()
                    || notice.is_some()
                    || fleet_failure.is_some()
                    || pill_failure.is_some() =>
                {
                    Failed {
                        tag: failure.map(|(tag, _)| tag),
                    }
                }
                _ if !server_offers.is_empty() || !provider_offers.is_empty() => {
                    detail = self.update_everything_summary();
                    let tag = server_offers
                        .values()
                        .next()
                        .cloned()
                        .or_else(|| provider_offers.first().map(|(_, _, _, tag)| tag.clone()))
                        .unwrap_or_default();
                    Available {
                        tag,
                        targets: server_offers.len() + provider_offers.len(),
                    }
                }
                _ if self.updater.success_up_to_date(Instant::now()) => UpToDate,
                _ => Idle,
            }
        };
        let retry_at = self.updater.retry_at();
        let mut summary = UpdateSummary::from_phase(
            phase,
            detail,
            raw,
            machines,
            last_check,
            retry_at.as_deref(),
            retry_at.is_none(),
        );
        if !summary.machines.is_empty() {
            summary.machines.insert(
                0,
                MachineUpdate {
                    is_app: true,
                    server: "local".into(),
                    name: "This app".into(),
                    status: self
                        .capsule_local_status()
                        .unwrap_or_else(|| summary.headline.clone()),
                    current_version: Some(summary.version.into()),
                    target_version: match &summary.phase {
                        Ready { tag, .. } | Downloading { tag, .. } => Some(tag.clone()),
                        _ => self.fleet.local_version.clone(),
                    },
                    action: summary
                        .action
                        .clone()
                        .filter(|action| !matches!(action, UpdateAction::UpdateEverything)),
                },
            );
        }
        if self.fleet.postponed_local && matches!(summary.phase, Ready { blocker: None, .. }) {
            summary.action = Some(UpdateAction::RestartPostponed);
        }
        if self.fleet.phase == crate::fleet_update::RunPhase::Countdown {
            summary.action = Some(UpdateAction::Postpone);
        }
        summary.dismissible = self.fleet.result_notice.is_some()
            || self
                .fleet
                .pill
                .as_ref()
                .is_some_and(|pill| pill.dismissible);
        if matches!(summary.phase, Idle | UpToDate)
            && let Some(notice) = self
                .fleet
                .result_notice
                .as_ref()
                .filter(|notice| notice.success)
        {
            summary.detail.push_str(&format!(" {}", notice.title));
        }
        summary
    }

    pub fn update_restart_blocker(&self) -> Option<&str> {
        if !self.provider_update_drivers.is_empty() {
            return Some("A provider is updating. Restart when it finishes.");
        }
        self.provider_work_blocker()
    }

    pub(crate) fn provider_work_blocker(&self) -> Option<&str> {
        if self.client.is_some() {
            if self.rename_thread.is_some() || self.captain_editor.is_some() {
                return Some("Finish editing before restarting.");
            }
            // Updating this desktop concerns Local, not the remote thread being read.
            if let Some(blocker) = self
                .machines
                .views
                .get("local")
                .and_then(|view| view["updateStatus"]["blocker"].as_str())
            {
                return Some(blocker);
            }
            if self.machines.selected != "local" {
                return None;
            }
        }
        if self.routines_running() {
            return Some("A routine is still running. Restart when it finishes.");
        }
        if self.tasks.iter().any(|task| {
            matches!(
                task.state,
                TaskState::Queued | TaskState::Starting | TaskState::Running
            )
        }) {
            return Some("A delegated task is running or queued. Restart when it finishes.");
        }
        // A provider can be starting or applying model settings before any work is sent.
        if self
            .threads
            .iter()
            .any(|t| t.status.busy() || t.queued() || t.request.is_some())
        {
            return Some("A thread is still working. Restart when it finishes.");
        }
        if self.projects.cloning() {
            return Some("A repository is still cloning.");
        }
        if self.rename_thread.is_some() || self.captain_editor.is_some() {
            return Some("Finish editing before restarting.");
        }
        None
    }

    /// Only an explicit restart installs updates. Every normal quit still saves and flushes.
    pub fn prepared_exit(
        &mut self,
        explicit: bool,
        quit_reply: Option<&mut String>,
    ) -> Result<(), String> {
        if !explicit {
            self.cancel_update_exit();
            return self.checkpoint_and_flush();
        }
        let blocker = self.update_restart_blocker();
        if explicit && let Some(blocker) = blocker {
            if let Some(reply) = quit_reply {
                *reply = blocker.to_owned();
            }
            return Err("The update cannot restart yet.".into());
        }
        let update = self
            .updater
            .prepared_update()
            .filter(|_| !self.demo && blocker.is_none())
            .cloned();
        let Some(update) = update else {
            if explicit {
                if let Some(reply) = quit_reply {
                    *reply = "The update is not ready yet.".into();
                }
                return Err("The update is not ready yet.".into());
            }
            return Ok(());
        };
        // A failed save blocks the install. Keep the download for another restart attempt.
        if let Err(error) = self.checkpoint_and_flush() {
            if let Some(reply) = quit_reply {
                *reply = error.clone();
            }
            return Err(error);
        }
        if self.client.is_some() {
            if !self
                .client
                .as_ref()
                .is_some_and(|client| client.commands.connected())
            {
                return Err("Reconnect to the local server before requesting an update.".into());
            }
            crate::update::start_coordinated_helper(&update)?;
            self.restart_for_update = false;
            self.restart_notice =
                Some("Waiting for the server and local clients to save before restarting.".into());
            return Err(
                "Coordinated restart requested. This window will close after every client saves."
                    .into(),
            );
        }
        crate::update::record_exit(crate::update::ExitUpdate {
            update,
            relaunch: explicit,
        });
        self.restart_approved |= explicit;
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Value {
        self.snapshot_scoped(None)
    }
    pub(crate) fn snapshot_scoped(
        &self,
        scope: Option<&std::collections::HashSet<usize>>,
    ) -> Value {
        if let Some(scope) = scope {
            let mut index = json!({"_partial":true,
                "threads":self.threads.iter().filter(|t|scope.contains(&t.id)).map(Thread::stored_metadata).collect::<Vec<_>>(),
                "threadViews":self.thread_views_scoped(Some(scope)),
                "runtimeInputs":self.threads.iter().filter(|t|scope.contains(&t.id)).map(|t|(t.id.to_string(),t.runtime_inputs_value())).collect::<serde_json::Map<_,_>>(),
                "execution":self.execution
            });
            self.snapshot_identity().annotate_index(&mut index);
            return index;
        }
        let mut index = json!({
            "version":crate::identity::INDEX_VERSION, "dark":self.dark, "zoom":self.zoom, "active":self.current().id, "tabs":self.tabs,
            "tabsEnabled":self.tabs_enabled, "ghostTab":self.ghost_tab, "ghostAnchor":self.ghost_anchor,
            "nextId":self.next_id, "nextProjectId":self.projects.next_id,
            "autoSettleAfterDays": self.auto_settle_days,
            "defaultHarness": self.default_harness,
            "threadDefaults": self.thread_defaults,
            "modelFavorites": self.model_favorites,
            "execution": self.execution,
            "tasks": self.tasks,
            "projects": self.projects.entries.iter().map(Project::to_value).collect::<Vec<_>>(),
            "cloneParent": self.projects.recents.clone_parent,
            "captains":self.captains.iter().map(Captain::to_value).collect::<Vec<_>>(),
            "captainScopes":self.captain_scopes,
            "captainRemoteScopes":self.captain_remote_scopes,
            "repositories":self.repositories,
            "repositoryBindings":self.repository_bindings,
            "captainRepositoryScopes":self.captain_repository_scopes,
            "captainHomes":self.captain_homes,
            "captainBooks":self.captain_book_value(),
            "captainLifecycles":self.captain_lifecycles,
            "captainReportPolicies":self.captain_report_policies,
            "captainReports":self.captain_reports,
            "captainQuestionAnswers":self.captain_question_answers,
            "captainRecoveryModes":self.captain_recovery_modes,
            "captainInbox":self.captain_inbox,
            "captainWaits":self.captain_waits,
            "threads":self.threads.iter().filter(|thread| {
                // A blank draft is saved only while it is open or has a tab.
                self.headless || self.persist_transcripts || !thread.blank_draft() || thread.crew_captain.is_some() || thread.id == self.current().id || self.tabs.contains(&thread.id)
            }).map(Thread::stored_metadata).collect::<Vec<_>>()
        });
        index["scheduler"] = json!(self.scheduler);
        index["taskPlans"] = json!(self.task_plans);
        index["commandReceipts"] = json!(self.command_receipts);
        index["commandIntents"] = json!(self.command_intents);
        // Bounded projection only. The durable table keeps every operation;
        // task_status keeps the latest 16 for the model payload.
        let projected_operations: Vec<_> = self
            .task_operations
            .iter()
            .rev()
            .take(64)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        index["taskOperations"] = json!(projected_operations);
        index["serverEpoch"] = json!(self.server_epoch);
        index["serverSeq"] = json!(self.server_seq);
        index["recoveredInputs"] = json!(self.recovered_inputs);
        if self.headless
            || self.persist_transcripts
            || self
                .threads
                .iter()
                .any(|t| !t.fresh_session_messages.is_empty())
        {
            index["threadViews"] = self.thread_views();
            index["runtimeInputs"] = json!(
                self.threads
                    .iter()
                    .map(|thread| (thread.id.to_string(), thread.runtime_inputs_value()))
                    .collect::<serde_json::Map<_, _>>()
            );
        }
        self.snapshot_identity().annotate_index(&mut index);
        index
    }
    pub(crate) fn snapshot_identity(&self) -> crate::identity::Catalog {
        self.identity.with_current(
            0..self.captains.len(),
            self.projects.entries.iter().map(|project| project.id),
            self.threads.iter().map(|thread| thread.id),
            self.tasks.iter().map(|task| task.id.clone()),
            self.background
                .routines
                .entries()
                .iter()
                .map(|routine| routine.id),
        )
    }
    pub(crate) fn populate_sidebar_cards_demo(&mut self) {
        self.populate_demo();
        self.dark = true;
        let thread = &mut self.threads[0];
        thread.title = "Intermittent Thread Stoppages".into();
        thread.branch = Some("work/sable-dogfood".into());
        self.captain_reports.push(crew_reports::CrewReport {
            id: "demo-sidebar-pr".into(),
            captain: 0,
            thread_id: thread.id,
            run_id: "demo-sidebar-run".into(),
            title: thread.title.clone(),
            outcome: crew_reports::CrewOutcome::Completed,
            policy: Default::default(),
            summary: "Example pull request reference.".into(),
            error: String::new(),
            created_at: lifecycle::now(),
            request_id: None,
            question: None,
            artifacts: vec![crew_reports::CrewArtifact {
                kind: crew_reports::ArtifactKind::PullRequest,
                label: "Example pull request".into(),
                reference: "https://github.com/example/shina/pull/84".into(),
            }],
            delivered: true,
            hold_delivery: false,
        });
    }

    pub fn populate_demo(&mut self) {
        self.demo = true;
        self.captains = Captain::presets();
        self.threads.clear();
        let mut thread = Thread::new(1);
        thread.title = "Fix the reconnect listener".into();
        thread.captain = Some(0);
        thread.status = Status::Completed;
        thread.model = DEFAULT_MODEL.into();
        thread.thinking = "medium".into();
        thread.elapsed = Some(42);
        thread.work.push(AssistantWork {
            start: 1,
            stopped: false,
            stopped_order: 0,
            expanded: true,
            elapsed: Some(42),
            outcome: Some(Status::Completed),
        });
        thread.messages = vec![
            Message { id: uuid::Uuid::new_v4().to_string(), role: "You", text: "Fix the reconnect race, and add a regression test for the duplicate listener.".into(), images: Vec::new(), context: None },
            Message { id: uuid::Uuid::new_v4().to_string(), role: "Pi", text: "I'll check the socket lifecycle and where the reconnect listener attaches before making the change.".into(), images: Vec::new(), context: None },
            Message { id: uuid::Uuid::new_v4().to_string(), role: "Pi", text: "The reconnect listener attaches too late. I'll move it before `socket.open()` and cover the initialization race.".into(), images: Vec::new(), context: None },
            Message { id: uuid::Uuid::new_v4().to_string(), role: "Pi", text: "The listener now attaches before the socket opens. Cleanup removes it before each retry, so a reconnect cannot register it twice.\n\n```diff\n- socket.open();\n  socket.on(\"reconnect\", reconnect);\n+ socket.open();\n```\n\nThe regression test passes. Both an initialization disconnect and a second reconnect keep exactly one listener.".into(), images: Vec::new(), context: None },
        ];
        thread.tools = vec![
            Tool {
                id: "example-1".into(),
                kind: None, changes: vec![], input: Value::Null, raw_expanded: false,
                order: 0,
                name: "read".into(),
                detail: "src/transport/socket.ts".into(),
                done: true,
                failed: false,
                output: "Example activity, not a real tool execution.".into(),
                anchor: ToolAnchor { message: 2, offset: 0 },
                block: 0,
                expanded: false,
                group_expanded: false,
                started: None,
                duration_ms: Some(180),
                subagents: vec![],
                images: vec![], structured: Value::Null, parent_call: None, exit_code: None,
            },
            Tool {
                id: "example-2".into(),
                kind: None, changes: vec![], input: Value::Null, raw_expanded: false,
                order: 0,
                name: "read".into(),
                detail: "tests/reconnect.test.ts".into(),
                done: true,
                failed: false,
                output: "Example activity, not a real tool execution.".into(),
                anchor: ToolAnchor { message: 2, offset: 0 },
                block: 1,
                expanded: false,
                group_expanded: false,
                started: None,
                duration_ms: Some(120),
                subagents: vec![],
                images: vec![], structured: Value::Null, parent_call: None, exit_code: None,
            },
            Tool {
                id: "example-3".into(),
                kind: None, changes: vec![], input: Value::Null, raw_expanded: false,
                order: 0,
                name: "edit".into(),
                detail: "src/transport/socket.ts".into(),
                done: true,
                failed: false,
                output: "Example output\nReplaced 2 lines in src/transport/socket.ts.".into(),
                anchor: ToolAnchor { message: 3, offset: 0 },
                block: 0,
                expanded: false,
                group_expanded: true,
                started: None,
                duration_ms: Some(240),
                subagents: vec![],
                images: vec![], structured: Value::Null, parent_call: None, exit_code: None,
            },
            Tool {
                id: "example-4".into(),
                kind: None, changes: vec![], input: Value::Null, raw_expanded: false,
                order: 0,
                name: "powershell".into(),
                detail: "npm test -- reconnect.test.ts".into(),
                done: true,
                failed: false,
                output: "Example output\nPASS tests/reconnect.test.ts\n  reconnect lifecycle\n    attaches before opening the socket\n    removes the listener before retrying\n\nTests: 2 passed, 2 total".into(),
                anchor: ToolAnchor { message: 3, offset: 0 },
                block: 1,
                expanded: false,
                group_expanded: false,
                started: None,
                duration_ms: Some(1100),
                subagents: vec![],
                images: vec![], structured: Value::Null, parent_call: None, exit_code: None,
            },
        ];
        thread.thoughts.push(Thought {
            order: 0,
            anchor: ToolAnchor { message: 2, offset: 0 },
            block: 2,
            text: "Example thinking, not a real model response.\n\nThe listener must attach before socket.open() can emit a reconnect. Cleanup must remove the old listener before another attempt.".into(),
            expanded: false,
            done: true,
            started: None,
            duration_ms: Some(18000),
        });
        self.threads.push(thread);
        for (id, title, status) in [
            (2, "Review sidebar keys", Status::Running),
            (3, "Reduce cold-start work", Status::Completed),
            (4, "Add a native file picker", Status::Completed),
            (5, "Plan the Pi session adapter", Status::Completed),
            (6, "Audit the Markdown renderer", Status::Ready),
        ] {
            let mut thread = Thread::new(id);
            thread.title = title.into();
            thread.status = status;
            thread.life.pinned_at = (id == 3).then_some(lifecycle::now());
            if id == 2 {
                thread.captain = Some(0);
            }
            self.threads.push(thread);
        }
        // One home per captain, in different states, so the crew shows its range.
        for (id, captain, status) in [
            (7, 0, Status::Ready),
            (8, 1, Status::Waiting),
            (9, 2, Status::Completed),
            (10, 3, Status::Ready),
        ] {
            let mut thread = Thread::new(id);
            thread.title = crate::visuals::CAPTAIN_NAMES[captain].into();
            thread.captain = Some(captain);
            thread.captain_home = true;
            thread.status = status;
            self.threads.push(thread);
        }
        self.next_id = 11;
        self.active = 0;
        self.projects = crate::projects::Projects::demo();
        self.projects.entries = ["shina", "rune", "ksgcode"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let mut project = Project::new(i + 1, PathBuf::from("/workspaces").join(name));
                project.captain = Some(i);
                project.captain_thread = Some(i + 7);
                project.updated = crate::projects::now().saturating_sub(i as u64 * 120);
                project
            })
            .collect();
        self.projects.next_id = 4;
        for thread in &mut self.threads {
            thread.project = match thread.id {
                1 | 2 | 7 => Some(1),
                3 | 4 | 8 => Some(2),
                5 | 9 => Some(3),
                _ => None,
            };
            thread.captain = thread.project.map(|id| id - 1);
            thread.captain_home = matches!(thread.id, 7..=9);
            thread.updated = crate::projects::now().saturating_sub(thread.id as u64 * 60);
        }
        self.threads[9].title = "Plan a weekend tool".into();
        self.threads[1].life.last_completed_at = Some(lifecycle::now());
        self.threads[1].life.last_visited_at = Some(0);
        for t in &mut self.threads {
            t.life.created_at = t.updated.saturating_mul(1000);
            t.life.updated_at = t.life.created_at;
            if !t.captain_home {
                t.life.latest_user_message_at = Some(t.life.created_at);
            }
        }
        self.threads[2].life.pin_order_key = Some("n".into());
        self.threads[3]
            .life
            .snooze(lifecycle::now(), lifecycle::now() + 10_800_000);
        self.threads[4].life.settle(lifecycle::now() - 86_400_000);
        self.shelves = [false; 3];
        self.undo = None;
        self.tabs = vec![1, 3, 10];
    }
}

fn next_counter(saved: &Value, maximum: usize) -> Result<usize, String> {
    let minimum = maximum.checked_add(1).ok_or("Invalid id counter.")?;
    let next = if saved.is_null() {
        minimum
    } else {
        usize::try_from(saved.as_u64().ok_or("Invalid id counter.")?)
            .map_err(|_| "Invalid id counter.")?
            .max(minimum)
    };
    if next == usize::MAX {
        return Err("Thread or project ids exhausted.".into());
    }
    Ok(next)
}
