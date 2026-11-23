use super::*;

impl Thread {
    pub(crate) fn execution_quiescent(&self) -> bool {
        self.pi.is_none()
            && self.connection_owner.quiescent()
            && !self.status.busy()
            && self.pending_prompt.is_none()
            && self.queued_prompt.is_none()
            && self.pending_injections.is_empty()
            && self.peer_provider_inputs.is_empty()
            && self.injection_id.is_none()
            && self.native.queue.is_empty()
            && !self.peer_recovery_held
            && !self.initializing
            && !self.configuring
            && self.request.is_none()
    }

    pub(super) fn note_provider_prompt_accepted(&mut self) {
        if let Some(run) = &self.run_id {
            self.provider_accepted_run = Some(run.clone());
        }
    }

    pub(super) fn note_provider_running(&mut self) {
        if let Some(run) = &self.run_id {
            self.provider_running_run = Some(run.clone());
        }
    }

    pub(super) fn note_provider_terminal(&mut self, status: &str) {
        if let Some(run) = &self.run_id {
            self.provider_terminal_run = Some(run.clone());
            self.provider_terminal_status = Some(status.into());
            if let Some(binding) = &self.execution
                && binding.run_id == *run
            {
                self.execution_settlements
                    .push((binding.clone(), status.to_owned()));
            }
        }
    }

    pub fn is_fresh_session_message(&self, message_id: &str) -> bool {
        self.fresh_session_messages
            .iter()
            .any(|id| id == message_id)
    }

    #[cfg(test)]
    pub fn fresh_session_command_available_local(&self) -> bool {
        !self.messages.is_empty()
            && crate::harness::supports_fresh_context(&self.harness)
            && self.request.is_none()
            && self.native.questionnaire.is_none()
    }

    pub fn can_choose_harness(&self) -> bool {
        if self.remote.is_some() {
            return self.projected_harness_choice;
        }
        !self.delegated
            && !self.archived()
            && !self.status.busy()
            && !self.configuring
            && self.messages.is_empty()
            && self.life.latest_user_message_at.is_none()
            && self.native.fork_source.is_none()
            && self.native.previous_session.is_none()
            && (self.session.is_none() || self.state_loaded && self.history_loaded)
    }
    pub fn pinned(&self) -> bool {
        self.life.pinned_at.is_some()
    }
    pub fn archived(&self) -> bool {
        self.life.archived_at.is_some()
    }
    pub fn unread(&self) -> bool {
        self.life.unread()
    }
    pub fn is_draft(&self) -> bool {
        !self.captain_home
            && !self.delegated
            && !self.archived()
            && !self.pinned()
            && !self.life.settled()
            && self.life.snoozed_at.is_none()
            && self.life.failed_at.is_none()
            && !self.worktree_bootstrapping()
            && self.status == Status::Ready
            && self.unused_session()
            && self.messages.is_empty()
            && self.life.latest_user_message_at.is_none()
            && self.life.last_completed_at.is_none()
            && self.client_creating.is_none()
            && self.remote_send.is_none()
            && self.client_promoted.is_none()
    }
    /// No session, or one that was only warmed for a thread nothing has been sent in.
    pub(super) fn unused_session(&self) -> bool {
        self.session.is_none() || self.never_sent
    }
    /// A new thread slot with nothing queued, running, saved or named in it. Its title becomes
    /// whatever the user types, before any prompt is sent.
    pub(super) fn blank_draft(&self) -> bool {
        !self.captain_home
            && !self.delegated
            && !self.archived()
            && !self.pinned()
            && !self.life.settled()
            && self.life.snoozed_at.is_none()
            && self.life.failed_at.is_none()
            && self.life.latest_user_message_at.is_none()
            && self.life.last_completed_at.is_none()
            && self.status == Status::Ready
            && self.unused_session()
            && self.messages.is_empty()
            && !self.has_draft_content()
            && self.draft_model_state.additional_models.is_empty()
            && self.remote_send.is_none()
            && self.client_creating.is_none()
            && !self.configuring
            && !self.queued()
            && !self.renamed
            && self.parent.is_none()
    }
    /// Working, in every count the user reads (the header's background line, the crew line,
    /// the band's sails): running now. Queued, interrupted, finished and approval-blocked
    /// threads are not working.
    pub fn running_now(&self) -> bool {
        self.status == Status::Running
            && self.request.is_none()
            && !self.offline_blocked
            && !self.provider_update_held
    }
    pub fn queued(&self) -> bool {
        self.native.projected_queued
            || self.queued_prompt.is_some()
            || self.pending_prompt.is_some()
            || !self.pending_injections.is_empty()
            || !self.peer_provider_inputs.is_empty()
            || self.native_pending_turns()
    }
    pub fn has_draft_content(&self) -> bool {
        !self.draft.text.trim().is_empty()
            || self.draft.pills().next().is_some()
            || !self.draft_images.is_empty()
            || self.draft_reply_to.is_some()
    }
    /// Add a validated attachment while preserving the text draft and its undo history.
    pub(crate) fn insert_attachment_chips(
        &mut self,
        attachments: &[Attachment],
        skip_image_inline_chip: bool,
    ) {
        self.insert_attachment_chips_inner(attachments, skip_image_inline_chip, false);
    }
    pub(crate) fn insert_prepared_image_chips(
        &mut self,
        attachments: &[Attachment],
        contextual: bool,
    ) {
        self.insert_attachment_chips_inner(attachments, !contextual, contextual);
    }
    pub(super) fn insert_attachment_chips_inner(
        &mut self,
        attachments: &[Attachment],
        skip_image_inline_chip: bool,
        force_image_chip: bool,
    ) {
        if attachments.is_empty()
            || self
                .request
                .as_ref()
                .is_some_and(|r| r["method"] != "confirm")
                && !self.answer_intake
                && !self.request_held_by_typing_guard()
        {
            return;
        }
        let prose = crate::composer_context::strip_inline_context_references(
            &crate::composer_rich::markdown(&self.draft),
        );
        let image_attachments_get_chips = force_image_chip
            || !skip_image_inline_chip
                && (self.answer_intake
                    || self.draft.anchor != self.draft.focus
                    || !prose.trim().is_empty());
        let mut records = Vec::new();
        for attachment in attachments {
            if attachment.is_image() && !image_attachments_get_chips {
                continue;
            }
            let rec = crate::composer_context::attachment_context_record(attachment);
            if let Some(existing) = self
                .draft_context
                .iter_mut()
                .find(|r| r.context_id == rec.context_id)
            {
                *existing = rec.clone();
            } else {
                self.draft_context.push(rec.clone());
            }
            records.push(rec);
        }
        if records.is_empty() {
            return;
        }
        let (start, end) = self.draft.range();
        let prev = self.draft.text[..start].chars().next_back();
        let needs_leading_space = !matches!(prev, None | Some(' ' | '\n' | '\t' | '\r'));
        let replace_end = if self.draft.text[end..].starts_with(' ') {
            end + 1
        } else {
            end
        };
        self.draft.undo_group(|e| {
            e.set_selection(start, replace_end);
            e.insert(if needs_leading_space { " " } else { "" });
            for (index, rec) in records.iter().enumerate() {
                if index > 0 {
                    e.insert(" ");
                }
                let pill_start = e.focus;
                e.insert(&rec.label);
                let pill_end = e.focus;
                let pill_id = crate::composer_context::format_composer_context_href(
                    rec.kind(),
                    &rec.context_id,
                );
                e.replace_with_pill(pill_start..pill_end, pill_id, &rec.label);
            }
            e.insert(" ");
        });
    }

    /// Insert a context pill into the draft. The returned `InsertedSpaces` say
    /// which spaces the insert added, so a cancelled citation edit can undo it
    /// (KSG's `insertedSpaces`); `None` means nothing was inserted.
    pub(crate) fn insert_context_chip(
        &mut self,
        record: crate::composer_context::ComposerContextRecord,
    ) -> Option<crate::ui::citation_comment::InsertedSpaces> {
        if self.request.is_some() {
            return None;
        }
        let (start, end) = if self.draft.focused {
            self.draft.range()
        } else {
            let end = self.draft.text.len();
            (end, end)
        };
        let prev = self.draft.text[..start].chars().next_back();
        let needs_leading_space = !matches!(prev, None | Some(' ' | '\n' | '\t' | '\r'));
        let replace_end = if self.draft.text[end..].starts_with(' ') {
            end + 1
        } else {
            end
        };
        let pill_id = crate::composer_context::format_composer_context_href(
            record.kind(),
            &record.context_id,
        );
        // A citation chip carries a gutter at its right end for the comment
        // action overlay (KSG's ContextChipAction inside the chip).
        let chip_label = if matches!(
            record.data,
            crate::composer_context::ComposerContextData::Response { .. }
        ) {
            crate::ui::citation_comment::chip_text(&record.label)
        } else {
            record.label.clone()
        };
        if let Some(existing) = self
            .draft_context
            .iter_mut()
            .find(|r| r.context_id == record.context_id)
        {
            *existing = record;
        } else {
            self.draft_context.push(record);
        }
        self.draft.undo_group(|e| {
            e.set_selection(start, replace_end);
            e.insert(if needs_leading_space { " " } else { "" });
            let pill_start = e.focus;
            e.insert(&chip_label);
            let pill_end = e.focus;
            e.replace_with_pill(pill_start..pill_end, pill_id, &chip_label);
            e.insert(" ");
        });
        Some(crate::ui::citation_comment::InsertedSpaces {
            before: needs_leading_space,
            after: replace_end == end,
        })
    }

    pub fn add_attachment(&mut self, image: Attachment) -> Result<(), String> {
        if self
            .draft_images
            .iter()
            .any(|current| current.id == image.id)
        {
            return Err(format!("{} is already attached.", image.name));
        }
        let mut proposed = self.draft_images.clone();
        proposed.push(image);
        if self.answer_intake {
            if self.question_reserved_count() >= attachments::MAX_ATTACHMENTS {
                return Err(format!(
                    "You can attach up to {} files per message.",
                    attachments::MAX_ATTACHMENTS
                ));
            }
        } else {
            attachments::validate_prompt_images(&proposed)?;
        }
        self.draft_images = proposed;
        self.attachment_error = None;
        Ok(())
    }
    pub fn remove_attachment(&mut self, id: &str) {
        let mut context_ids: std::collections::HashSet<_> = self
            .draft_context
            .iter()
            .filter_map(|record| match &record.data {
                crate::composer_context::ComposerContextData::Image { attachment_id, .. }
                | crate::composer_context::ComposerContextData::File { attachment_id, .. }
                    if attachment_id == id =>
                {
                    Some(record.context_id.clone())
                }
                _ => None,
            })
            .collect();
        if let Some(attachment) = self.draft_images.iter().find(|a| a.id == id) {
            context_ids.insert(crate::composer_context::to_kind_scoped_composer_context_id(
                if attachment.is_image() {
                    "image"
                } else {
                    "file"
                },
                id,
            ));
        }
        let ranges: Vec<_> = self
            .draft
            .pills()
            .filter_map(|(range, href)| {
                let (_, context_id) = crate::composer_context::parse_composer_context_href(href)?;
                context_ids.contains(&context_id).then_some(range)
            })
            .collect();
        if !ranges.is_empty() {
            self.draft.undo_group(|edit| {
                for range in ranges.into_iter().rev() {
                    let mut start = range.start;
                    let mut end = range.end;
                    if edit.text[end..].starts_with(' ') {
                        end += 1;
                    } else if edit.text[..start].ends_with(' ') {
                        start -= 1;
                    }
                    edit.set_selection(start, end);
                    edit.insert("");
                }
            });
        }
        self.draft_context
            .retain(|record| !context_ids.contains(&record.context_id));
        for id in context_ids {
            self.retained_context.records.remove(&id);
        }
        self.draft_images.retain(|image| image.id != id);
        self.attachment_error = if self.answer_intake {
            None
        } else {
            attachments::validate_prompt_images(&self.draft_images).err()
        };
    }
    pub(super) fn image_model_error(&self) -> String {
        let model = if self.model.is_empty() {
            "The selected model"
        } else {
            &self.model
        };
        if self.supports_images == Some(false) {
            format!(
                "{model} does not accept image input. Remove the images or choose a model that supports images."
            )
        } else {
            format!(
                "{model} does not declare image input. Remove the images or choose a model that supports images."
            )
        }
    }
    pub fn new(id: usize) -> Self {
        Self {
            remote: None,
            remote_send: None,
            remote_send_fresh_arm: None,
            remote_send_reply_to: None,
            fresh_session_arm: None,
            fresh_session_messages: Vec::new(),
            captain_return_context: false,
            handoffs: Vec::new(),
            handoff_sessions: std::collections::BTreeMap::new(),
            staged_handoff: None,
            inflight_handoff: None,
            handoff_revert: None,
            handoff_package: None,
            composer_context_supported: true,
            fallback_active: None,
            fallback_explicit_override: false,
            background_start_pending: false,
            background_send: None,
            pending_send_and_new_thread: None,
            worktree_bootstrap: None,
            remote_bootstrap_snapshot: None,
            queued_edit: None,
            queued_save: None,
            queue_edit_supported: true,
            run_id: None,
            execution: None,
            next_execution: None,
            peer_input_executions: Default::default(),
            peer_provider_inputs: Default::default(),
            execution_acceptances: Vec::new(),
            execution_settlements: Vec::new(),
            provider_accepted_run: None,
            provider_running_run: None,
            provider_terminal_run: None,
            provider_terminal_status: None,
            resumable_run_id: None,
            continuation_of_run_id: None,
            prompt_suggestion: None,
            prompt_suggestions_enabled: false,
            stop_control: StopControl::default(),
            run_control_supported: true,
            resume_command: None,
            request_generation: uuid::Uuid::new_v4().to_string(),
            server_owned: false,
            client_draft: false,
            crew_draft: None,
            crew_draft_notice: None,
            client_creating: None,
            client_promoted: None,
            recovery_command: None,
            projected_harness_choice: false,
            id,
            harness: "codex".into(),
            harness_context_sent: false,
            harness_state: json!({}),
            project: None,
            worktree_path: None,
            scratch_directory: None,
            placement: None,
            branch: None,
            base_branch: None,
            env_mode: None,
            start_from_origin: None,
            worktree_setup: None,
            worktree_setup_output_open: false,
            worktree_setup_seq: 0,
            worktree_bootstrap_fresh_context: false,
            machine: "local".into(),
            peer_origin: None,
            stop_serial: 0,
            updated: crate::projects::now(),
            title: "New thread".into(),
            renamed: false,
            status: Status::Ready,
            offline_blocked: false,
            provider_update_held: false,
            messages: vec![],
            tools: vec![],
            agent_index: Default::default(),
            projected_agent_counts: None,
            thoughts: vec![],
            work: vec![],
            transcript_fields: std::collections::HashMap::new(),
            transcript_projection: None,
            transcript_disclosures: client_state::Disclosures::default(),
            transcript_structure: 0,
            transcript_content: 0,
            timeline_cache: Default::default(),
            live_work_collapsed: false,
            draft: crate::composer_rich::draft(""),
            prompt_history: crate::composer::History::default(),
            typing_guard: crate::composer::TypingGuard::default(),
            paste_history: crate::composer_paste::PasteHistory::default(),
            composer_rules: crate::composer_rich::Rules::default(),
            composer_source: crate::composer_source::MarkdownSourceState::default(),
            code_highlight: Default::default(),
            draft_images: Vec::new(),
            draft_context: Vec::new(),
            draft_reply_to: None,
            retained_context: Default::default(),
            attachment_error: None,
            submission_error: None,
            pi: None,
            supports_images: None,
            model: DEFAULT_MODEL.into(),
            thinking: String::new(),
            model_provider: String::new(),
            model_id: String::new(),
            context_window: None,
            context: ContextMeter::default(),
            native: PiSession::default(),
            models: vec![],
            thinking_levels: vec![],
            configuring: false,
            phase: String::new(),
            error: None,
            diagnostics: vec![],
            request: None,
            peer_document_read: None,
            requests_supported: true,
            question_contract_supported: true,
            responding_request: None,
            approval_send: None,
            reply: TextEdit::default(),
            question_drafts: Default::default(),
            answer_intake: false,
            answer_scope: None,
            question_submit_requested: false,
            question_submission: None,
            question_resolutions: Vec::new(),
            session: None,
            never_sent: true,
            life: Lifecycle::new(lifecycle::now()),
            working: Working::default(),
            started: None,
            elapsed: None,
            captain: None,
            captain_home: false,
            crew_captain: None,
            captain_context: String::new(),
            captain_context_inputs: None,
            instruction_role: None,
            shina: crate::model::capabilities::THREAD_INSTRUCTIONS,
            instruction_context_pending: false,
            delegated: false,
            parent: None,
            host: None,
            connection_owner: Arc::default(),
            assistant: None,
            serial: 0,
            state_loaded: false,
            history_loaded: false,
            history_has_more: false,
            history_loading: false,
            history_error: None,
            deferred_work: Vec::new(),
            deferred_work_supported: false,
            initializing: false,
            session_started: None,
            last_activity: Instant::now(),
            prewarm_attempted: false,
            selected_model: None,
            selected_thinking: None,
            model_options: vec![],
            draft_model_state: Default::default(),
            startup_model_applied: false,
            startup_thinking_applied: false,
            startup_cursor_option: 0,
            pending_cursor_option: None,
            cursor_catalog_models: None,
            live_option_descriptors: None,
            startup_selection_check: None,
            startup_selection_verified: false,
            context_startup_requested: false,
            queued_prompt: None,
            retry_prompt: None,
            pending_thinking: None,
            pending_prompt: None,
            pending_injections: VecDeque::new(),
            injection_id: None,
            injection_dispatches: Vec::new(),
            rejected_injections: Vec::new(),
            peer_prompt: None,
            peer_recovery_held: false,
            peer_input_history: Default::default(),
            peer_dispatches: Vec::new(),
            injection_retry_at: None,
            operation_deadline: None,
            task_result: None,
            completion_notified: true,
            forwarded_question: None,
            accepted_deliveries: Vec::new(),
            crew_report_run: None,
            crew_report_policy: ReportPolicy::React,
            crew_report_outbox: Vec::new(),
            crew_report_last_run: None,
            recovering_task: false,
            task_history: None,
            task_delivery_probe_started: false,
            task_result_history: None,
            admission: None,
            admission_run: 0,
            admission_blocked: false,
            memory: None,
            learned_turn: None,
            memory_pending: VecDeque::new(),
            memory_context: Default::default(),
            memory_staged: Vec::new(),
            memory_suggestions: Vec::new(),
            scope_context_pending: false,
        }
    }

    pub fn connect_with_instructions(
        &mut self,
        path: PathBuf,
        wake: Waker,
        instructions: Option<String>,
    ) {
        if self.pi.is_some() || self.provider_update_held {
            return;
        }
        self.request_generation = uuid::Uuid::new_v4().to_string();
        let backend = match crate::backend::BackendHost::resolve(&self.machine) {
            Ok(host) => host,
            Err(error) => {
                self.fail_session(error);
                return;
            }
        };
        self.error = None;
        if !self.pending_injections.is_empty() && !self.status.busy() {
            self.arm_task();
            self.status = Status::Running;
            self.started = None;
            self.phase = "Working".into();
        } else if self.status == Status::Failed {
            self.status = Status::Ready;
        }
        self.initializing = true;
        self.prewarm_attempted = true;
        self.session_started = Some(Instant::now());
        self.last_activity = Instant::now();
        self.state_loaded = false;
        self.history_loaded = false;
        self.peer_input_history = None;
        self.startup_model_applied = false;
        self.startup_thinking_applied = false;
        self.startup_cursor_option = 0;
        self.pending_cursor_option = None;
        self.live_option_descriptors = None;
        self.startup_selection_check = None;
        self.startup_selection_verified = false;
        self.context_startup_requested = false;
        self.context.begin_model_change();
        let source = self.planned_session_source();
        match crate::harness::start(
            backend,
            crate::harness::Start {
                id: self.harness.clone(),
                workspace: path,
                source,
                instructions,
                shina: self.shina_instructions(),
                link: self.host.clone(),
                owner: self.connection_owner.clone(),
                context_sent: self.harness_context_sent,
                prompt_suggestions: self.prompt_suggestions_enabled && !self.delegated,
                metadata_only: false,
            },
            wake,
            &self.models,
        ) {
            Ok(session) => self.pi = Some(session),
            Err(error) => self.fail_session(error),
        }
    }

    pub(crate) fn planned_session_source(&self) -> crate::backend::SessionSource {
        self.native
            .fork_source
            .clone()
            .map(crate::backend::SessionSource::Fork)
            .or_else(|| {
                self.session
                    .clone()
                    .map(crate::backend::SessionSource::Resume)
            })
            .unwrap_or(crate::backend::SessionSource::New)
    }

    pub fn set_model(&mut self, provider: String, model_id: String) {
        let handoff_switch = self.has_started_session()
            && provider
                != if self.harness == "pi" {
                    "cpa"
                } else {
                    self.harness.as_str()
                };
        if self.forward(
            "thread.configure",
            if handoff_switch {
                json!({"provider":provider,"model":model_id,"handoff":true})
            } else {
                json!({"provider":provider,"model":model_id})
            },
        ) {
            return;
        }
        // KSG: a different harness on a started thread stages a context handoff instead
        // of being refused, when the current state allows the switch.
        if handoff_switch {
            match self.handoff_block_reason() {
                None => {
                    self.attachment_error = None;
                    self.stage_handoff(provider, model_id);
                    return;
                }
                Some(reason) => {
                    self.error = Some(reason);
                    return;
                }
            }
        }
        if self.harness == "pi" && provider != "cpa"
            || self.harness != "pi" && provider != self.harness
        {
            self.error = Some(if self.harness == "pi" {
                "Shina uses CPA proxy models only. Choose a model from the CPA catalog.".into()
            } else {
                "Choose a model advertised by this thread's ACP harness.".into()
            });
            return;
        }
        if self.status.busy() || self.configuring {
            return;
        }
        self.attachment_error = None;
        if self.model_id != model_id {
            self.model_options.clear();
            if self.harness == "codex" {
                self.harness_state["serviceTier"] = Value::Null;
            }
        }
        self.selected_model = Some((provider.clone(), model_id.clone()));
        if self.pi.is_none() || self.initializing {
            let known = self
                .models
                .iter()
                .find(|m| m.id == model_id)
                .map(|m| (m.name.clone(), m.context_window, m.supports_images));
            if self.model_id != model_id {
                self.context.begin_model_change();
                self.context_window = None;
                self.supports_images = None;
            }
            self.model_provider = provider;
            self.model_id = model_id;
            match known {
                Some((name, window, images)) => {
                    self.model = name;
                    if window.is_some() {
                        self.context_window = window;
                    }
                    self.supports_images = images;
                }
                None => self.model = self.model_id.clone(),
            }
            self.refresh_thinking_levels();
        } else if self.command("set_model", json!({"provider":provider,"modelId":model_id})) {
            self.configuring = true;
        }
        self.native.access = resolve_compatible_access_mode(
            self.native.access,
            supported_access_modes(&self.harness),
        )
        .1;
    }

    pub fn set_thinking(&mut self, level: String) {
        if self.forward("thread.configure", json!({"thinking":level})) {
            return;
        }
        if !self.can_choose_thinking() {
            return;
        }
        self.selected_thinking = Some(level.clone());
        if self.configuring {
            // An earlier configure is still awaiting its response. The wanted level is
            // recorded; the acknowledgement path reconciles it in order instead of
            // sending a second, concurrent change.
            return;
        }
        if self.pi.is_none() || self.initializing {
            self.thinking = level;
        } else if self.command("set_thinking_level", json!({"level":level})) {
            self.configuring = true;
            self.pending_thinking = Some(level);
        }
    }

    /// A second effort pick can queue behind the first. A pending model change still locks
    /// effort because its available levels can change with the model.
    pub fn can_choose_thinking(&self) -> bool {
        !self.status.busy() && (!self.configuring || self.pending_thinking.is_some())
    }

    /// After a configure response clears `configuring`, send the reasoning level the user
    /// last chose when it still differs from what the provider applied. Runs serialized,
    /// one command at a time, so rapid picks during an in-flight acknowledgement land in
    /// order instead of racing or being lost.
    pub(super) fn reconcile_thinking(&mut self) {
        if self.pi.is_none() || self.initializing || self.status.busy() || self.configuring {
            return;
        }
        let Some(level) = self.selected_thinking.clone() else {
            return;
        };
        if level == self.thinking {
            return;
        }
        if self.command("set_thinking_level", json!({"level": level})) {
            self.configuring = true;
            self.pending_thinking = Some(level);
        }
    }

    /// A refused `set_thinking_level` leaves no trace of the level the provider would not
    /// run: `model_options` and the draft's pinned selections revert to `self.thinking`,
    /// so the reasoning menu and the next composer render what is actually in effect.
    pub(super) fn revert_rejected_thinking(&mut self) {
        let rejected = self.pending_thinking.clone();
        let applied = self.thinking.clone();
        let revert_option = |option: &mut crate::model_selection::ProviderOptionSelection| {
            if matches!(
                option.id.as_str(),
                "thinking" | "effort" | "reasoningEffort" | "reasoning" | "thought_level"
            ) && option.value.as_str() == rejected.as_deref()
            {
                option.value = crate::model_selection::OptionValue::Str(applied.clone());
            }
        };
        for option in &mut self.model_options {
            revert_option(option);
        }
        for selection in self
            .draft_model_state
            .model_selection_by_provider
            .values_mut()
        {
            for option in &mut selection.options {
                revert_option(option);
            }
        }
    }

    /// The pinned choice if one exists, otherwise the model Pi last applied.
    pub(super) fn selected_or_effective_model(&self) -> Option<(String, String)> {
        let expected = if self.harness == "pi" {
            "cpa"
        } else {
            &self.harness
        };
        self.selected_model
            .as_ref()
            .filter(|(provider, model)| provider == expected && !model.trim().is_empty())
            .cloned()
            .or_else(|| {
                (self.model_provider == expected && !self.model_id.trim().is_empty())
                    .then(|| (self.model_provider.clone(), self.model_id.clone()))
            })
    }

    pub(super) fn selected_or_effective_reasoning(&self) -> Option<String> {
        self.selected_thinking
            .clone()
            .filter(|level| !level.trim().is_empty())
            .or_else(|| (!self.thinking.trim().is_empty()).then(|| self.thinking.clone()))
    }

    /// What a delegated child inherits when its target omits a model or level.
    pub(super) fn inherited_selection(&self) -> Option<Selection> {
        let (provider, model) = self.selected_or_effective_model()?;
        Some(Selection {
            provider,
            model,
            reasoning: self.selected_or_effective_reasoning(),
        })
    }

    /// What the thread is actually running now, for status and read output.
    pub(super) fn effective_selection(&self) -> Option<Selection> {
        let provider = if self.harness == "pi" {
            "cpa"
        } else {
            &self.harness
        };
        if self.model_provider != provider || self.model_id.trim().is_empty() {
            return None;
        }
        Some(Selection {
            provider: self.model_provider.clone(),
            model: self.model_id.clone(),
            reasoning: (!self.thinking.trim().is_empty()).then(|| self.thinking.clone()),
        })
    }

    pub fn set_machine(&mut self, machine: impl Into<String>) {
        let machine = machine.into();
        if self.machine != machine {
            self.machine = machine;
            self.prompt_suggestion = None;
        }
    }

    pub fn command(&mut self, kind: &str, mut data: Value) -> bool {
        if kind == "set_model" && self.harness == "cursor" {
            let models = self
                .cursor_catalog_models
                .as_deref()
                .unwrap_or(self.models.as_slice());
            data["fromCursorCatalog"] = json!(models.iter().any(|model| {
                model.provider == "cursor" && model.id == data["modelId"] && !model.is_unavailable
            }));
        }
        if kind == "prompt" {
            self.prompt_suggestion = None;
            if data.get("runId").is_none()
                && let Some(run_id) = &self.run_id
            {
                data["runId"] = json!(run_id);
            }
            if self.harness == "codex" {
                data["serviceTier"] = self.harness_state["serviceTier"].clone();
            }
        }
        self.track_learned_turn(kind, &data);
        let context = if kind == "prompt" {
            self.prepare_memory_prompt(&mut data)
        } else {
            None
        };
        self.serial += 1;
        data["type"] = kind.into();
        data["id"] = format!("{}-{}", self.id, self.serial).into();
        if let Err(error) = self.send_memory_command(data) {
            self.error = Some(error);
            return false;
        }
        if kind == "prompt" {
            self.instruction_context_pending = false;
        }
        if let Some(context) = context {
            self.memory_context = context;
            self.scope_context_pending = false;
        }
        self.last_activity = Instant::now();
        match kind {
            "set_model" => {
                self.operation_deadline =
                    Some(("set_model", Instant::now() + Duration::from_secs(30)))
            }
            "set_thinking_level" => {
                self.operation_deadline = Some((
                    "set_thinking_level",
                    Instant::now() + Duration::from_secs(30),
                ))
            }
            "abort" => {
                self.operation_deadline = Some(("abort", Instant::now() + Duration::from_secs(10)))
            }
            _ => {}
        }
        true
    }

    pub(crate) fn refresh_estimated_breakdown(&mut self) {
        if self.harness == "claude" {
            return;
        }
        let Some(used_tokens) = self.context.usage.tokens.filter(|tokens| *tokens > 0) else {
            self.context.usage.breakdown = None;
            return;
        };
        let totals = crate::context_breakdown::thread_char_totals(
            &self.messages,
            &self.tools,
            &self.thoughts,
            &self.native.compactions,
        );
        self.context.usage.breakdown =
            crate::context_breakdown::estimate_context_breakdown(&totals, used_tokens);
    }

    /// Asks Pi for the current context window. Event-driven only: startup, a completed turn,
    /// a model change and compaction completion call this. Nothing polls on a timer.
    pub(super) fn refresh_context_usage(&mut self) -> bool {
        if self.pi.is_none()
            || self.configuring
            || self.context.is_pending()
            || self.context.phase == crate::context::ContextPhase::Compacting
        {
            return false;
        }
        // Telemetry transport errors must not replace the conversation's error.
        let previous_error = self.error.clone();
        if !self.command("get_session_stats", json!({})) {
            self.error = previous_error;
            return false;
        }
        self.context
            .requested(format!("{}-{}", self.id, self.serial));
        true
    }

    #[cfg(test)]
    pub fn send(&mut self) {
        self.send_with_fresh(None, &[], &str::to_owned, false);
    }

    pub(crate) fn send_with_fresh(
        &mut self,
        target_server_id: Option<&str>,
        peers: &[crate::peer::Peer],
        machine_name: &dyn Fn(&str) -> String,
        fresh_context: bool,
    ) {
        if self.offline_blocked || self.status == Status::Paused {
            return;
        }
        if self.status.busy() || self.configuring || !self.has_draft_content() {
            return;
        }
        if self.harness == "pi" && !self.model_provider.is_empty() && self.model_provider != "cpa" {
            self.error = Some("Choose a CPA model before sending. Shina will not send prompts to a direct provider.".into());
            return;
        }
        if self.draft_images.iter().any(Attachment::is_image) && self.supports_images == Some(false)
        {
            self.attachment_error = Some(self.image_model_error());
            return;
        }
        let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
            &self.draft,
            &self.draft_images,
            &self.draft_context,
            self.draft_reply_to.as_ref(),
            Some(&self.machine),
            target_server_id,
            peers,
            machine_name,
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.attachment_error = Some(error);
                return;
            }
        };
        self.prompt_history.reset();
        self.typing_guard.on_blur_or_send();
        self.paste_history = Default::default();
        self.draft.replace_text("");
        self.draft_images.clear();
        self.draft_context.clear();
        self.draft_reply_to = None;
        self.retained_context = Default::default();
        self.native.queue_held = false;
        self.native.access = resolve_compatible_access_mode(
            self.native.access,
            supported_access_modes(&self.harness),
        )
        .1;
        self.begin_prompt_with_fresh(prompt, fresh_context);
    }

    pub fn effective_start_from_origin(&self) -> bool {
        self.start_from_origin.unwrap_or(true)
    }

    pub fn worktree_bootstrapping(&self) -> bool {
        self.worktree_bootstrap.is_some()
    }

    pub fn reconcile_draft_context(&mut self) {
        let referenced = crate::composer_tokens::collect_inline_context_ids_from_edit(&self.draft);
        crate::composer_context::reconcile_context_references(
            &referenced,
            &mut self.draft_context,
            &mut self.draft_images,
            &mut self.retained_context,
        );
    }

    pub fn validate_prompt_length(&self, prompt: &Prompt) -> Option<String> {
        let expanded =
            crate::attachments::pi_message(prompt).unwrap_or_else(|_| prompt.text.clone());
        crate::composer::prompt_length_validation_message(&expanded)
    }

    pub fn update_submission_error(&mut self) {
        if self.submission_error.is_none() {
            return;
        }
        let expanded = crate::composer_tokens::prompt_for_with_context(
            &self.draft,
            &self.draft_images,
            &self.draft_context,
            Some(&self.machine),
            &str::to_owned,
        )
        .ok()
        .and_then(|p| crate::attachments::pi_message(&p).ok())
        .unwrap_or_else(|| crate::composer_rich::markdown(&self.draft));
        self.submission_error = crate::composer::prompt_length_validation_message(&expanded);
    }

    pub(crate) fn begin_prompt(&mut self, prompt: Prompt) {
        self.begin_prompt_with_fresh(prompt, false);
    }

    pub(crate) fn begin_prompt_with_fresh(&mut self, prompt: Prompt, fresh_context: bool) {
        let fresh_context = fresh_context || self.captain_return_context;
        self.apply_staged_handoff(&prompt);
        // A staged handoff already cleared the session state it replaced; the fresh
        // reset would wipe the destination's retained session.
        if fresh_context && self.inflight_handoff.is_none() {
            let held_queue = !self.native.queue.is_empty();
            let previous_held = self.native.queue_held;
            if held_queue {
                self.native.queue_held = true;
            }
            if self.pi.is_some() {
                self.disconnect();
            }
            if held_queue {
                self.native.queue_held = previous_held;
            }
            self.session = None;
            self.harness_context_sent = false;
            self.native.fork_source = None;
            self.native.fork_entry = None;
            self.native.previous_session = None;
            self.native.session_name = None;
            self.context.begin_model_change();
        }
        self.apply_staged_handoff(&prompt);
        if let Some(binding) = self.next_execution.take() {
            self.run_id = Some(binding.run_id.clone());
            self.execution = Some(binding);
        } else {
            self.run_id = Some(uuid::Uuid::new_v4().to_string());
        }
        self.resumable_run_id = None;
        self.continuation_of_run_id = None;
        self.prompt_suggestion = None;
        self.stop_control = StopControl::default();
        self.native.initial_user = true;
        self.queued_prompt = Some((prompt.clone(), self.messages.len()));
        self.retry_prompt = Some(prompt.clone());
        self.arm_task();
        if self.messages.is_empty() && !self.captain_home && !self.renamed {
            self.title = if prompt.text.is_empty() {
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
        }
        let message_id = uuid::Uuid::new_v4().to_string();
        if fresh_context {
            self.fresh_session_messages.push(message_id.clone());
        }
        self.messages.push(Message {
            id: message_id,
            role: "You",
            text: prompt.text.clone(),
            images: prompt.images.to_vec(),
            context: prompt.context.clone(),
        });
        self.attachment_error = None;
        self.status = Status::Running;
        self.updated = crate::projects::now();
        self.life.user_sent(lifecycle::now());
        self.started = Some(Instant::now());
        self.elapsed = None;
        self.assistant = None;
        self.ensure_work();
        self.error = None;
        self.phase = if self.provider_update_held {
            "Waiting for provider update"
        } else {
            "Working"
        }
        .into();
        self.last_activity = Instant::now();
        self.drive_session();
    }

    pub fn stop(&mut self) {
        if self.remote.is_some() && !self.run_control_supported {
            self.error = Some(crate::protocol::RUN_CONTROL_UPDATE_REQUIRED.into());
            return;
        }
        if self.stop_control.disabled() && self.stop_control.target() == self.run_id.as_deref() {
            return;
        }
        self.begin_stop();
        self.stop_serial = self.stop_serial.saturating_add(1);
        if self.forward("thread.stop", json!({"runId":self.run_id})) {
            return;
        }
        let _ = self.stop_task();
    }

    pub(super) fn stop_task(&mut self) -> Result<(), String> {
        if self.status.busy()
            && !self.initializing
            && self.queued_prompt.is_none()
            && self.pending_prompt.is_none()
            && !self.has_pending_sends()
        {
            self.mark_stopped();
        }
        self.memory_staged.clear();
        self.memory_context.clear();
        self.scope_context_pending = true;
        self.native.queue_held = true;
        self.pending_injections.clear();
        self.injection_id = None;
        self.injection_retry_at = None;
        self.operation_deadline = None;
        if self.initializing
            || self.queued_prompt.is_some()
            || self.native.operating() && !matches!(self.harness.as_str(), "codex" | "claude")
            || self.has_pending_sends()
            || self.context.phase == crate::context::ContextPhase::Compacting
                && !matches!(self.harness.as_str(), "codex" | "claude")
        {
            self.disconnect();
            self.complete_task("cancelled");
            return Ok(());
        }
        if self.status == Status::Running || self.status == Status::Waiting {
            // Release a blocking extension dialog before asking Pi to wait for idle.
            self.cancel_native_dialogs();
            if !self.command("clear_queue", json!({})) || !self.command("abort", json!({})) {
                let error = self
                    .error
                    .clone()
                    .unwrap_or_else(|| "Pi could not stop the task.".into());
                self.disconnect();
                self.complete_task("cancelled");
                self.error = Some(error.clone());
                return Err(error);
            }
            self.phase = "Stopping".into();
        } else {
            self.complete_task("cancelled");
        }
        Ok(())
    }

    pub fn respond(&mut self, mut response: Value) {
        if self.responding_request.is_some() {
            return;
        }
        if self.native.questionnaire.is_some() && !self.question_contract_supported {
            self.error = Some(crate::protocol::QUESTIONS_UPDATE_REQUIRED.into());
            return;
        }
        if self
            .native
            .questionnaire
            .as_ref()
            .is_some_and(|form| !form.can_respond())
        {
            self.error = Some("This request is not resumable.".into());
            return;
        }
        if let Some(request) = &self.request {
            if !crate::approval::can_respond(request) {
                self.error = Some("This request is not resumable.".into());
                return;
            }
            if request["method"] == "confirm" {
                if let Err(error) = crate::approval::validate(request, &response) {
                    self.error = Some(error);
                    return;
                }
                let decision = crate::approval::decision(&response)
                    .expect("validated")
                    .to_owned();
                let feedback = response.get("feedback").cloned();
                response = crate::approval::response(&decision);
                if let Some(feedback) = feedback {
                    response["feedback"] = feedback;
                }
                if self.remote.is_none() && self.harness == "pi" && decision == "acceptAlways" {
                    response = json!({"cancelled":true});
                }
            }
        }
        if self.remote.is_some() {
            if !self.requests_supported {
                self.error = Some(crate::protocol::REQUESTS_UPDATE_REQUIRED.into());
                return;
            }
            let request = self.request.clone().unwrap_or(Value::Null);
            let params = json!({"threadId":self.id,"requestId":request["id"],"generation":request["runtimeGeneration"],
                "requestContract":true,"questionContract":self.native.questionnaire.is_some(),
                "answers":self.native.questionnaire.as_ref().map(|form| json!(form.answers)),"response":response});
            match self
                .remote
                .as_ref()
                .expect("remote")
                .send("request.respond", params)
            {
                Ok(id) => self.responding_request = Some(id),
                Err(error) => self.error = Some(error),
            }
            return;
        }
        if self.respond_message_question(&response) {
            return;
        }
        if response["cancelled"] == true {
            self.cancel_questionnaire();
        }
        let Some(request) = &self.request else {
            return;
        };
        if request["method"] == "confirm" {
            let request = request.clone();
            match self.write_approval_response(&request, response) {
                Ok(()) => {
                    self.request = None;
                    self.forwarded_question = None;
                    self.native_dialog_done();
                }
                Err(error) => self.error = Some(error),
            }
            return;
        }
        if response.get("feedback").is_some() {
            self.error = Some(
                "Feedback requires a live approval that supports it and a decline decision.".into(),
            );
            return;
        }
        response["type"] = "extension_ui_response".into();
        response["id"] = request["id"].clone();
        if self.harness == "pi"
            && let Some(fields) = response.as_object_mut()
        {
            // Pi's extension transport consumes only its boolean reply, not our contract.
            fields.remove("decision");
            fields.remove("allowForSession");
        }
        let result = self
            .pi
            .as_ref()
            .ok_or_else(|| "The session is unavailable. Retry to resume it.".to_owned())
            .and_then(|pi| crate::harness::send(pi, response));
        match result {
            Ok(()) => {
                self.request = None;
                self.forwarded_question = None;
                self.native_dialog_done();
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// KSG's pick rule: stage a pending handoff, retarget the staged one, or disarm it
    /// when the user picks the thread's current harness again.
    pub fn stage_handoff(&mut self, instance: String, model: String) {
        if let Some(index) = self.staged_handoff {
            if instance == self.harness {
                self.handoffs.remove(index);
                self.staged_handoff = None;
            } else {
                let handoff = &mut self.handoffs[index];
                handoff.to_instance = instance;
                handoff.to_model = Some(model);
                handoff.updated_at = lifecycle::now();
            }
            return;
        }
        if instance == self.harness {
            return;
        }
        // Re-arm a stale pending record for the same destination instead of a duplicate.
        if let Some(index) = self.handoffs.iter().position(|handoff| {
            handoff.status == crate::handoff::HandoffStatus::Pending
                && handoff.to_instance == instance
        }) {
            let handoff = &mut self.handoffs[index];
            handoff.to_model = Some(model);
            handoff.updated_at = lifecycle::now();
            handoff.message_index = self.messages.len();
            self.staged_handoff = Some(index);
            return;
        }
        let from_model = self
            .selected_model
            .as_ref()
            .map(|(_, model)| model.clone())
            .or_else(|| (!self.model_id.is_empty()).then(|| self.model_id.clone()));
        self.handoffs.push(crate::handoff::Handoff::new(
            self.harness.clone(),
            from_model,
            instance,
            Some(model),
            self.messages.len(),
            lifecycle::now(),
        ));
        self.staged_handoff = Some(self.handoffs.len() - 1);
    }

    /// The staged switch's destination, if any.
    #[allow(dead_code)] // Read by UI tests and the picker's pending-row checks.
    pub fn staged_handoff(&self) -> Option<&crate::handoff::Handoff> {
        self.staged_handoff
            .and_then(|index| self.handoffs.get(index))
    }

    /// Why a provider switch cannot stage right now. KSG blocks the change while a run
    /// is active or an approval is pending.
    pub fn handoff_block_reason(&self) -> Option<String> {
        if self.status.busy() {
            return Some("Wait for the current run to finish before switching providers.".into());
        }
        if self.request.is_some()
            || self.approval_send.is_some()
            || !self.pending_approvals().is_empty()
        {
            return Some("Answer the pending request before switching providers.".into());
        }
        None
    }

    /// The transcript as KSG `HistoricalMessage`s, in display order: each message, then
    /// the tools anchored to it. Reasoning stays out, like KSG's `historicalMessage`.
    pub fn handoff_items(&self) -> Vec<crate::handoff::HistoricalItem> {
        use crate::handoff::HistoricalItem;
        // Items between handoff k-1's and handoff k's coverage end ran under record k's
        // `from` harness; everything after the last record ran under the current one.
        let spans: Vec<(usize, &str)> = self
            .handoffs
            .iter()
            .filter_map(|handoff| {
                handoff
                    .from_instances
                    .first()
                    .map(|from| (handoff.covered.to as usize, from.as_str()))
            })
            .collect();
        let provider_for = |index: usize| -> Option<String> {
            Some(
                spans
                    .iter()
                    .find(|(end, _)| index < *end)
                    .map(|(_, from)| (*from).to_owned())
                    .unwrap_or_else(|| self.harness.clone()),
            )
        };
        let tool_item = |tool: &Tool, run: usize, index: usize| -> Option<HistoricalItem> {
            let command = format!(
                "Command: {}\nExit code: {}\n{}",
                if tool.detail.is_empty() {
                    &tool.name
                } else {
                    &tool.detail
                },
                tool.exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".into()),
                tool.output
            );
            let (kind, text) = if !tool.changes.is_empty() && tool.output.is_empty() {
                (
                    "file_change",
                    format!(
                        "File change: {}",
                        tool.changes
                            .iter()
                            .map(|change| change.path.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )
            } else if tool.failed && tool.exit_code.is_none() && !tool.output.is_empty() {
                ("error", tool.output.clone())
            } else if !tool.detail.is_empty() || !tool.output.is_empty() || tool.exit_code.is_some()
            {
                ("command_execution", command)
            } else {
                return None;
            };
            Some(HistoricalItem {
                role: "assistant",
                kind,
                text,
                item_id: tool.id.clone(),
                run: format!("{run}"),
                provider_thread: provider_for(index),
                status: if tool.failed {
                    "failed"
                } else if tool.done {
                    "completed"
                } else {
                    "running"
                },
            })
        };
        let mut items = Vec::new();
        let mut turn = 0usize;
        for (index, message) in self.messages.iter().enumerate() {
            let (role, kind) = match message.role {
                "You" => {
                    turn += 1;
                    ("user", "user_message")
                }
                "Shina" => ("user", "user_message"),
                _ => ("assistant", "assistant_message"),
            };
            items.push(HistoricalItem {
                role,
                kind,
                text: if message.role == "Shina" {
                    message.display_text().to_owned()
                } else {
                    message.text.clone()
                },
                item_id: message.id.clone(),
                run: format!("{}", turn.max(1)),
                provider_thread: provider_for(items.len()),
                status: "completed",
            });
            let mut anchored: Vec<&Tool> = self
                .tools
                .iter()
                .filter(|tool| tool.anchor.message == index)
                .collect();
            anchored.sort_by_key(|tool| (tool.anchor.offset, tool.order));
            let position = items.len();
            items.extend(
                anchored
                    .iter()
                    .enumerate()
                    .filter_map(|(offset, tool)| tool_item(tool, turn.max(1), position + offset)),
            );
        }
        let position = items.len();
        let mut trailing: Vec<&Tool> = self
            .tools
            .iter()
            .filter(|tool| tool.anchor.message >= self.messages.len())
            .collect();
        trailing.sort_by_key(|tool| (tool.anchor.message, tool.anchor.offset, tool.order));
        items.extend(
            trailing
                .iter()
                .enumerate()
                .filter_map(|(offset, tool)| tool_item(tool, turn.max(1), position + offset)),
        );
        items
    }

    /// How much of the item sequence `instance` has seen: natively before it was
    /// switched away from, or through a delivered handoff.
    pub fn handoff_seen_through(&self, instance: &str) -> usize {
        self.handoffs
            .iter()
            .filter(|handoff| {
                handoff
                    .from_instances
                    .first()
                    .is_some_and(|from| from == instance)
                    || (handoff.to_instance == instance
                        && handoff.status == crate::handoff::HandoffStatus::Ready)
            })
            .map(|handoff| handoff.covered.to as usize)
            .max()
            .unwrap_or(0)
    }

    /// The switch at send time: retain the source session id, point the thread at the
    /// destination (its retained session if it has one), and stage the package on the
    /// thread for `drive_session` to prepend to this turn.
    pub(super) fn apply_staged_handoff(&mut self, prompt: &Prompt) {
        let Some(index) = self.staged_handoff.take() else {
            return;
        };
        let Some(handoff) = self.handoffs.get(index) else {
            return;
        };
        if handoff.status != crate::handoff::HandoffStatus::Pending
            || handoff.to_instance == self.harness
        {
            return;
        }
        let to_instance = handoff.to_instance.clone();
        let to_model = handoff.to_model.clone();
        // disconnect() calls revert_handoff(), so the source state is stashed after it.
        if self.pi.is_some() {
            self.disconnect();
        }
        self.handoff_revert = Some(HandoffRevert {
            harness: self.harness.clone(),
            session: self.session.clone(),
            selected_model: self.selected_model.clone(),
            model_provider: self.model_provider.clone(),
            model_id: self.model_id.clone(),
            model: self.model.clone(),
        });
        if let Some(session) = self.session.take() {
            self.handoff_sessions.insert(self.harness.clone(), session);
        }
        // History items carry the provider that produced them; compute before the switch.
        let items = self.handoff_items();
        let seen = self.handoff_seen_through(&to_instance);
        self.session = self.handoff_sessions.get(&to_instance).cloned();
        self.harness = to_instance.clone();
        if let Some(model) = to_model {
            self.selected_model = Some((to_instance.clone(), model.clone()));
            self.model_provider = to_instance.clone();
            self.model_id = model.clone();
            self.model = model;
        }
        self.harness_context_sent = false;
        self.native.fork_source = None;
        self.native.fork_entry = None;
        self.native.previous_session = None;
        self.native.session_name = None;
        self.context.begin_model_change();
        let budget = crate::handoff::handoff_budget(&crate::handoff::BudgetInput {
            token_cap: crate::handoff::configured_token_cap(),
            user_text: &prompt.text,
            images: prompt.images.iter().filter(|a| a.is_image()).count(),
            files: prompt.images.iter().filter(|a| !a.is_image()).count(),
            native_used: 0,
            max_tokens: self.context_window,
            model_context_window: self.context_window,
        });
        let package = crate::handoff::build_package(&format!("{}", self.id), &items, seen, budget);
        let handoff = &mut self.handoffs[index];
        handoff.covered = package.covered;
        handoff.strategy = package.strategy;
        handoff.summary = package.context.clone();
        handoff.delivery.delivered_item_ids = package.delivered_item_ids;
        handoff.delivery.omitted_item_ids = package.omitted_item_ids;
        handoff.updated_at = lifecycle::now();
        self.handoff_package = Some(package.text);
        self.inflight_handoff = Some(index);
    }

    /// KSG marks the record delivered once the provider accepts the turn.
    pub(super) fn mark_handoff_delivered(&mut self) {
        let Some(index) = self.inflight_handoff.take() else {
            return;
        };
        if let Some(handoff) = self.handoffs.get_mut(index) {
            handoff.status = crate::handoff::HandoffStatus::Ready;
            handoff.delivery.status = crate::handoff::DeliveryStatus::Inline;
            handoff.updated_at = lifecycle::now();
        }
        self.handoff_package = None;
        self.handoff_revert = None;
    }

    /// Back to the source harness and model, with its retained session restored; the
    /// record stays pending for a later explicit retry. Returns true when it reverted.
    pub(super) fn revert_handoff(&mut self) -> bool {
        if self.inflight_handoff.take().is_none() && self.handoff_revert.is_none() {
            return false;
        }
        if let Some(revert) = self.handoff_revert.take() {
            self.harness = revert.harness;
            self.session = revert.session;
            self.selected_model = revert.selected_model;
            self.model_provider = revert.model_provider;
            self.model_id = revert.model_id;
            self.model = revert.model;
        }
        self.handoff_package = None;
        true
    }

    pub fn disconnect(&mut self) {
        self.revert_handoff();
        self.memory_pending.clear();
        self.learned_turn = None;
        self.memory_staged.clear();
        self.native_disconnect();
        self.finish_thoughts();
        if let Some((prompt, index)) = self.queued_prompt.take() {
            self.remove_optimistic_prompt(&prompt, index);
            self.restore_prompt(&prompt);
        }
        if let Some((_, prompt, index)) = self.pending_prompt.take() {
            self.remove_optimistic_prompt(&prompt, index);
            self.restore_prompt(&prompt);
        }
        self.pi = None;
        self.task_result_history = None;
        self.status = Status::Ready;
        self.assistant = None;
        if self.native.questionnaire.is_none() {
            self.request = None;
        }
        self.forwarded_question = None;
        self.initializing = false;
        self.session_started = None;
        self.configuring = false;
        self.pending_thinking = None;
        self.injection_id = None;
        self.injection_retry_at = None;
        self.operation_deadline = None;
        self.context.cancel_request();
        self.started = None;
        self.phase.clear();
    }

    pub(super) fn arm_task(&mut self) {
        self.arm_crew_report();
        if self.delegated {
            self.task_result = None;
            self.completion_notified = false;
        }
    }

    pub(super) fn complete_task(&mut self, result: &'static str) {
        self.complete_crew_report(result);
        if self.delegated && !self.completion_notified && self.task_result.is_none() {
            self.task_result = Some(result);
        }
    }

    pub(super) fn enqueue_injection(&mut self, text: String) -> String {
        self.serial += 1;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let delivery = format!("{}-{}-{stamp}", self.id, self.serial);
        self.pending_injections.push_back(Injection {
            text: format!("{INJECTION_PREFIX}[delivery {delivery}]\n{text}"),
            shown: false,
            steer: None,
            peer_input: None,
        });
        delivery
    }

    pub(crate) fn pending_local_attempt(&self) -> Option<&str> {
        if self.delegated && self.peer_origin.is_none() {
            self.next_execution.as_ref()?.attempt_id.as_deref()
        } else {
            None
        }
    }

    pub(super) fn flush_injection(&mut self) -> bool {
        if self.offline_blocked
            || self.provider_update_held
            || self.admission_blocked
            || self.status == Status::Paused
        {
            return false;
        }
        #[cfg(debug_assertions)]
        if test_hold_injection_flush() {
            return false;
        }
        if self
            .injection_retry_at
            .is_some_and(|at| Instant::now() >= at)
        {
            self.injection_retry_at = None;
        }
        if self.pending_injections.is_empty()
            || self.pi.is_none()
            || self.initializing
            || self.queued_prompt.is_some()
            || self.injection_id.is_some()
            || self.configuring
            || self.context.phase == crate::context::ContextPhase::Compacting
            || self
                .injection_retry_at
                .is_some_and(|at| Instant::now() < at)
            || self.phase == "Stopping"
            || !matches!(
                self.status,
                Status::Ready | Status::Completed | Status::Running | Status::Waiting
            )
        {
            return false;
        }
        if self.pending_injections[0].peer_input.is_some()
            && (!self.history_loaded || self.peer_recovery_held || test_hold_peer_input_flush())
        {
            // Recovery needs a completed stable-history read before an input
            // that may have crossed the crash boundary can be retried.
            return false;
        }
        if self.harness == "pi" && self.model_provider != "cpa" {
            self.error =
                Some("Choose a CPA model before Shina can deliver queued messages.".into());
            return false;
        }
        let prepared_local_attempt = self.pending_local_attempt().is_some();
        if let Some(attempt) = self.pending_local_attempt() {
            // Startup can report Running before an admitted brief has started.
            // Never send this brief into a different turn that actually began.
            if self.status.busy()
                && self.started.is_some()
                && self.run_id.is_some()
                && self.provider_terminal_run != self.run_id
            {
                return false;
            }
            let Some(index) = self.pending_injections.iter().position(|input| {
                input.peer_input.is_none() && tasks::delivery_id(&input.text) == Some(attempt)
            }) else {
                return false;
            };
            // Other messages (including reports from this worker's own crew)
            // must not take the run identity reserved for the new brief.
            if index > 0 {
                let brief = self
                    .pending_injections
                    .remove(index)
                    .expect("located brief");
                self.pending_injections.push_front(brief);
            }
        }
        let text = self.pending_injections[0].text.clone();
        let peer_input = self.pending_injections[0].peer_input;
        let peer_execution = peer_input
            .as_ref()
            .and_then(|id| self.peer_input_executions.get(&id.to_string()))
            .cloned();
        if let Some(binding) = &peer_execution
            && self.status.busy()
            && self.run_id.is_some()
            && self.provider_terminal_run != self.run_id
            && self.execution.as_ref() != Some(binding)
            && self
                .peer_origin
                .as_ref()
                .and_then(|origin| origin.attempt_id.as_ref())
                != binding.attempt_id.as_ref()
        {
            // A different attempt cannot enter the current provider queue.
            // Same-attempt inputs retain their own acceptance attribution.
            return false;
        }
        // A steer injection pins the run it was declared against. If that run
        // already ended, the message becomes ordinary follow-up input so it
        // queues behind the newer work rather than vanishing.
        let steer = self.pending_injections[0]
            .steer
            .as_ref()
            .filter(|run| self.status.busy() && Some(*run) == self.run_id.as_ref())
            .is_some();
        if prepared_local_attempt
            || !self.status.busy()
            || self.run_id.is_none()
            || peer_execution.is_some() && self.provider_terminal_run == self.run_id
        {
            if let Some(binding) = peer_execution.clone().or_else(|| {
                self.next_execution.clone().filter(|binding| {
                    !self
                        .peer_input_executions
                        .values()
                        .any(|peer| peer == binding)
                })
            }) {
                if self.next_execution.as_ref() == Some(&binding) {
                    self.next_execution = None;
                }
                self.run_id = Some(binding.run_id.clone());
                if let Some(origin) = &mut self.peer_origin {
                    origin.attempt_id = binding.attempt_id.clone();
                }
                self.execution = Some(binding);
            } else {
                self.run_id = Some(uuid::Uuid::new_v4().to_string());
            }
            if let Some(origin) = &mut self.peer_origin {
                // A preserved follow-up starts a distinct turn. A prior stop
                // or completion result must not mask this turn's result.
                origin.result = None;
                origin.acknowledged = false;
                origin.attempt_message = text.clone();
                origin.reply_after_position = self.messages.len();
            }
            self.prompt_suggestion = None;
            self.arm_task();
        }
        // prompt starts an idle Pi; followUp also covers a run starting before Pi reads it.
        let behavior = if steer { "steer" } else { "followUp" };
        let input_run = peer_execution
            .as_ref()
            .map(|binding| binding.run_id.clone())
            .or_else(|| self.run_id.clone());
        if !self.command(
            "prompt",
            json!({"message":text,"streamingBehavior":behavior,"runId":input_run}),
        ) {
            return false;
        }
        let prompt_command = format!("{}-{}", self.id, self.serial);
        self.injection_id = Some(prompt_command.clone());
        if let Some(peer) = peer_input {
            if let Some(binding) = peer_execution
                && self.execution.as_ref() != Some(&binding)
            {
                self.peer_provider_inputs.insert(peer.to_string(), binding);
            }
            self.peer_prompt = Some((prompt_command, peer));
            self.peer_dispatches
                .push((peer.to_string(), behavior.into(), input_run.clone()));
            if let Some(origin) = &mut self.peer_origin
                && let Some(evidence) = origin
                    .inputs
                    .iter_mut()
                    .find(|evidence| evidence.command_id == peer.to_string())
            {
                evidence.realized_delivery = Some(
                    if behavior == "steer" {
                        "steer"
                    } else {
                        "follow_up"
                    }
                    .into(),
                );
                evidence.run_id = input_run;
                evidence.observed_at = lifecycle::now();
            }
        }
        if let Some(delivery) = tasks::delivery_id(&text) {
            self.injection_dispatches
                .push((delivery.to_owned(), behavior.to_owned()));
        }
        let at = lifecycle::now();
        if self.life.settled() {
            self.life.unsettle(at, false);
        }
        self.life.last_requested_at = Some(at);
        if !self.pending_injections[0].shown {
            self.messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "Shina",
                text,
                images: Vec::new(),
                context: None,
            });
            self.pending_injections[0].shown = true;
        }
        if !self.status.busy() {
            self.arm_task();
            self.status = Status::Running;
            self.started = Some(Instant::now());
            self.elapsed = None;
            self.assistant = None;
            self.error = None;
            self.phase = if self.harness == "pi" {
                "Waiting for Pi"
            } else {
                "Waiting for ACP"
            }
            .into();
            self.ensure_work();
        }
        true
    }

    pub(super) fn diagnostic(&mut self, text: &str) {
        const LIMIT: usize = 128;
        if self.diagnostics.len() >= LIMIT {
            self.diagnostics.drain(..self.diagnostics.len() + 1 - LIMIT);
        }
        self.diagnostics
            .push(crate::update::redact(&crate::projects::redact(&truncate(
                text, 2048,
            ))));
    }

    pub(super) fn check_deadlines(&mut self, now: Instant) -> bool {
        if self.request.is_some() {
            return false;
        }
        let Some((operation, deadline)) = self.operation_deadline else {
            return false;
        };
        if now < deadline {
            return false;
        }
        if operation == "abort" {
            self.operation_deadline = None;
            self.tick_stop(now);
        } else {
            self.fail_session(format!(
                "Pi did not answer the {operation} operation before its deadline. Retry to resume."
            ));
        }
        true
    }

    pub(super) fn remove_optimistic_prompt(&mut self, prompt: &Prompt, index: usize) {
        if self.messages.get(index).is_some_and(|message| {
            message.role == "You"
                && message.text == prompt.text
                && message.images.as_slice() == prompt.images.as_ref()
        }) {
            self.messages.remove(index);
        }
    }
    pub(super) fn reject_images(&mut self, prompt: &Prompt, index: usize, error: String) {
        self.restore_prompt(prompt);
        self.remove_optimistic_prompt(prompt, index);
        self.attachment_error = Some(error);
        self.status = Status::Ready;
        self.started = None;
        self.elapsed = None;
        self.assistant = None;
        self.phase.clear();
    }

    pub(super) fn fail_session(&mut self, error: String) {
        self.finish_retry(Some(&error));
        self.memory_staged.clear();
        self.memory_context.clear();
        self.scope_context_pending = true;
        self.native.queue_held = true;
        let handoff_delivery = self.inflight_handoff.is_some() || self.handoff_revert.is_some();
        self.disconnect();
        self.error = Some(error.clone());
        if handoff_delivery {
            self.submission_error = Some(error);
        }
        self.status = Status::Failed;
        self.life.failed_at = Some(lifecycle::now());
        self.complete_task("failed");
    }

    /// Settings and history are transport details, never conversation states.
    pub(super) fn drive_session(&mut self) {
        // Boot recovery only reads history. poll_tasks disconnects this session
        // in the same tick, before the outbox can send any captured commands.
        if self.recovering_task
            || self.pi.is_none()
            || self.configuring
            || matches!(self.status, Status::Failed | Status::Paused)
            || self.offline_blocked
            || self.provider_update_held
            || self.admission_blocked
        {
            return;
        }
        if self.initializing {
            if !(self.state_loaded && self.history_loaded) {
                return;
            }
            if !self.startup_model_applied {
                self.startup_model_applied = true;
                if let Some((provider, id)) = self.selected_model.clone() {
                    self.configuring =
                        self.command("set_model", json!({"provider":provider,"modelId":id}));
                    if !self.configuring {
                        self.fail_session(self.error.clone().unwrap());
                    }
                    return;
                }
            }
            if !self.startup_thinking_applied {
                self.startup_thinking_applied = true;
                if let Some(level) = self.selected_thinking.clone() {
                    self.pending_thinking = Some(level.clone());
                    self.configuring = self.command("set_thinking_level", json!({"level":level}));
                    if !self.configuring {
                        self.fail_session(self.error.clone().unwrap());
                    }
                    return;
                }
            }
            // Drafts can pick Cursor's context and other native tiers before an
            // ACP session exists. Validate each saved option against the live
            // session's descriptors after the model handshake, prune stale ones,
            // and wait for each supported change to be accepted.
            if self.harness == "cursor" {
                let live_descriptors = self.live_option_descriptors.as_deref().or_else(|| {
                    self.models
                        .iter()
                        .find(|model| model.id == self.model_id)
                        .map(|model| model.capabilities.option_descriptors.as_slice())
                });
                while self.startup_cursor_option < self.model_options.len() {
                    let option = self.model_options[self.startup_cursor_option].clone();
                    if let Some(descriptors) = live_descriptors
                        && !crate::cursor::option_supported_by_descriptors(
                            descriptors,
                            &option.id,
                            &option.value,
                        )
                    {
                        self.model_options.remove(self.startup_cursor_option);
                        continue;
                    }
                    let data = json!({"optionId":option.id,"value":option.value});
                    self.pending_cursor_option = Some(option.id);
                    self.startup_cursor_option += 1;
                    self.configuring = self.command("set_option", data);
                    if !self.configuring {
                        self.pending_cursor_option = None;
                        self.fail_session(self.error.clone().unwrap());
                    }
                    return;
                }
            }
            // Pi acknowledges set_thinking_level without returning the applied level.
            // Verify its state before delivering a delegated brief, since Pi can clamp it.
            if self.delegated && self.selected_model.is_some() && !self.startup_selection_verified {
                if self.startup_selection_check.is_none() {
                    if self.command("get_state", json!({})) {
                        self.startup_selection_check = Some(format!("{}-{}", self.id, self.serial));
                    } else {
                        self.fail_session(
                            self.error.clone().unwrap_or_else(|| {
                                "Pi did not accept selection verification.".into()
                            }),
                        );
                    }
                }
                return;
            }
            self.initializing = false;
            self.session_started = None;
            if self.queued_prompt.is_none()
                && self.started.is_none()
                && self.status == Status::Running
            {
                self.status = Status::Ready;
                self.phase.clear();
            }
        }
        if self.drive_native() {
            return;
        }
        // Pi has answered its session handshake, so it can report the context window it
        // resumed. A directly wired transport in a test has not answered that handshake.
        if !self.context_startup_requested && self.state_loaded && self.history_loaded {
            self.context_startup_requested = true;
            self.refresh_context_usage();
        }
        let Some((prompt, index)) = self.queued_prompt.take() else {
            return;
        };
        if self.harness == "pi" && self.model_provider != "cpa" {
            self.queued_prompt = Some((prompt, index));
            self.fail_session("Choose a CPA model before sending. Shina will not send prompts to a direct provider.".into());
            return;
        }
        if prompt.images.iter().any(Attachment::is_image) && self.supports_images != Some(true) {
            let error = self.image_model_error();
            self.reject_images(&prompt, index, error);
            return;
        }
        let mut prompt = prompt;
        if self.harness == "claude" {
            let effort = self
                .model_options
                .iter()
                .find(|o| o.id == "effort")
                .and_then(|o| o.value.as_str());
            let caps = self
                .models
                .iter()
                .find(|m| m.provider == self.model_provider && m.id == self.model_id)
                .map(|m| m.capabilities.clone())
                .unwrap_or_default();
            let injected = crate::descriptors::resolve_prompt_injected_effort(&caps, effort);
            prompt.text = crate::descriptors::apply_claude_prompt_effort_prefix(
                &prompt.text,
                injected.as_deref().or(effort),
            );
        }
        if self.harness == "pi"
            && let Some(tier) = self
                .model_options
                .iter()
                .find(|o| o.id == "serviceTier")
                .and_then(|o| o.value.as_str())
                .filter(|&t| t != "default" && !t.is_empty())
        {
            let _ = self.command(
                "prompt",
                json!({"message": format!("/shina-service-tier {tier}")}),
            );
        }
        let mut record = match attachments::prompt_record(&prompt, self.harness == "pi") {
            Ok(record) => record,
            Err(error) => {
                self.reject_images(&prompt, index, error);
                return;
            }
        };
        if self.inflight_handoff.is_some()
            && let Some(package) = &self.handoff_package
        {
            let user_text = record["message"].as_str().unwrap_or_default().to_owned();
            record["message"] = json!(crate::handoff::with_inline_context(package, &user_text));
        }
        if self.captain_return_context {
            let recent = crate::handoff::bounded_transcript_context(
                &self.messages[..index.min(self.messages.len())],
            );
            let user_text = record["message"].as_str().unwrap_or_default();
            record["message"] = json!(crate::handoff::with_inline_context(
                &format!(
                    "Recent conversation after a linked Captain turn, for context only:\n{recent}"
                ),
                user_text,
            ));
        }
        if !self.captain_context.is_empty() {
            let user_text = record["message"].as_str().unwrap_or_default().to_owned();
            record["message"] = json!(format!("{}{user_text}", self.captain_context));
        }
        if self.command("prompt", record) {
            self.pending_prompt = Some((format!("{}-{}", self.id, self.serial), prompt, index));
        } else {
            self.queued_prompt = Some((prompt, index));
            self.fail_session(
                self.error
                    .clone()
                    .unwrap_or_else(|| "Pi did not accept the message. Retry to resume.".into()),
            );
        }
    }

    /// Reads the selected model's levels from `models`. A model with no row there
    /// keeps the levels its session or server last reported.
    pub(super) fn refresh_thinking_levels(&mut self) {
        if let Some(model) = self
            .models
            .iter()
            .find(|m| m.provider == self.model_provider && m.id == self.model_id)
        {
            self.thinking_levels = model.reasoning_levels();
        }
    }

    /// A draft that moves to another folder or machine drops what the old
    /// workspace's catalog told it: model rows, reasoning levels and commands.
    /// The next catalog pass fills them in for the new workspace. Pi's CPA rows
    /// belong to the machine, so they survive a folder change on it.
    pub(crate) fn forget_workspace_catalog(&mut self, same_machine: bool) {
        self.cursor_catalog_models = None;
        self.native.commands.clear();
        self.native.skills.clear();
        self.native.commands_loaded = false;
        self.thinking_levels.clear();
        if same_machine && self.harness == "pi" {
            self.refresh_thinking_levels();
        } else {
            self.models.clear();
        }
    }

    pub fn restoring_history(&self) -> bool {
        self.initializing
            && !self.history_loaded
            && (self.session.is_some() && !self.never_sent || self.native.fork_source.is_some())
    }

    pub(super) fn reclaim_idle(&mut self, now: Instant) -> bool {
        if self.pi.is_some()
            && !self.initializing
            && !self.configuring
            && !self.status.busy()
            && self.pending_injections.is_empty()
            && self.injection_id.is_none()
            && now.duration_since(self.last_activity) >= IDLE_TIMEOUT
        {
            let status = self.status;
            self.disconnect();
            self.status = status;
            return true;
        }
        false
    }

    pub(super) fn assistant(&mut self) -> &mut Message {
        self.ensure_work();
        let index = *self.assistant.get_or_insert_with(|| {
            self.messages.push(Message {
                id: uuid::Uuid::new_v4().to_string(),
                role: "Pi",
                text: String::new(),
                images: Vec::new(),
                context: None,
            });
            self.messages.len() - 1
        });
        &mut self.messages[index]
    }

    pub(super) fn ensure_work(&mut self) -> &mut AssistantWork {
        let start = self
            .messages
            .iter()
            .rposition(|message| message.role != "Pi")
            .map_or(0, |index| index + 1);
        if self.work.last().is_none_or(|work| work.start != start) {
            // A finished turn folds its steps into one "Worked for" line, as Cursor does.
            self.work.push(AssistantWork {
                start,
                stopped: false,
                stopped_order: 0,
                expanded: false,
                elapsed: None,
                outcome: None,
            });
        }
        self.work.last_mut().unwrap()
    }

    pub(super) fn thought(&mut self, block: usize) -> &mut Thought {
        self.assistant();
        let message = self.assistant.unwrap();
        let order = self.next_work_order();
        let index = self
            .thoughts
            .iter()
            .position(|thought| thought.anchor.message == message && thought.block == block)
            .unwrap_or_else(|| {
                self.thoughts.push(Thought {
                    anchor: ToolAnchor {
                        message,
                        offset: self.messages[message].text.len(),
                    },
                    block,
                    order,
                    text: String::new(),
                    expanded: false,
                    done: false,
                    started: None,
                    duration_ms: None,
                });
                self.thoughts.len() - 1
            });
        &mut self.thoughts[index]
    }

    /// Reuse Pi's content-block order for both restored history and completed live messages.
    /// Keep Thread.tools flat for existing consumers and record its transcript positions.
    pub(super) fn anchor_content_work(&mut self, content: &Value, message: usize, restored: bool) {
        let Some(blocks) = content.as_array() else {
            return;
        };
        let mut offset = 0;
        let mut texts = 0;
        for (block_index, block) in blocks.iter().enumerate() {
            if block["type"] == "text" {
                if let Some(text) = block["text"].as_str() {
                    offset += text.len() + usize::from(texts > 0);
                    texts += 1;
                }
            } else if block["type"] == "toolCall" {
                let anchor = ToolAnchor { message, offset };
                let id = string(block, "id");
                if !restored
                    && let Some(tool) = self
                        .tools
                        .iter_mut()
                        .rev()
                        .find(|tool| tool.id == id && tool.anchor.message == message)
                {
                    tool.anchor = anchor;
                    tool.block = block_index;
                } else {
                    let mut tool =
                        Tool::new(id, string(block, "name"), &block["arguments"], anchor);
                    tool.accept_metadata(block);
                    tool.order = self.next_work_order();
                    tool.block = block_index;
                    tool.parent_call = block["parentToolCallId"].as_str().map(str::to_owned);
                    if restored {
                        tool.done = true;
                        tool.failed = true;
                        tool.output = "No saved result for this tool call.".into();
                    }
                    tool.seed_subagents();
                    self.agent_index.observe(self.tools.len(), &tool);
                    self.tools.push(tool);
                }
            } else if block["type"] == "thinking" {
                let order = self.next_work_order();
                let index = self
                    .thoughts
                    .iter()
                    .position(|thought| {
                        thought.anchor.message == message && thought.block == block_index
                    })
                    .unwrap_or_else(|| {
                        self.thoughts.push(Thought {
                            anchor: ToolAnchor { message, offset },
                            block: block_index,
                            order,
                            text: String::new(),
                            expanded: false,
                            done: true,
                            started: None,
                            duration_ms: None,
                        });
                        self.thoughts.len() - 1
                    });
                let thought = &mut self.thoughts[index];
                thought.anchor = ToolAnchor { message, offset };
                thought.text = string(block, "thinking").into();
                thought.done = true;
                thought.duration_ms = thought.duration_ms.or_else(|| {
                    thought
                        .started
                        .map(|start| start.elapsed().as_millis() as u64)
                });
            }
        }
    }
}

impl Thread {
    pub(super) fn stored_metadata(&self) -> Value {
        let mut record = json!({
            "id":self.id, "title":self.title, "renamed":self.renamed, "session":self.session, "lifecycle":self.life,
            "harness":self.harness, "harnessContextSent":self.harness_context_sent,
            "agentCount":crate::agents::total_count(self),"activeAgentCount":crate::agents::active_count(self),
            "runId":self.run_id,"resumableRunId":self.resumable_run_id,"continuationOfRunId":self.continuation_of_run_id,
            "execution":self.execution,"nextExecution":self.next_execution,
            "peerInputExecutions":self.peer_input_executions,
            "peerProviderInputs":self.peer_provider_inputs,
            "providerAcceptedRun":self.provider_accepted_run,
            "providerRunningRun":self.provider_running_run,
            "providerTerminalRun":self.provider_terminal_run,
            "providerTerminalStatus":self.provider_terminal_status,
            "memoryContext":self.memory_context, "scopeContextPending":self.scope_context_pending,
            "captainReturnContext":self.captain_return_context,
            "neverSent":self.never_sent && self.life.latest_user_message_at.is_none(),
            "forkSource":self.native.fork_source, "forkEntry":self.native.fork_entry,
            "previousSession":self.native.previous_session,"accessMode":self.native.access.id(),
            "captain":self.captain, "captainHome":self.captain_home, "crewCaptain":self.crew_captain,
            "delegated":self.delegated,"parent":self.parent,
            "crewReportOutbox":self.crew_report_outbox,"crewReportLastRun":self.crew_report_last_run
        });
        record.as_object_mut().unwrap().extend(json!({
            "peerOrigin":self.peer_origin,"stopSerial":self.stop_serial,
            "admission":self.admission,"admissionRun":self.admission_run,
            // A managed thread's accepted-but-unsent prompt is the durable
            // payload of its admission reservation; without it a queued
            // crew root would survive restart with nothing left to run.
            "admissionPrompt":(self.crew_captain.is_some() || self.peer_origin.is_some())
                .then(|| self.queued_prompt.as_ref().map(|(prompt, _)| json!({
                    "text":prompt.text,"images":prompt.images.iter().map(Attachment::to_value).collect::<Vec<_>>(),
                    "context":prompt.context,"draft":prompt.draft
                })))
                .flatten(),
            "project":self.project,"machine":self.machine,"updated":self.updated,
            "model":self.selected_model.as_ref().map(|(_,id)|id),"thinking":self.selected_thinking,
            "options":self.model_options,
            "contextBreakdown":self.context.usage.breakdown,
            "promptSuggestion":self.prompt_suggestion,
            "serviceTier":self.harness_state["serviceTier"],
            "handoffs":self.handoffs,"handoffSessions":self.handoff_sessions
        }).as_object().unwrap().clone());
        if let Some(worktree_path) = &self.worktree_path {
            record["worktreePath"] = json!(worktree_path);
        }
        if let Some(placement) = &self.placement {
            record["placement"] = placement.value();
        }
        if let Some(directory) = &self.scratch_directory {
            record["scratchDirectory"] = json!(directory);
        }
        if let Some(branch) = &self.branch {
            record["branch"] = json!(branch);
        }
        if let Some(base_branch) = &self.base_branch {
            record["baseBranch"] = json!(base_branch);
        }
        if let Some(env_mode) = self.env_mode {
            record["envMode"] = json!(env_mode);
        }
        if let Some(start_from_origin) = self.start_from_origin {
            record["startFromOrigin"] = json!(start_from_origin);
        }
        if let Some(worktree_setup) = &self.worktree_setup {
            record["worktreeSetup"] = json!(worktree_setup);
        }
        record
    }
}
