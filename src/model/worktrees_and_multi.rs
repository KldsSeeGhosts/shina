use super::*;

impl State {
    pub fn send(&mut self) {
        let _ = self.dispatch_send();
    }

    /// Ported from KSG Code's `useComposerSend` (`onSend` with `submissionIntent: "background"`):
    /// on a draft thread, starts in the background and opens a fresh composer; on an existing
    /// thread, dispatches the turn and opens a new draft once accepted.
    pub fn send_background(&mut self) {
        self.send_background_intent(false);
    }

    pub(super) fn send_background_intent(&mut self, approval_redirect: bool) {
        if !self
            .current()
            .draft_model_state
            .additional_models
            .is_empty()
            && !self.demo
            && !self.ownership_denied
            && !self.current().archived()
        {
            let _ = self.dispatch_send_intent(true, approval_redirect);
            return;
        }
        let source_index = self.active;
        let source_id = self.current().id;
        let was_draft = self.current().is_draft();
        let project = self.current().project;
        let seed = self.composer_seed();
        let server = self.machines.selected.clone();

        if !self.dispatch_send_intent(true, approval_redirect) {
            return;
        }
        if self.client.is_some() {
            if was_draft {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == source_id) {
                    thread.background_send = Some(server);
                }
                self.new_client_draft_with_seed(project, &seed);
                if !self.headless {
                    self.restore_composer_focus(crate::ui::composer_ui::FocusReason::ThreadOpened);
                }
                self.checkpoint();
            } else if let Some((command_id, _)) = self
                .threads
                .iter()
                .find(|t| t.id == source_id)
                .and_then(|t| t.remote_send.as_ref())
            {
                let command_id = *command_id;
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == source_id) {
                    thread.pending_send_and_new_thread = Some((command_id, server));
                }
            }
            return;
        }

        if was_draft {
            self.create_thread_in(project, &[source_id], &seed);
            if !self.headless {
                self.restore_composer_focus(crate::ui::composer_ui::FocusReason::ThreadOpened);
            }
            let failed = self
                .threads
                .iter()
                .find(|t| t.id == source_id)
                .is_some_and(|t| t.status == Status::Failed);
            if failed {
                let error = self
                    .threads
                    .iter()
                    .find(|t| t.id == source_id)
                    .and_then(|t| t.error.clone())
                    .unwrap_or_else(|| "Failed to send message.".into());
                self.notify_background_failure("local", source_id, error);
            } else {
                if let Some(thread) = self.threads.iter_mut().find(|t| t.id == source_id) {
                    thread.background_start_pending =
                        thread.pending_prompt.is_some() || thread.queued_prompt.is_some();
                }
                self.notify_background_started("local", source_id);
            }
            self.checkpoint();
        } else {
            let failed = self
                .threads
                .get(source_index)
                .is_some_and(|t| t.id == source_id && t.status == Status::Failed);
            if !failed {
                self.create_thread_in(project, &[source_id], &seed);
                if !self.headless {
                    self.restore_composer_focus(crate::ui::composer_ui::FocusReason::ThreadOpened);
                }
            }
            self.checkpoint();
        }
    }

    pub(crate) fn fresh_session_available(&self, thread: &Thread) -> bool {
        !self.demo
            && !thread.messages.is_empty()
            && crate::harness::supports_fresh_context(&thread.harness)
            && self.client.as_ref().is_none_or(|client| {
                client.supports_capability(&thread.machine, crate::protocol::FRESH_CONTEXT)
            })
    }

    pub(crate) fn fresh_session_command_available(&self, thread: &Thread) -> bool {
        self.fresh_session_available(thread)
            && thread.request.is_none()
            && thread.native.questionnaire.is_none()
    }

    pub fn usage_limits_command_available(&self, thread: &Thread) -> bool {
        !self.demo
            && crate::usage_limits::has_usage_limits(&thread.harness)
            && self.client.as_ref().is_none_or(|client| {
                client.supports_capability(&thread.machine, crate::protocol::USAGE_LIMITS)
            })
    }

    pub fn slash_matches(&self) -> Vec<pi_session::SlashCommand> {
        self.current().slash_matches_with_capabilities(
            self.fresh_session_command_available(self.current()),
            self.usage_limits_command_available(self.current()),
        )
    }

    pub fn effective_default_thread_env_mode(&self) -> ThreadEnvMode {
        if self.client.is_some() {
            self.machines
                .views
                .get(&self.machines.selected)
                .and_then(|v| {
                    serde_json::from_value::<ThreadEnvMode>(
                        v["index"]["threadDefaults"]["default_thread_env_mode"].clone(),
                    )
                    .ok()
                })
                .unwrap_or(self.thread_defaults.default_thread_env_mode)
        } else {
            self.thread_defaults.default_thread_env_mode
        }
    }

    pub fn effective_env_mode(&self, thread: &Thread) -> ThreadEnvMode {
        if thread.worktree_path.is_some() {
            return ThreadEnvMode::Worktree;
        }
        if !thread.messages.is_empty() {
            return ThreadEnvMode::Local;
        }
        thread
            .env_mode
            .unwrap_or_else(|| self.effective_default_thread_env_mode())
    }

    pub fn supports_vcs_worktrees_on(&self, machine: &str) -> bool {
        self.client.as_ref().is_none_or(|client| {
            client.supports_capability(machine, crate::protocol::VCS_WORKTREES)
        })
    }

    pub fn thread_vcs_repo_key(&self, thread: &Thread) -> Option<(String, String)> {
        if thread.captain_home && !thread.delegated {
            return None;
        }
        let project = self.project(thread.project)?;
        let machine = if self.client.is_some() {
            self.machines.selected.clone()
        } else {
            thread.machine.clone()
        };
        Some((machine, project.path.to_string_lossy().into_owned()))
    }

    /// Whether the current thread hides its composer behind a read-only Captain bar
    /// (an archived Captain home or an earlier Captain conversation).
    pub fn composer_hidden(&self) -> bool {
        let t = self.current();
        let archived_home = t.captain_home
            && t.captain
                .is_some_and(|captain| self.captain_phase(captain) == CaptainPhase::Archived);
        let earlier_home = self.is_captain_home(t)
            && t.captain
                .is_some_and(|captain| self.captain_home(captain) != Some(t.id));
        archived_home || earlier_home
    }

    pub fn composer_vcs_controls(&self) -> ComposerVcsControls {
        if self.demo {
            return ComposerVcsControls::Hidden;
        }
        let thread = self.current();
        let Some((machine, project_path)) = self.thread_vcs_repo_key(thread) else {
            return ComposerVcsControls::Hidden;
        };
        if !self.supports_vcs_worktrees_on(&machine) {
            return ComposerVcsControls::Hidden;
        }
        if let Some(worktree_path) = &thread.worktree_path {
            return ComposerVcsControls::LockedWorktree {
                branch: thread.branch.clone(),
                worktree_path: worktree_path.clone(),
            };
        }
        if !thread.messages.is_empty() {
            return ComposerVcsControls::Hidden;
        }
        let is_repo = self
            .vcs_branches
            .get(&(machine, project_path))
            .is_some_and(|entry| entry.is_repo);
        if !is_repo {
            return ComposerVcsControls::Hidden;
        }
        let mode = if !thread.draft_model_state.additional_models.is_empty() {
            ThreadEnvMode::Worktree
        } else {
            self.effective_env_mode(thread)
        };
        ComposerVcsControls::Editable {
            mode,
            base_branch: thread.base_branch.clone(),
            start_from_origin: thread.effective_start_from_origin(),
        }
    }

    pub fn ensure_vcs_branches(&mut self, force_refresh: bool) {
        if self.demo {
            return;
        }
        if !force_refresh && !self.runtime_enabled && self.client.is_none() {
            return;
        }
        let Some((machine, project_path)) = self.thread_vcs_repo_key(self.current()) else {
            return;
        };
        if !self.supports_vcs_worktrees_on(&machine) {
            return;
        }
        let key = (machine.clone(), project_path.clone());
        if let Some(entry) = self.vcs_branches.get(&key)
            && (entry.loading || (entry.loaded && !force_refresh))
        {
            return;
        }
        if let Some(client) = &self.client {
            if !self.machine_connected(&machine) {
                return;
            }
            let target = if machine == "local" {
                Some(client)
            } else {
                client.linked.get(&machine)
            };
            let Some(target) = target else {
                return;
            };
            self.vcs_branches.entry(key.clone()).or_default().loading = true;
            self.next_vcs_request_id += 1;
            let req_id = self.next_vcs_request_id;
            self.pending_vcs_requests.insert(req_id, key);
            let input = crate::vcs::VcsListBranchesInput {
                cwd: project_path,
                query: None,
                cursor: None,
                include_matching_remote_refs: None,
                ref_kind: None,
                refresh: force_refresh.then_some(true),
                limit: Some(crate::vcs::GIT_LIST_BRANCHES_MAX_LIMIT),
            };
            if let Ok(params) = serde_json::to_value(&input) {
                target.call_rpc(format!("vcs-branches:{req_id}"), "vcs.listBranches", params);
            }
            return;
        }
        self.vcs_branches.entry(key).or_default().loading = true;
        let (tx, rx) = std::sync::mpsc::channel();
        self.vcs_branch_rxs.push(rx);
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let input = crate::vcs::VcsListBranchesInput {
                cwd: project_path.clone(),
                query: None,
                cursor: None,
                include_matching_remote_refs: None,
                ref_kind: None,
                refresh: force_refresh.then_some(true),
                limit: Some(crate::vcs::GIT_LIST_BRANCHES_MAX_LIMIT),
            };
            let result = crate::vcs::list_branches(&input);
            let _ = tx.send(VcsBranchResultMsg {
                machine,
                project_path,
                result,
            });
            wake();
        });
    }

    pub fn poll_vcs_branches(&mut self) -> bool {
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.vcs_branch_rxs.len());
        let rxs = std::mem::take(&mut self.vcs_branch_rxs);
        for rx in rxs {
            match rx.try_recv() {
                Ok(msg) => {
                    changed = true;
                    self.apply_vcs_branches_result(msg.machine, msg.project_path, msg.result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => changed = true,
            }
        }
        self.vcs_branch_rxs = remaining;
        changed
    }

    pub(crate) fn apply_vcs_branches_result(
        &mut self,
        machine: String,
        project_path: String,
        result: Result<crate::vcs::VcsListBranchesResult, String>,
    ) {
        let key = (machine.clone(), project_path.clone());
        let entry = self.vcs_branches.entry(key.clone()).or_default();
        entry.loading = false;
        entry.loaded = true;
        match result {
            Ok(res) => {
                entry.is_repo = res.is_repo;
                entry.error = None;
                entry.refs = res.refs;
            }
            Err(err) => {
                entry.error = Some(err);
            }
        }
        let default_branch = if entry.is_repo {
            crate::branch_toolbar::default_worktree_base_branch(&entry.refs, None)
        } else {
            None
        };
        let controls = self.composer_vcs_controls();
        if !matches!(
            controls,
            ComposerVcsControls::Editable {
                mode: ThreadEnvMode::Worktree,
                ..
            }
        ) {
            self.base_branch_picker = false;
        }
        if !matches!(controls, ComposerVcsControls::Editable { .. }) {
            self.env_mode_picker = false;
        }
        if let Some(default_branch) = default_branch {
            let mut updated_active = false;
            let default_mode = self.effective_default_thread_env_mode();
            for index in 0..self.threads.len() {
                let matches = self.thread_vcs_repo_key(&self.threads[index]).as_ref() == Some(&key);
                if matches {
                    let t = &self.threads[index];
                    let mode = if t.worktree_path.is_some() {
                        ThreadEnvMode::Worktree
                    } else if !t.messages.is_empty() {
                        ThreadEnvMode::Local
                    } else {
                        t.env_mode.unwrap_or(default_mode)
                    };
                    if t.messages.is_empty()
                        && t.worktree_path.is_none()
                        && mode == ThreadEnvMode::Worktree
                        && t.base_branch.is_none()
                    {
                        self.threads[index].base_branch = Some(default_branch.clone());
                        if index == self.active {
                            updated_active = true;
                        }
                    }
                }
            }
            if updated_active {
                self.checkpoint();
            }
        }
    }

    pub(crate) fn handle_vcs_branches_rpc(&mut self, req_id: u64, result: Result<Value, String>) {
        let Some((machine, project_path)) = self.pending_vcs_requests.remove(&req_id) else {
            return;
        };
        let parsed = result.and_then(|val| {
            serde_json::from_value::<crate::vcs::VcsListBranchesResult>(val)
                .map_err(|e| format!("Invalid vcs.listBranches response: {e}"))
        });
        self.apply_vcs_branches_result(machine, project_path, parsed);
    }

    pub fn set_draft_env_mode(&mut self, mode: ThreadEnvMode) {
        let t = self.current();
        if !t.messages.is_empty() || t.worktree_path.is_some() {
            return;
        }
        let default_branch = self.thread_vcs_repo_key(t).and_then(|key| {
            self.vcs_branches
                .get(&key)
                .filter(|entry| entry.is_repo)
                .and_then(|entry| {
                    crate::branch_toolbar::default_worktree_base_branch(&entry.refs, None)
                })
        });
        let thread = self.current_mut();
        thread.env_mode = Some(mode);
        if mode == ThreadEnvMode::Local {
            if thread.submission_error.as_deref()
                == Some("Select a base branch before sending in New worktree mode.")
            {
                thread.submission_error = None;
            }
        } else {
            if thread.base_branch.is_none() {
                thread.base_branch = default_branch;
            }
            // Do not keep an idle prewarmed session in the main checkout once switched to worktree.
            if thread.pi.is_some() && thread.session.is_none() && thread.messages.is_empty() {
                thread.disconnect();
                thread.prewarm_attempted = false;
            }
        }
        self.env_mode_picker = false;
        if mode == ThreadEnvMode::Worktree {
            self.ensure_vcs_branches(false);
        }
        self.checkpoint();
    }

    pub fn set_draft_base_branch(&mut self, branch: String) {
        let thread = self.current_mut();
        if !thread.messages.is_empty() || thread.worktree_path.is_some() {
            return;
        }
        thread.base_branch = Some(branch);
        if thread.submission_error.as_deref()
            == Some("Select a base branch before sending in New worktree mode.")
        {
            thread.submission_error = None;
        }
        self.base_branch_picker = false;
        self.checkpoint();
    }

    pub fn set_draft_start_from_origin(&mut self, start_from_origin: bool) {
        let thread = self.current_mut();
        if !thread.messages.is_empty() || thread.worktree_path.is_some() {
            return;
        }
        thread.start_from_origin = Some(start_from_origin);
        self.checkpoint();
    }

    pub(crate) fn missing_worktree_base_branch_error(&self, thread: &Thread) -> Option<String> {
        let should_create = !thread.worktree_bootstrapping()
            && !thread.native.queue_held
            && thread.messages.is_empty()
            && thread.worktree_path.is_none()
            && thread.project.is_some()
            && !self.is_captain_home(thread)
            && self.effective_env_mode(thread) == ThreadEnvMode::Worktree;
        if should_create
            && thread
                .base_branch
                .as_deref()
                .is_none_or(|b| b.trim().is_empty())
        {
            Some("Select a base branch before sending in New worktree mode.".into())
        } else {
            None
        }
    }

    pub(super) fn build_current_prompt(&mut self) -> Option<Prompt> {
        if self.current().harness == "pi"
            && !self.current().model_provider.is_empty()
            && self.current().model_provider != "cpa"
        {
            self.current_mut().error = Some(
                "Choose a CPA model before sending. Shina will not send prompts to a direct provider."
                    .into(),
            );
            return None;
        }
        if self.current().draft_images.iter().any(Attachment::is_image)
            && self.current().supports_images == Some(false)
        {
            let err = self.current().image_model_error();
            self.current_mut().attachment_error = Some(err);
            return None;
        }
        let local_server_id = self.machine_identity(&self.machines.selected);
        let peers = self.peers.clone();
        let labels: Vec<(String, String)> = peers
            .iter()
            .map(|p| (p.server_id.clone(), p.label.clone()))
            .collect();
        let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
            self.current().draft_reply_to.as_ref(),
            Some(&self.current().machine),
            Some(&local_server_id),
            &peers,
            &|m| {
                if m == "local" {
                    "Local".into()
                } else {
                    labels
                        .iter()
                        .find(|(id, _)| id == m)
                        .map(|(_, l)| l.clone())
                        .unwrap_or_else(|| m.to_owned())
                }
            },
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.current_mut().attachment_error = Some(error);
                return None;
            }
        };
        if !self.headless
            && let Some(error) = self.current().validate_prompt_length(&prompt)
        {
            self.current_mut().submission_error = Some(error);
            return None;
        }
        self.current_mut().submission_error = None;
        Some(prompt)
    }

    pub(super) fn try_worktree_bootstrap_send(
        &mut self,
        fresh_context: bool,
        fresh_arm: Option<u64>,
    ) -> Option<bool> {
        let queue_behind_bootstrap = self.current().worktree_bootstrapping()
            || (self.current().native.queue_held
                && self.current().worktree_path.is_some()
                && !self.current().status.busy());
        if queue_behind_bootstrap {
            let prompt = match self.build_current_prompt() {
                Some(p) => p,
                None => return Some(false),
            };
            let thread = self.current_mut();
            thread.prompt_history.reset();
            thread.typing_guard.on_blur_or_send();
            thread.paste_history = Default::default();
            thread.draft.replace_text("");
            thread.draft_images.clear();
            thread.draft_context.clear();
            thread.draft_reply_to = None;
            thread.retained_context = Default::default();
            thread.native.queue_held = true;
            thread.serial += 1;
            let queued_id = thread.serial;
            if fresh_context {
                thread.native.fresh_queue_ids.insert(queued_id);
            }
            thread.native.queue.push_back(pi_session::QueuedMessage {
                id: queued_id,
                prompt,
            });
            thread.life.user_sent(lifecycle::now());
            let id = thread.id;
            let server = self.machines.selected.clone();
            self.set_keep_full_history(&server, id, false);
            if let Some(arm) = fresh_arm {
                self.disarm_fresh_session(id, Some(arm));
            }
            self.checkpoint();
            return Some(true);
        }

        let is_first_worktree = self.current().messages.is_empty()
            && self.current().worktree_path.is_none()
            && self.current().project.is_some()
            && !self.is_captain_home(self.current())
            && self.effective_env_mode(self.current()) == ThreadEnvMode::Worktree;
        if !is_first_worktree {
            return None;
        }

        if let Some(error) = self.missing_worktree_base_branch_error(self.current()) {
            self.current_mut().submission_error = Some(error);
            return Some(false);
        }
        let base_branch = self
            .current()
            .base_branch
            .clone()
            .map(|b| b.trim().to_owned())
            .filter(|b| !b.is_empty())?;
        let prompt = match self.build_current_prompt() {
            Some(p) => p,
            None => return Some(false),
        };
        let project_cwd = self
            .project(self.current().project)
            .map(|p| p.path.clone())?;
        let snapshot = crate::composer_rich::Snapshot::capture_with_context(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
        );
        let thread = self.current_mut();
        let reply_to = thread.draft_reply_to.take();
        let start_from_origin = thread.effective_start_from_origin();
        thread.prompt_history.reset();
        thread.typing_guard.on_blur_or_send();
        thread.paste_history = Default::default();
        thread.draft.replace_text("");
        thread.draft_images.clear();
        thread.draft_context.clear();
        thread.retained_context = Default::default();
        thread.error = None;
        thread.attachment_error = None;
        if thread.pi.is_some() && thread.session.is_none() {
            thread.disconnect();
            thread.prewarm_attempted = false;
        }
        thread.native.queue_held = true;
        thread.serial += 1;
        let queued_id = thread.serial;
        if fresh_context {
            thread.native.fresh_queue_ids.insert(queued_id);
        }
        thread.native.queue.push_back(pi_session::QueuedMessage {
            id: queued_id,
            prompt,
        });
        thread.worktree_bootstrap = Some(WorktreeBootstrapState {
            snapshot,
            reply_to,
            queued_id,
            fresh_context,
        });
        let thread_id = thread.id;
        let server = self.machines.selected.clone();
        self.set_keep_full_history(&server, thread_id, false);
        self.invalidate_undo(thread_id);
        if let Some(arm) = fresh_arm {
            self.disarm_fresh_session(thread_id, Some(arm));
        }

        let worktrees_dir = self.data_dir.join("worktrees");
        let wake = self.wake.clone();
        #[cfg(test)]
        let git_ctx = self.test_git_context.clone().unwrap_or_default();
        #[cfg(not(test))]
        let git_ctx = crate::vcs::GitContext::default();
        let (tx, rx) = std::sync::mpsc::channel();
        self.worktree_bootstrap_rxs.push(rx);
        std::thread::spawn(move || {
            let cwd_str = project_cwd.to_string_lossy().into_owned();
            let raw_branch = crate::vcs::build_temporary_worktree_branch_name(
                &uuid::Uuid::new_v4().simple().to_string(),
            );
            let new_branch =
                crate::vcs::resolve_available_branch_name_with(&project_cwd, &raw_branch, &git_ctx)
                    .unwrap_or(raw_branch);
            let result = crate::vcs::create_worktree_with(
                &crate::vcs::VcsCreateWorktreeInput {
                    cwd: cwd_str,
                    ref_name: base_branch.clone(),
                    new_ref_name: Some(new_branch),
                    base_ref_name: Some(base_branch.clone()),
                    path: None,
                    start_from_origin: Some(start_from_origin),
                },
                &worktrees_dir,
                &git_ctx,
            );
            let _ = tx.send(WorktreeBootstrapResult {
                thread_id,
                base_branch,
                result,
            });
            wake();
        });
        self.checkpoint();
        Some(true)
    }

    pub fn poll_worktree_bootstraps(&mut self) -> bool {
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.worktree_bootstrap_rxs.len());
        let rxs = std::mem::take(&mut self.worktree_bootstrap_rxs);
        for rx in rxs {
            match rx.try_recv() {
                Ok(msg) => {
                    changed = true;
                    let Some(index) = self.threads.iter().position(|t| t.id == msg.thread_id)
                    else {
                        continue;
                    };
                    let Some(boot) = self.threads[index].worktree_bootstrap.take() else {
                        continue;
                    };
                    let in_multi_batch = self.is_multi_model_target_thread(msg.thread_id);
                    match msg.result {
                        Ok(created) => {
                            let project_cwd = self
                                .project(self.threads[index].project)
                                .map(|p| p.path.clone());
                            let wt_path = PathBuf::from(&created.worktree.path);
                            let script_check = project_cwd
                                .as_deref()
                                .map(|cwd| crate::vcs::find_setup_script_info(&wt_path, cwd));
                            let thread = &mut self.threads[index];
                            thread.worktree_path = Some(created.worktree.path.clone());
                            thread.branch = Some(created.worktree.ref_name.clone());
                            thread.base_branch = Some(msg.base_branch);
                            thread.env_mode = Some(ThreadEnvMode::Worktree);
                            thread.submission_error = None;
                            thread.worktree_bootstrap_fresh_context = boot.fresh_context;
                            thread.life.user_sent(lifecycle::now());
                            if boot.fresh_context {
                                self.persist_transcripts = true;
                            }
                            match script_check {
                                Some(Ok(Some((script_name, script_command)))) => {
                                    thread.worktree_setup = Some(WorktreeSetupState {
                                        phase: WorktreeSetupPhase::Running,
                                        script_name: Some(script_name),
                                        script_command: Some(script_command),
                                        error: None,
                                        output: String::new(),
                                        exit_code: None,
                                        timed_out: false,
                                        duration_ms: 0,
                                        skipped: false,
                                    });
                                    thread.worktree_setup_output_open = false;
                                    thread.native.queue_held = true;
                                    let tid = thread.id;
                                    self.spawn_worktree_setup_runner(tid);
                                }
                                Some(Err(error)) => {
                                    thread.worktree_setup = Some(WorktreeSetupState {
                                        phase: WorktreeSetupPhase::Failed,
                                        script_name: None,
                                        script_command: None,
                                        error: Some(error),
                                        output: String::new(),
                                        exit_code: None,
                                        timed_out: false,
                                        duration_ms: 0,
                                        skipped: false,
                                    });
                                    thread.worktree_setup_output_open = true;
                                    thread.native.queue_held = true;
                                }
                                _ => {
                                    thread.native.queue_held = false;
                                    self.release_after_worktree_setup(index);
                                }
                            }
                            if in_multi_batch
                                && let Some(idx) =
                                    self.threads.iter().position(|t| t.id == msg.thread_id)
                            {
                                if self.threads[idx].status == Status::Failed {
                                    let error = self.threads[idx]
                                        .error
                                        .clone()
                                        .unwrap_or_else(|| "Failed to start thread.".into());
                                    self.remove_failed_multi_model_thread(msg.thread_id);
                                    self.record_multi_model_target_outcome(
                                        msg.thread_id,
                                        Err(error),
                                    );
                                } else {
                                    self.threads[idx].background_start_pending =
                                        self.runtime_enabled || self.threads[idx].pi.is_some();
                                    self.record_multi_model_target_outcome(msg.thread_id, Ok(()));
                                }
                            }
                            self.checkpoint();
                        }
                        Err(error) => {
                            if in_multi_batch {
                                self.remove_failed_multi_model_thread(msg.thread_id);
                                self.record_multi_model_target_outcome(msg.thread_id, Err(error));
                            } else {
                                let thread = &mut self.threads[index];
                                thread.native.queue.retain(|q| q.id != boot.queued_id);
                                thread.native.fresh_queue_ids.remove(&boot.queued_id);
                                thread.native.queue_held = false;
                                thread.worktree_path = None;
                                thread.branch = None;
                                thread.env_mode = Some(ThreadEnvMode::Worktree);
                                thread.restore_snapshot(&boot.snapshot);
                                if thread.draft_reply_to.is_none() {
                                    thread.draft_reply_to = boot.reply_to;
                                }
                                thread.submission_error = Some(error);
                            }
                            self.checkpoint();
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => changed = true,
            }
        }
        self.worktree_bootstrap_rxs = remaining;
        self.resolve_completed_multi_model_batches();
        changed
    }

    pub(super) fn is_multi_model_target_thread(&self, thread_id: usize) -> bool {
        self.multi_model_batches.iter().any(|batch| {
            batch
                .targets
                .iter()
                .any(|target| target.thread_id == Some(thread_id))
        })
    }

    pub(super) fn remove_failed_multi_model_thread(&mut self, thread_id: usize) {
        let active_id = self.threads.get(self.active).map(|t| t.id);
        if let Some(t) = self.threads.iter_mut().find(|t| t.id == thread_id) {
            let _ = t.stop_task();
            t.disconnect();
        }
        self.threads.retain(|t| t.id != thread_id);
        self.tabs.retain(|id| *id != thread_id);
        self.drop_ghost(thread_id);
        if let Some(aid) = active_id
            && let Some(new_idx) = self.threads.iter().position(|t| t.id == aid)
        {
            self.active = new_idx;
        } else if self.active >= self.threads.len() && !self.threads.is_empty() {
            self.active = self.threads.len() - 1;
        }
    }

    pub(super) fn record_multi_model_target_outcome(
        &mut self,
        thread_id: usize,
        result: Result<(), String>,
    ) {
        for batch in &mut self.multi_model_batches {
            if let Some(target) = batch
                .targets
                .iter_mut()
                .find(|t| t.thread_id == Some(thread_id))
            {
                target.outcome = match result {
                    Ok(()) => MultiModelTargetOutcome::Started,
                    Err(err) => MultiModelTargetOutcome::Failed(err),
                };
                return;
            }
        }
    }

    pub(crate) fn handle_multi_model_async_failure(
        &mut self,
        thread_id: usize,
        error: &str,
    ) -> bool {
        let in_open_batch = self
            .multi_model_batches
            .iter()
            .any(|batch| batch.targets.iter().any(|t| t.thread_id == Some(thread_id)));
        if !in_open_batch {
            return false;
        }
        self.remove_failed_multi_model_thread(thread_id);
        self.record_multi_model_target_outcome(thread_id, Err(error.to_owned()));
        self.resolve_completed_multi_model_batches();
        true
    }

    pub(super) fn resolve_completed_multi_model_batches(&mut self) {
        let mut remaining = Vec::with_capacity(self.multi_model_batches.len());
        let mut completed = Vec::new();
        for batch in self.multi_model_batches.drain(..) {
            if batch
                .targets
                .iter()
                .any(|t| matches!(t.outcome, MultiModelTargetOutcome::Pending))
            {
                remaining.push(batch);
            } else {
                completed.push(batch);
            }
        }
        self.multi_model_batches = remaining;
        for batch in completed {
            self.finalize_multi_model_batch(batch);
        }
    }

    pub(super) fn finalize_multi_model_batch(&mut self, batch: MultiModelBatch) {
        let started_count = batch
            .targets
            .iter()
            .filter(|t| matches!(t.outcome, MultiModelTargetOutcome::Started))
            .count();
        let failed_entries: Vec<&MultiModelTargetEntry> = batch
            .targets
            .iter()
            .filter(|t| matches!(t.outcome, MultiModelTargetOutcome::Failed(_)))
            .collect();

        if failed_entries.is_empty() {
            self.push_toast(
                ToastKind::Success,
                format!(
                    "Started {started_count} {} in background",
                    if started_count == 1 {
                        "thread"
                    } else {
                        "threads"
                    }
                ),
                None,
                None,
                Some(5_000),
            );
            self.checkpoint();
            return;
        }

        if let Some(source_idx) = self
            .threads
            .iter()
            .position(|t| t.id == batch.source_thread_id)
            && !self.threads[source_idx].has_draft_content()
        {
            self.threads[source_idx].restore_snapshot(&batch.snapshot);
            if self.threads[source_idx].draft_reply_to.is_none() {
                self.threads[source_idx].draft_reply_to = batch.reply_to.clone();
            }
            if started_count == 0 {
                self.threads[source_idx].draft_model_state = batch.saved_draft_model_state.clone();
            } else {
                let first_failed = &failed_entries[0].selection;
                let remaining_failed: Vec<crate::model_selection::ModelSelection> = failed_entries
                    .iter()
                    .skip(1)
                    .map(|e| e.selection.clone())
                    .collect();
                let current_active = self.active;
                self.active = source_idx;
                if self.threads[source_idx].harness != first_failed.instance_id
                    && self.validate_harness(&first_failed.instance_id).is_ok()
                {
                    self.choose_harness(&first_failed.instance_id);
                }
                let provider = if first_failed.instance_id == "pi" {
                    "cpa".to_owned()
                } else {
                    first_failed.instance_id.clone()
                };
                self.threads[source_idx].set_model(provider, first_failed.model.clone());
                for opt in &first_failed.options {
                    self.threads[source_idx].set_option(opt.id.clone(), opt.value.clone());
                }
                self.threads[source_idx].draft_model_state =
                    crate::model_selection::DraftModelState {
                        active_provider: Some(first_failed.instance_id.clone()),
                        model_selection_by_provider: std::collections::BTreeMap::from([(
                            first_failed.instance_id.clone(),
                            first_failed.clone(),
                        )]),
                        model_selection_explicit: true,
                        additional_models: remaining_failed,
                    };
                self.threads[source_idx].env_mode = Some(ThreadEnvMode::Worktree);
                if self.threads[source_idx].base_branch.is_none() {
                    self.threads[source_idx].base_branch = Some(batch.base_branch.clone());
                }
                self.active = current_active;
            }
        }

        let failed_names: Vec<String> = failed_entries
            .iter()
            .map(|e| e.display_name.clone())
            .collect();
        let mut errors = Vec::new();
        for entry in &failed_entries {
            if let MultiModelTargetOutcome::Failed(err) = &entry.outcome
                && !errors.contains(err)
            {
                errors.push(err.clone());
            }
        }
        self.push_toast(
            ToastKind::Error,
            format!("Could not start {}", failed_names.join(", ")),
            (!errors.is_empty()).then(|| errors.join(" ")),
            None,
            None,
        );
        self.checkpoint();
    }

    pub(super) fn model_display_name_for_selection(
        &self,
        selection: &crate::model_selection::ModelSelection,
    ) -> String {
        if selection.instance_id == "pi" {
            if let Some(m) = self.catalog.models.iter().find(|m| m.id == selection.model) {
                return m.name.clone();
            }
        }
        self.picker_models_by_instance()
            .get(&selection.instance_id)
            .and_then(|opts| opts.iter().find(|o| o.slug == selection.model))
            .map(|o| o.name.clone())
            .unwrap_or_else(|| selection.model.clone())
    }

    pub(super) fn validate_local_multi_model_target(
        &self,
        selection: &crate::model_selection::ModelSelection,
        has_images: bool,
        entries: &[crate::model_picker::ProviderInstanceEntry],
        picker_models: &std::collections::BTreeMap<
            String,
            Vec<crate::model_picker::PickerModelOption>,
        >,
    ) -> Result<Vec<AgentModel>, String> {
        self.validate_harness(&selection.instance_id)?;
        if let Some(entry) = entries
            .iter()
            .find(|e| e.instance_id == selection.instance_id)
            && !crate::model_picker::is_provider_instance_picker_ready(entry)
        {
            return Err(crate::model_picker::get_provider_status_message(entry));
        }

        if selection.instance_id == "pi" {
            if let Some(model) = self.catalog.models.iter().find(|m| m.id == selection.model) {
                if !model.provider.is_empty() && model.provider != "cpa" {
                    return Err(
                        "Choose a CPA model before sending. Shina will not send prompts to a direct provider."
                            .into(),
                    );
                }
                if has_images && model.supports_images == Some(false) {
                    return Err(
                        "This model does not support image attachments. Remove the images or switch to a vision model."
                            .into(),
                    );
                }
            } else if !self.catalog.models.is_empty() && !selection.model.starts_with("cpa/") {
                return Err(
                    "Choose a CPA model before sending. Shina will not send prompts to a direct provider."
                        .into(),
                );
            }
            return Ok(self.catalog.models.clone());
        }

        if let Some(opt) = picker_models
            .get(&selection.instance_id)
            .and_then(|opts| opts.iter().find(|o| o.slug == selection.model))
            && opt.is_unavailable
        {
            return Err(format!("{} is unavailable.", opt.name));
        }

        // These rows seed the new threads, so they come from the sending draft's
        // workspace and never from another folder's catalog.
        let cwd = self.catalog_workspace_cwd(self.active);
        let models = self.harness_models_for_cwd(&selection.instance_id, cwd.as_deref());
        if has_images
            && models
                .iter()
                .find(|m| m.id == selection.model)
                .is_some_and(|m| m.supports_images == Some(false))
        {
            return Err(
                "This model does not support image attachments. Remove the images or switch to a vision model."
                    .into(),
            );
        }
        Ok(models)
    }

    pub(super) fn advance_remote_multi_model_batch(&mut self, batch_idx: usize) {
        let Some(batch) = self.multi_model_batches.get_mut(batch_idx) else {
            return;
        };
        let server = batch.server.clone();
        while batch.next_remote_index < batch.targets.len() {
            let target_idx = batch.next_remote_index;
            batch.next_remote_index += 1;
            let selection = batch.targets[target_idx].selection.clone();
            let provider = if selection.instance_id == "pi" {
                "cpa".to_owned()
            } else {
                selection.instance_id.clone()
            };
            let thinking = selection
                .options
                .iter()
                .find(|o| {
                    matches!(
                        o.id.as_str(),
                        "thinking" | "effort" | "reasoningEffort" | "reasoning" | "thought_level"
                    )
                })
                .and_then(|o| o.value.as_str())
                .map(str::to_owned);
            let service_tier = selection
                .options
                .iter()
                .find(|o| o.id == "serviceTier")
                .and_then(|o| o.value.as_str())
                .map(str::to_owned);
            let mut occupied: Vec<usize> = self
                .threads
                .iter()
                .filter(|t| {
                    !t.client_draft
                        && (t.has_draft_content()
                            || !t.messages.is_empty()
                            || self.tabs.contains(&t.id))
                })
                .map(|t| t.id)
                .collect();
            for &id in &batch.occupied_remote_ids {
                if !occupied.contains(&id) {
                    occupied.push(id);
                }
            }
            let params = json!({
                "project": batch.project,
                "occupiedDrafts": occupied,
                "harness": selection.instance_id,
                "selectedModel": (provider, selection.model),
                "thinking": thinking,
                "access": batch.access.id(),
                "serviceTier": service_tier,
            });
            let result = self
                .client
                .as_ref()
                .and_then(|client| client.commands_for(&server))
                .ok_or_else(|| "This machine is not linked. Your draft was kept.".to_owned())
                .and_then(|commands| commands.send("thread.create", params));
            match result {
                Ok(cmd_id) => {
                    batch.targets[target_idx].create_cmd_id = Some(cmd_id);
                    break;
                }
                Err(error) => {
                    batch.targets[target_idx].outcome = MultiModelTargetOutcome::Failed(error);
                }
            }
        }
    }

    pub(crate) fn handle_multi_model_remote_receipt(
        &mut self,
        server: &str,
        command: &crate::protocol::Command,
        value: &Value,
    ) -> bool {
        let accepted = value["result"]["status"] == "accepted";
        let error_msg = value["error"]["message"]
            .as_str()
            .or(value["result"]["result"]["error"].as_str())
            .unwrap_or("Failed to send message.")
            .to_owned();

        if command.method == "thread.create" {
            let found = self
                .multi_model_batches
                .iter()
                .enumerate()
                .find_map(|(bi, b)| {
                    (b.server == server).then(|| {
                        b.targets
                            .iter()
                            .position(|t| t.create_cmd_id == Some(command.command_id))
                            .map(|ti| (bi, ti))
                    })?
                });
            let Some((batch_idx, target_idx)) = found else {
                return false;
            };
            if !accepted {
                self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                    MultiModelTargetOutcome::Failed(error_msg);
            } else if let Some(tid) = value["result"]["result"]["threadId"].as_u64() {
                let tid = tid as usize;
                self.multi_model_batches[batch_idx]
                    .occupied_remote_ids
                    .push(tid);
                self.multi_model_batches[batch_idx].targets[target_idx].thread_id = Some(tid);
                let batch = &self.multi_model_batches[batch_idx];
                let selection = &batch.targets[target_idx].selection;
                let commands = self
                    .client
                    .as_ref()
                    .and_then(|client| client.commands_for(server))
                    .cloned();
                let supports_options = self
                    .client
                    .as_ref()
                    .is_some_and(|c| c.supports_capability(server, crate::protocol::MODEL_OPTIONS));
                if supports_options
                    && !selection.options.is_empty()
                    && let Some(ref cmds) = commands
                {
                    let _ = cmds.send(
                        "thread.configure",
                        json!({"threadId": tid, "options": selection.options}),
                    );
                }
                let params = json!({
                    "threadId": tid,
                    "text": batch.prompt.text,
                    "attachments": batch.prompt.images.as_ref(),
                    "context": batch.prompt.context,
                    "draft": batch.prompt.draft,
                    "editorRevision": uuid::Uuid::new_v4(),
                    "mode": "prompt",
                    "envMode": ThreadEnvMode::Worktree,
                    "baseBranch": batch.base_branch,
                    "startFromOrigin": batch.start_from_origin,
                });
                let send_res = commands
                    .ok_or_else(|| "This machine is not linked. Your draft was kept.".to_owned())
                    .and_then(|cmds| cmds.send("thread.send", params));
                match send_res {
                    Ok(cmd_id) => {
                        self.multi_model_batches[batch_idx].targets[target_idx].send_cmd_id =
                            Some(cmd_id);
                    }
                    Err(err) => {
                        self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                            MultiModelTargetOutcome::Failed(err);
                        if let Some(cmds) =
                            self.client.as_ref().and_then(|c| c.commands_for(server))
                        {
                            let _ = cmds
                                .send("thread.delete", json!({"threadId": tid, "blankOnly": true}));
                        }
                    }
                }
            } else {
                self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                    MultiModelTargetOutcome::Failed(
                        "Thread creation was not confirmed. Your draft was kept.".into(),
                    );
            }
            self.advance_remote_multi_model_batch(batch_idx);
            self.resolve_completed_multi_model_batches();
            return true;
        }

        if command.method == "thread.send" {
            let found = self
                .multi_model_batches
                .iter()
                .enumerate()
                .find_map(|(bi, b)| {
                    (b.server == server).then(|| {
                        b.targets
                            .iter()
                            .position(|t| t.send_cmd_id == Some(command.command_id))
                            .map(|ti| (bi, ti))
                    })?
                });
            let Some((batch_idx, target_idx)) = found else {
                return false;
            };
            if accepted {
                self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                    MultiModelTargetOutcome::Started;
            } else {
                let tid = self.multi_model_batches[batch_idx].targets[target_idx].thread_id;
                self.multi_model_batches[batch_idx].targets[target_idx].outcome =
                    MultiModelTargetOutcome::Failed(error_msg);
                if let Some(tid) = tid
                    && let Some(cmds) = self.client.as_ref().and_then(|c| c.commands_for(server))
                {
                    let _ = cmds.send("thread.delete", json!({"threadId": tid, "blankOnly": true}));
                }
            }
            self.resolve_completed_multi_model_batches();
            return true;
        }

        false
    }

    /// Ported from KSG Code's `components/chat/useMultipleModelSend.ts`:
    /// starts one background thread per selected model, each in its own worktree.
    pub(super) fn dispatch_multi_model_send(&mut self) -> bool {
        if self.current().draft.text.trim() == "/model" {
            self.choose_slash("model".into());
            return false;
        }
        if self.current().configuring
            || self.current().native.operating()
            || self.current().phase == "Stopping"
            || !self.current().has_draft_content()
        {
            return false;
        }
        if self.client.is_some() {
            if !self.machine_connected(&self.machines.selected) {
                self.push_toast(
                    ToastKind::Warning,
                    "Not connected: message not sent",
                    Some("Reconnecting to the environment. Try again once it is connected.".into()),
                    None,
                    Some(5_000),
                );
                return false;
            }
        } else if self.network_blocked() {
            self.current_mut().error =
                Some("Waiting for the network after resume. Your draft has not been sent.".into());
            return false;
        }

        let machine = if self.client.is_some() {
            self.machines.selected.clone()
        } else {
            self.current().machine.clone()
        };
        let server_supports = self.supports_vcs_worktrees_on(&machine);
        let is_local_draft = self.current().is_draft()
            && !self.current().worktree_bootstrapping()
            && !self.current().native.queue_held
            && self.current().worktree_path.is_none();
        let repo_key = self.thread_vcs_repo_key(self.current());
        let is_git_repo = repo_key
            .as_ref()
            .is_some_and(|k| self.vcs_branches.get(k).is_some_and(|e| e.is_repo));
        let base_branch = self
            .current()
            .base_branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                repo_key
                    .as_ref()
                    .and_then(|k| self.vcs_branches.get(k))
                    .filter(|e| e.is_repo)
                    .and_then(|e| {
                        crate::branch_toolbar::default_worktree_base_branch(&e.refs, None)
                    })
            });
        let selections = self.multi_model_selections();
        match crate::model_selection::multiple_model_send_block(
            selections.len(),
            server_supports,
            is_local_draft,
            is_git_repo,
            base_branch.as_deref(),
        ) {
            Some(crate::model_selection::MultipleModelSendBlock::ServerOutdated) => {
                self.current_mut().error =
                    Some("Update this server before starting multiple models.".into());
                return false;
            }
            Some(crate::model_selection::MultipleModelSendBlock::NeedsNewGitThread) => {
                if server_supports && is_local_draft && is_git_repo && base_branch.is_none() {
                    self.current_mut().submission_error =
                        Some("Select a base branch before sending in New worktree mode.".into());
                } else {
                    self.push_toast(
                        ToastKind::Warning,
                        "Choose models and a base branch",
                        Some(
                            "Multiple models need a new thread in a Git project. Each gets its own worktree."
                                .into(),
                        ),
                        None,
                        Some(5_000),
                    );
                }
                return false;
            }
            None => {}
        }
        let base_branch = base_branch.unwrap();

        let target_server_id = self.machine_identity(&machine);
        let peers = self.peers.clone();
        let labels: Vec<(String, String)> = peers
            .iter()
            .map(|p| (p.server_id.clone(), p.label.clone()))
            .collect();
        let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
            self.current().draft_reply_to.as_ref(),
            Some(&machine),
            Some(&target_server_id),
            &peers,
            &|m| {
                if m == "local" {
                    "Local".into()
                } else {
                    labels
                        .iter()
                        .find(|(id, _)| id == m)
                        .map(|(_, l)| l.clone())
                        .unwrap_or_else(|| m.to_owned())
                }
            },
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.current_mut().attachment_error = Some(error);
                return false;
            }
        };
        if !self.headless
            && let Some(error) = self.current().validate_prompt_length(&prompt)
        {
            self.current_mut().submission_error = Some(error);
            return false;
        }

        let snapshot = crate::composer_rich::Snapshot::capture_with_context(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
        );
        let reply_to = self.current().draft_reply_to.clone();
        let saved_draft_model_state = self.current().draft_model_state.clone();
        let source_thread_id = self.current().id;
        let project = self.current().project;
        let access = self.current().native.access;
        let start_from_origin = self.current().effective_start_from_origin();
        let title_seed: String = if prompt.text.is_empty() {
            prompt
                .images
                .first()
                .map(|image| {
                    format!(
                        "{}: {}",
                        if image.is_image() { "Image" } else { "File" },
                        image.name
                    )
                })
                .unwrap_or_else(|| "New thread".into())
                .chars()
                .take(45)
                .collect()
        } else {
            prompt.text.chars().take(45).collect()
        };

        let targets: Vec<MultiModelTargetEntry> = selections
            .iter()
            .map(|sel| MultiModelTargetEntry {
                display_name: self.model_display_name_for_selection(sel),
                selection: sel.clone(),
                thread_id: None,
                create_cmd_id: None,
                send_cmd_id: None,
                outcome: MultiModelTargetOutcome::Pending,
            })
            .collect();

        // Clear the source draft before starting background threads, keeping the user on the draft.
        {
            let thread = self.current_mut();
            thread.prompt_history.reset();
            thread.typing_guard.on_blur_or_send();
            thread.paste_history = Default::default();
            thread.draft.replace_text("");
            thread.draft_images.clear();
            thread.draft_context.clear();
            thread.draft_reply_to = None;
            thread.retained_context = Default::default();
            thread.error = None;
            thread.attachment_error = None;
            thread.submission_error = None;
            thread.draft_model_state.additional_models.clear();
        }
        self.usage_limits_notice_key = None;
        self.composer_notices.remove("usage-limits");

        if self.client.is_some() {
            self.multi_model_batches.push(MultiModelBatch {
                source_thread_id,
                server: machine,
                project,
                base_branch,
                start_from_origin,
                access,
                prompt,
                snapshot,
                reply_to,
                saved_draft_model_state,
                targets,
                occupied_remote_ids: Vec::new(),
                next_remote_index: 0,
            });
            let batch_idx = self.multi_model_batches.len() - 1;
            self.advance_remote_multi_model_batch(batch_idx);
            self.resolve_completed_multi_model_batches();
            self.checkpoint();
            return !self.multi_model_batches.is_empty()
                || self
                    .toasts
                    .iter()
                    .any(|t| t.kind == ToastKind::Success && t.title.contains("in background"));
        }

        let Some(project_cwd) = self.project(project).map(|p| p.path.clone()) else {
            return false;
        };
        let entries = self.provider_instance_entries();
        let picker_models = self.picker_models_by_instance();
        let has_images = prompt.images.iter().any(Attachment::is_image);
        let worktrees_dir = self.data_dir.join("worktrees");
        #[cfg(test)]
        let git_ctx = self.test_git_context.clone().unwrap_or_default();
        #[cfg(not(test))]
        let git_ctx = crate::vcs::GitContext::default();

        let mut resolved_targets = targets;
        for target in &mut resolved_targets {
            let models = match self.validate_local_multi_model_target(
                &target.selection,
                has_images,
                &entries,
                &picker_models,
            ) {
                Ok(models) => models,
                Err(error) => {
                    target.outcome = MultiModelTargetOutcome::Failed(error);
                    continue;
                }
            };
            let thread_id = self.next_id;
            self.next_id += 1;
            let mut thread = Thread::new(thread_id);
            thread.project = project;
            thread.machine = machine.clone();
            thread.harness = target.selection.instance_id.clone();
            thread.env_mode = Some(ThreadEnvMode::Worktree);
            thread.base_branch = Some(base_branch.clone());
            thread.start_from_origin = Some(start_from_origin);
            thread.native.access =
                resolve_compatible_access_mode(access, supported_access_modes(&thread.harness)).1;
            thread.title = title_seed.clone();
            thread.never_sent = false;
            thread.models = models;
            let provider = if target.selection.instance_id == "pi" {
                "cpa".to_owned()
            } else {
                target.selection.instance_id.clone()
            };
            thread.set_model(provider, target.selection.model.clone());
            for opt in &target.selection.options {
                thread.set_option(opt.id.clone(), opt.value.clone());
            }
            thread.native.queue_held = true;
            thread.serial += 1;
            let queued_id = thread.serial;
            thread.native.queue.push_back(pi_session::QueuedMessage {
                id: queued_id,
                prompt: prompt.clone(),
            });
            thread.worktree_bootstrap = Some(WorktreeBootstrapState {
                snapshot: snapshot.clone(),
                reply_to: reply_to.clone(),
                queued_id,
                fresh_context: false,
            });
            self.threads.push(thread);

            let wake = self.wake.clone();
            let worktrees_dir_thread = worktrees_dir.clone();
            let project_cwd_thread = project_cwd.clone();
            let base_branch_thread = base_branch.clone();
            let git_ctx_thread = git_ctx.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.worktree_bootstrap_rxs.push(rx);
            std::thread::spawn(move || {
                let cwd_str = project_cwd_thread.to_string_lossy().into_owned();
                let raw_branch = crate::vcs::build_temporary_worktree_branch_name(
                    &uuid::Uuid::new_v4().simple().to_string(),
                );
                let new_branch = crate::vcs::resolve_available_branch_name_with(
                    &project_cwd_thread,
                    &raw_branch,
                    &git_ctx_thread,
                )
                .unwrap_or(raw_branch);
                let result = crate::vcs::create_worktree_with(
                    &crate::vcs::VcsCreateWorktreeInput {
                        cwd: cwd_str,
                        ref_name: base_branch_thread.clone(),
                        new_ref_name: Some(new_branch),
                        base_ref_name: Some(base_branch_thread.clone()),
                        path: None,
                        start_from_origin: Some(start_from_origin),
                    },
                    &worktrees_dir_thread,
                    &git_ctx_thread,
                );
                let _ = tx.send(WorktreeBootstrapResult {
                    thread_id,
                    base_branch: base_branch_thread,
                    result,
                });
                wake();
            });

            target.thread_id = Some(thread_id);
            target.outcome = MultiModelTargetOutcome::Pending;
        }

        let any_started_or_pending = resolved_targets
            .iter()
            .any(|t| !matches!(t.outcome, MultiModelTargetOutcome::Failed(_)));
        self.multi_model_batches.push(MultiModelBatch {
            source_thread_id,
            server: machine,
            project,
            base_branch,
            start_from_origin,
            access,
            prompt,
            snapshot,
            reply_to,
            saved_draft_model_state,
            targets: resolved_targets,
            occupied_remote_ids: Vec::new(),
            next_remote_index: 0,
        });
        self.resolve_completed_multi_model_batches();
        self.checkpoint();
        any_started_or_pending
    }

    pub(super) fn release_after_worktree_setup(&mut self, index: usize) {
        if index >= self.threads.len() {
            return;
        }
        if self.threads[index].worktree_bootstrap_fresh_context {
            self.persist_transcripts = true;
        }
        self.threads[index].native.queue_held = false;
        if self.runtime_enabled || self.threads[index].pi.is_some() {
            if self.threads[index].pi.is_none() {
                self.connect_thread(index);
            }
            if self.threads[index].pi.is_some() {
                self.threads[index].drive_session();
            }
        }
    }

    pub(super) fn spawn_worktree_setup_runner(&mut self, thread_id: usize) {
        let Some(index) = self.threads.iter().position(|t| t.id == thread_id) else {
            return;
        };
        let Some(project_cwd) = self
            .project(self.threads[index].project)
            .map(|p| p.path.to_string_lossy().into_owned())
        else {
            return;
        };
        let Some(wt_path) = self.threads[index].worktree_path.clone() else {
            return;
        };
        self.threads[index].worktree_setup_seq =
            self.threads[index].worktree_setup_seq.saturating_add(1);
        let run_seq = self.threads[index].worktree_setup_seq;
        let wake = self.wake.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.worktree_setup_rxs.push(rx);
        std::thread::spawn(move || {
            let result = crate::vcs::run_setup(&crate::vcs::VcsRunSetupInput {
                cwd: project_cwd,
                worktree_path: Some(wt_path),
                timeout_ms: None,
            });
            let _ = tx.send(WorktreeSetupResultMsg {
                thread_id,
                run_seq,
                result,
            });
            wake();
        });
    }

    pub fn poll_worktree_setup(&mut self) -> bool {
        let mut changed = false;
        let mut remaining = Vec::with_capacity(self.worktree_setup_rxs.len());
        let rxs = std::mem::take(&mut self.worktree_setup_rxs);
        for rx in rxs {
            match rx.try_recv() {
                Ok(msg) => {
                    changed = true;
                    let Some(index) = self.threads.iter().position(|t| t.id == msg.thread_id)
                    else {
                        continue;
                    };
                    if self.threads[index].worktree_setup_seq != msg.run_seq {
                        continue;
                    }
                    match msg.result {
                        Ok(res) if res.status == "no_script" => {
                            self.threads[index].worktree_setup = None;
                            if self.threads[index].placement.is_some() {
                                self.finish_placement_ready(index);
                            } else {
                                self.release_after_worktree_setup(index);
                            }
                        }
                        Ok(res) if res.exit_code == Some(0) && !res.timed_out => {
                            let existing = self.threads[index].worktree_setup.take();
                            self.threads[index].worktree_setup = Some(WorktreeSetupState {
                                phase: WorktreeSetupPhase::Succeeded,
                                script_name: res.script_name.or_else(|| {
                                    existing.as_ref().and_then(|s| s.script_name.clone())
                                }),
                                script_command: res.script_command.or_else(|| {
                                    existing.as_ref().and_then(|s| s.script_command.clone())
                                }),
                                error: None,
                                output: res.output,
                                exit_code: res.exit_code,
                                timed_out: false,
                                duration_ms: res.duration_ms,
                                skipped: false,
                            });
                            if self.threads[index].placement.is_some() {
                                self.finish_placement_ready(index);
                            } else {
                                self.release_after_worktree_setup(index);
                            }
                        }
                        Ok(res) => {
                            let existing = self.threads[index].worktree_setup.take();
                            let error = if res.timed_out {
                                "Setup script timed out.".to_owned()
                            } else if let Some(code) = res.exit_code {
                                format!("Setup script exited with code {code}.")
                            } else {
                                "Setup script failed.".to_owned()
                            };
                            self.threads[index].worktree_setup = Some(WorktreeSetupState {
                                phase: WorktreeSetupPhase::Failed,
                                script_name: res.script_name.or_else(|| {
                                    existing.as_ref().and_then(|s| s.script_name.clone())
                                }),
                                script_command: res.script_command.or_else(|| {
                                    existing.as_ref().and_then(|s| s.script_command.clone())
                                }),
                                error: Some(error.clone()),
                                output: res.output,
                                exit_code: res.exit_code,
                                timed_out: res.timed_out,
                                duration_ms: res.duration_ms,
                                skipped: false,
                            });
                            if self.threads[index].placement.is_some() {
                                self.fail_placement(index, error);
                            } else {
                                self.threads[index].worktree_setup_output_open = true;
                                self.threads[index].native.queue_held = true;
                            }
                        }
                        Err(error) => {
                            let existing = self.threads[index].worktree_setup.take();
                            self.threads[index].worktree_setup = Some(WorktreeSetupState {
                                phase: WorktreeSetupPhase::Failed,
                                script_name: existing.as_ref().and_then(|s| s.script_name.clone()),
                                script_command: existing
                                    .as_ref()
                                    .and_then(|s| s.script_command.clone()),
                                error: Some(error.clone()),
                                output: existing.map(|s| s.output).unwrap_or_default(),
                                exit_code: None,
                                timed_out: false,
                                duration_ms: 0,
                                skipped: false,
                            });
                            if self.threads[index].placement.is_some() {
                                self.fail_placement(index, error);
                            } else {
                                self.threads[index].worktree_setup_output_open = true;
                                self.threads[index].native.queue_held = true;
                            }
                        }
                    }
                    self.checkpoint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => changed = true,
            }
        }
        self.worktree_setup_rxs = remaining;
        changed
    }

    pub fn retry_worktree_setup(&mut self) {
        if self.forward("thread.worktreeSetup", json!({"action": "retry"})) {
            return;
        }
        let thread = self.current_mut();
        let Some(setup) = thread.worktree_setup.as_mut() else {
            return;
        };
        if setup.phase != WorktreeSetupPhase::Failed || thread.worktree_path.is_none() {
            return;
        }
        if let Some(placement) = thread.placement.as_mut() {
            placement.phase = placement::PlacementPhase::Pending;
            placement.error = None;
        }
        setup.phase = WorktreeSetupPhase::Running;
        setup.error = None;
        setup.output.clear();
        setup.exit_code = None;
        setup.timed_out = false;
        setup.duration_ms = 0;
        setup.skipped = false;
        thread.native.queue_held = true;
        let tid = thread.id;
        self.spawn_worktree_setup_runner(tid);
        self.checkpoint();
    }

    pub fn skip_worktree_setup(&mut self) {
        if self.forward("thread.worktreeSetup", json!({"action": "skip"})) {
            return;
        }
        let index = self.active;
        let thread = &mut self.threads[index];
        let Some(setup) = thread.worktree_setup.as_mut() else {
            return;
        };
        if setup.phase == WorktreeSetupPhase::Succeeded {
            return;
        }
        thread.worktree_setup_seq = thread.worktree_setup_seq.saturating_add(1);
        setup.phase = WorktreeSetupPhase::Succeeded;
        setup.skipped = true;
        setup.error = None;
        if thread.placement.is_some() {
            self.finish_placement_ready(index);
        } else {
            self.release_after_worktree_setup(index);
        }
        self.checkpoint();
    }
}
