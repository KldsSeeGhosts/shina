use super::*;

impl State {
    pub fn open_external_url(&mut self, url: &str) {
        self.pending_open_urls.push(url.to_owned());
    }

    pub fn open_usage_limits(&mut self) -> bool {
        let machine = self.machines.selected.clone();
        let thread = self.current();
        let thread_id = thread.id;
        let harness = thread.harness.clone();
        let client_draft = thread.client_draft;
        if !self.usage_limits_command_available(thread) {
            self.push_toast(
                ToastKind::Warning,
                "Usage limits are unavailable for this provider",
                None,
                None,
                Some(5_000),
            );
            return false;
        }
        if let Some(client) = &self.client {
            let target = if machine == "local" {
                Some(client)
            } else {
                client.linked.get(&machine)
            };
            let Some(target) = target else {
                self.push_toast(
                    ToastKind::Warning,
                    "Usage limits are unavailable for this provider",
                    None,
                    None,
                    Some(5_000),
                );
                return false;
            };
            if !target.has_capability(crate::protocol::USAGE_LIMITS) {
                self.push_toast(
                    ToastKind::Warning,
                    "Usage limits are unavailable for this provider",
                    None,
                    None,
                    Some(5_000),
                );
                return false;
            }
            self.next_usage_limits_request_id += 1;
            let req_id = self.next_usage_limits_request_id;
            self.pending_usage_limits_request = Some((req_id, machine, thread_id, harness.clone()));
            let params = if client_draft {
                json!({"threadId": thread_id, "draft": true, "harness": harness})
            } else {
                json!({"threadId": thread_id})
            };
            target.call_rpc(format!("usage-limits:{req_id}"), "usage.limits", params);
            return true;
        }

        let catalog = match harness.as_str() {
            "codex" => Some(&self.codex_catalog),
            "claude" => Some(&self.claude_catalog),
            _ => None,
        };
        let hs = &thread.harness_state;
        let usage_limits = (!hs["usageLimits"].is_null())
            .then_some(&hs["usageLimits"])
            .or_else(|| catalog.map(|c| &c.usage_limits).filter(|v| !v.is_null()));
        let usage_limits_raw = (!hs["usageLimitsRaw"].is_null())
            .then_some(&hs["usageLimitsRaw"])
            .or_else(|| {
                catalog
                    .map(|c| &c.usage_limits_raw)
                    .filter(|v| !v.is_null())
            });
        let usage_account = (!hs["usageAccount"].is_null())
            .then_some(&hs["usageAccount"])
            .or_else(|| catalog.map(|c| &c.usage_account).filter(|v| !v.is_null()));
        let Some(report) = crate::usage_limits::build_usage_limits_report(
            crate::usage_limits::UsageLimitsReportInput {
                harness: &harness,
                now_ms: lifecycle::now() as i64,
                usage_limits,
                usage_limits_raw,
                usage_account,
            },
        ) else {
            self.push_toast(
                ToastKind::Warning,
                "Usage limits are unavailable for this provider",
                None,
                None,
                Some(5_000),
            );
            return false;
        };
        self.show_usage_limits_report(report);
        true
    }

    pub(crate) fn apply_usage_limits_result(
        &mut self,
        req_id: u64,
        result: Result<crate::usage_limits::UsageLimitsReport, String>,
    ) {
        let Some((pending_id, machine, thread_id, harness)) =
            self.pending_usage_limits_request.as_ref()
        else {
            return;
        };
        if *pending_id != req_id {
            return;
        }
        let matches_current = self.machines.selected == *machine
            && self.current().id == *thread_id
            && self.current().harness == *harness;
        self.pending_usage_limits_request = None;
        if !matches_current {
            return;
        }
        match result {
            Ok(report) => self.show_usage_limits_report(report),
            Err(_) => {
                self.usage_limits_notice_key = None;
                self.composer_notices.remove("usage-limits");
                self.push_toast(
                    ToastKind::Warning,
                    "Usage limits are unavailable for this provider",
                    None,
                    None,
                    Some(5_000),
                );
            }
        }
    }

    pub fn show_usage_limits_report(&mut self, report: crate::usage_limits::UsageLimitsReport) {
        let description = if report.accounts.len() == 1 {
            report
                .accounts
                .first()
                .map(crate::usage_limits::account_summary)
        } else {
            Some(format!("{} accounts", report.accounts.len()))
        };
        self.usage_limits_notice_key = Some((
            self.machines.selected.clone(),
            self.current().id,
            self.current().harness.clone(),
        ));
        self.composer_notices.remove("usage-limits");
        self.composer_notices.push(
            crate::composer_notices::Notice::new(
                "usage-limits",
                crate::composer_notices::Variant::Info,
                "alert",
                "Usage limits",
            )
            .group(crate::composer_notices::Group::Attention)
            .description(description)
            .body(crate::composer_notices::NoticeBody::UsageLimits(report))
            .dismiss_label("Dismiss usage limits"),
        );
    }

    pub fn redeem_usage_reset_credit(
        &mut self,
        input: crate::usage_limits::ProviderConsumeResetCreditInput,
    ) {
        let _ = input;
        if self.client.is_none() && self.current().harness == "codex" && self.current().pi.is_some()
        {
            let key = uuid::Uuid::new_v4().to_string();
            self.current_mut()
                .command("consume_reset_credit", json!({"idempotencyKey": key}));
            return;
        }
        self.push_toast(
            ToastKind::Error,
            "Could not use the reset credit.",
            None,
            None,
            Some(5_000),
        );
    }

    pub fn arm_fresh_session(&mut self, thread_id: usize) {
        self.fresh_arm_seq = self.fresh_arm_seq.saturating_add(1).max(1);
        let arm = self.fresh_arm_seq;
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == thread_id) {
            thread.fresh_session_arm = Some(arm);
        }
        self.sync_fresh_session_notice();
    }

    pub fn disarm_fresh_session(&mut self, thread_id: usize, expected_arm: Option<u64>) {
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == thread_id)
            && expected_arm.is_none_or(|arm| thread.fresh_session_arm == Some(arm))
        {
            thread.fresh_session_arm = None;
        }
        self.sync_fresh_session_notice();
    }

    pub fn dismiss_composer_notice(&mut self, id: &str) {
        if id == "fresh-session" {
            let thread_id = self.current().id;
            self.disarm_fresh_session(thread_id, None);
            self.composer_notices.remove(id);
            self.checkpoint();
        } else if id == "usage-limits" {
            self.usage_limits_notice_key = None;
            self.composer_notices.remove(id);
        } else {
            self.composer_notices.dismiss(id);
        }
    }

    pub fn fallback_model_candidates(&self) -> Vec<crate::model_selection::ModelSelection> {
        let entries = self.provider_instance_entries();
        let models_by_instance = self.picker_models_by_instance();
        let mut out = Vec::new();
        // Order non-default providers first so toggling on prefers an alternate provider.
        let mut ordered_entries = entries.clone();
        ordered_entries.sort_by_key(|e| e.instance_id == self.default_harness);
        for entry in ordered_entries {
            if !entry.enabled || !entry.is_available {
                continue;
            }
            if let Some(opts) = models_by_instance.get(&entry.instance_id)
                && !opts.is_empty()
            {
                for opt in opts {
                    out.push(crate::model_selection::ModelSelection {
                        instance_id: entry.instance_id.clone(),
                        model: opt.slug.clone(),
                        options: Vec::new(),
                    });
                }
            } else {
                out.push(crate::model_selection::ModelSelection {
                    instance_id: entry.instance_id.clone(),
                    model: crate::model_selection::default_model_by_provider(&entry.driver_kind)
                        .to_owned(),
                    options: Vec::new(),
                });
            }
        }
        out
    }

    pub(super) fn default_model_selection_for_fallback(
        &self,
    ) -> Option<crate::model_selection::ModelSelection> {
        let harness = &self.default_harness;
        if let Some(def) = self.thread_defaults.models.get(harness)
            && !def.model.trim().is_empty()
        {
            let mut options = Vec::new();
            if !def.effort.trim().is_empty() {
                let key = if harness == "codex" {
                    "reasoningEffort"
                } else {
                    "effort"
                };
                options.push(crate::model_selection::ProviderOptionSelection {
                    id: key.into(),
                    value: crate::model_selection::OptionValue::Str(def.effort.clone()),
                });
            }
            return Some(crate::model_selection::ModelSelection {
                instance_id: harness.clone(),
                model: def.model.clone(),
                options,
            });
        }
        // The default is for the open thread, so only its workspace's rows count.
        let cwd = self.catalog_workspace_cwd(self.active);
        let model = self
            .default_harness_model(harness, cwd.as_deref())
            .map(|m| m.id.clone())
            .unwrap_or_else(|| {
                crate::model_selection::default_model_by_provider(harness).to_owned()
            });
        Some(crate::model_selection::ModelSelection {
            instance_id: harness.clone(),
            model,
            options: Vec::new(),
        })
    }

    pub(super) fn provider_usage_windows(
        &self,
        harness: &str,
        now_ms: i64,
    ) -> Vec<crate::usage_limits::ServerProviderUsageWindow> {
        if !crate::usage_limits::has_usage_limits(harness) {
            return Vec::new();
        }
        let catalog = match harness {
            "codex" => Some(&self.codex_catalog),
            "claude" => Some(&self.claude_catalog),
            _ => None,
        };
        let thread = if self.current().harness == harness {
            Some(self.current())
        } else {
            self.threads.iter().find(|t| {
                t.harness == harness
                    && (!t.harness_state["usageLimitsRaw"].is_null()
                        || !t.harness_state["usageLimits"].is_null())
            })
        };
        let hs = thread.map(|t| &t.harness_state);
        let usage_limits = hs
            .map(|h| &h["usageLimits"])
            .filter(|v| !v.is_null())
            .or_else(|| catalog.map(|c| &c.usage_limits).filter(|v| !v.is_null()));
        let usage_limits_raw = hs
            .map(|h| &h["usageLimitsRaw"])
            .filter(|v| !v.is_null())
            .or_else(|| {
                catalog
                    .map(|c| &c.usage_limits_raw)
                    .filter(|v| !v.is_null())
            });
        let usage_account = hs
            .map(|h| &h["usageAccount"])
            .filter(|v| !v.is_null())
            .or_else(|| catalog.map(|c| &c.usage_account).filter(|v| !v.is_null()));
        crate::usage_limits::build_usage_limits_report(
            crate::usage_limits::UsageLimitsReportInput {
                harness,
                now_ms,
                usage_limits,
                usage_limits_raw,
                usage_account,
            },
        )
        .and_then(|r| r.accounts.into_iter().next())
        .map(|a| a.limits.windows)
        .unwrap_or_default()
    }

    pub(crate) fn friendly_model_name(&self, harness: &str, model_id: &str) -> String {
        if self.current().harness == harness
            && let Some(m) = self.current().models.iter().find(|m| m.id == model_id)
        {
            return m.name.clone();
        }
        let cwd = self.catalog_workspace_cwd(self.active);
        self.harness_model(harness, model_id, cwd.as_deref())
            .map(|m| m.name.clone())
            .unwrap_or_else(|| model_id.to_owned())
    }

    pub(super) fn apply_selection_to_thread(
        &mut self,
        index: usize,
        selection: &crate::model_selection::ModelSelection,
    ) {
        let prev_active = self.active;
        self.active = index;
        if self.threads[index].harness != selection.instance_id {
            self.choose_harness(&selection.instance_id);
        }
        let provider = if selection.instance_id == "pi" {
            "cpa".to_owned()
        } else {
            selection.instance_id.clone()
        };
        let cwd = self.catalog_workspace_cwd(index);
        let models = self.harness_models_for_cwd(&selection.instance_id, cwd.as_deref());
        if self.threads[index].models.is_empty() && !models.is_empty() {
            self.threads[index].models = models;
        }
        self.threads[index].set_model(provider, selection.model.clone());
        for opt in &selection.options {
            self.threads[index].set_option(opt.id.clone(), opt.value.clone());
        }
        self.reconcile_current_access_mode();
        self.active = prev_active;
    }

    pub fn use_default_from_fallback(&mut self) {
        let Some(info) = self.current().fallback_active.clone() else {
            return;
        };
        self.current_mut().fallback_active = None;
        self.current_mut().fallback_explicit_override = true;
        self.apply_selection_to_thread(self.active, &info.default_selection);
        self.composer_notices.sync("fallback-model", None);
        self.checkpoint();
    }

    /// Evaluates KSG's new-thread fallback rule for the active thread and syncs the
    /// `fallback-model` composer notice.
    pub(crate) fn sync_fallback_model(&mut self, now_ms: i64) -> bool {
        let t = self.current();
        let eligible = !self.demo
            && t.messages.is_empty()
            && t.life.latest_user_message_at.is_none()
            && t.session.is_none()
            && !t.fallback_explicit_override;
        if !eligible {
            self.current_mut().fallback_active = None;
            return self.composer_notices.sync("fallback-model", None);
        }
        let Some(fallback_sel) = self.thread_defaults.fallback_model_selection.clone() else {
            if let Some(prev) = self.current_mut().fallback_active.take() {
                self.apply_selection_to_thread(self.active, &prev.default_selection);
            }
            return self.composer_notices.sync("fallback-model", None);
        };
        let default_sel = self
            .current()
            .fallback_active
            .as_ref()
            .map(|f| f.default_selection.clone())
            .or_else(|| self.default_model_selection_for_fallback());
        let entries = self.provider_instance_entries();
        let windows_by_id: Vec<(String, Vec<crate::usage_limits::ServerProviderUsageWindow>)> =
            entries
                .iter()
                .map(|e| {
                    (
                        e.instance_id.clone(),
                        self.provider_usage_windows(&e.instance_id, now_ms),
                    )
                })
                .collect();
        let candidates: Vec<crate::usage_limits::ProviderFallbackCandidate<'_>> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| crate::usage_limits::ProviderFallbackCandidate {
                instance_id: &e.instance_id,
                driver_kind: &e.driver_kind,
                display_name: &e.display_name,
                enabled: e.enabled,
                installed: e.installed,
                authenticated: !matches!(
                    e.auth_status,
                    crate::model_picker::ProviderAuthStatus::Unauthenticated
                ),
                is_available: e.is_available,
                windows: &windows_by_id[i].1,
            })
            .collect();
        let res = crate::usage_limits::resolve_new_thread_model_selection(
            default_sel.as_ref(),
            None,
            Some(&fallback_sel),
            &candidates,
            now_ms,
        );
        if let (Some(target_sel), Some(info)) = (res.selection, res.fallback) {
            let needs_apply = self.current().fallback_active.as_ref() != Some(&info)
                || self.current().harness != target_sel.instance_id
                || self.current().model_id != target_sel.model;
            if needs_apply {
                self.apply_selection_to_thread(self.active, &target_sel);
                self.current_mut().fallback_active = Some(info.clone());
            }
            let fallback_model_name =
                self.friendly_model_name(&target_sel.instance_id, &target_sel.model);
            let default_model_name = self.friendly_model_name(
                &info.default_selection.instance_id,
                &info.default_selection.model,
            );
            let resets_suffix = info
                .resets_at
                .as_deref()
                .and_then(crate::usage_limits::format_reset_clock_iso)
                .map(|clock| format!(" · resets {clock}"))
                .unwrap_or_default();
            let notice = crate::composer_notices::Notice::new(
                "fallback-model",
                crate::composer_notices::Variant::Info,
                "alert",
                format!("Using {fallback_model_name}"),
            )
            .group(crate::composer_notices::Group::Attention)
            .description(Some(format!(
                "{} is out of usage{resets_suffix}",
                info.default_provider_name
            )))
            .actions(vec![crate::composer_notices::NoticeAction::new(
                format!("Use {default_model_name}"),
                |s| s.use_default_from_fallback(),
            )])
            .dismiss_label("Dismiss fallback model notice");
            self.composer_notices.sync("fallback-model", Some(notice))
        } else {
            if let Some(prev) = self.current_mut().fallback_active.take() {
                self.apply_selection_to_thread(self.active, &prev.default_selection);
            }
            self.composer_notices.sync("fallback-model", None)
        }
    }

    pub(crate) fn sync_fresh_session_notice(&mut self) -> bool {
        let t = self.current();
        let show =
            self.fresh_session_available(t) && t.fresh_session_arm.is_some() && t.request.is_none();
        let notice = show.then(|| {
            crate::composer_notices::Notice::new(
                "fresh-session",
                crate::composer_notices::Variant::Info,
                "alert",
                "Your next message starts a fresh session",
            )
            .group(crate::composer_notices::Group::Attention)
            .description(Some(
                "The agent won't see earlier messages in this thread.".into(),
            ))
            .dismiss_label("Keep the current session")
        });
        self.composer_notices.sync("fresh-session", notice)
    }

    pub(crate) fn keep_full_history(&self, thread: &Thread) -> bool {
        self.keep_full_history_threads
            .contains(&(self.machines.selected.clone(), thread.id))
    }

    pub(crate) fn set_keep_full_history(&mut self, server: &str, thread_id: usize, keep: bool) {
        let key = (server.to_owned(), thread_id);
        if keep {
            self.keep_full_history_threads.insert(key);
        } else {
            self.keep_full_history_threads.remove(&key);
        }
    }

    pub fn toggle_keep_full_history(&mut self) {
        let server = self.machines.selected.clone();
        let thread_id = self.current().id;
        let next = !self.keep_full_history(self.current());
        self.set_keep_full_history(&server, thread_id, next);
    }

    /// Ported from KSG Code's `compactBeforeSendTokens` / `resumeCompactionTokens`
    /// (`packages/client-runtime/src/state/contextWindow.ts` and `ChatView.tsx`).
    pub(crate) fn resume_compaction_tokens(&self, thread: &Thread, now_ms: u64) -> Option<u64> {
        if self.demo {
            return None;
        }
        if self.fresh_session_available(thread) && thread.fresh_session_arm.is_some() {
            return None;
        }
        if thread.archived()
            || thread.status.busy()
            || thread.initializing
            || thread.configuring
            || thread.native.operating()
            || thread.context.phase == crate::context::ContextPhase::Compacting
            || thread.native.queue_held
            || thread.request.is_some()
            || thread.native.questionnaire.is_some()
            || !crate::harness::supports_compaction(&thread.harness)
        {
            return None;
        }
        let has_prior_turn = thread.session.is_some()
            || thread.messages.iter().any(|m| {
                m.role == "You"
                    && (!m.text.trim().eq_ignore_ascii_case("/compact") || !m.images.is_empty())
            });
        if !has_prior_turn {
            return None;
        }
        let used_tokens = thread.context.usage.tokens?;
        crate::context::should_offer_resume_compaction(crate::context::ResumeCompactionInput {
            harness: &thread.harness,
            used_tokens: Some(used_tokens),
            updated_at_ms: thread.context.updated_at,
            prompt_cache_ttl_ms: thread.context.prompt_cache_ttl_ms,
            now_ms,
        })
        .then_some(used_tokens)
    }

    pub fn prompt_suggestions_enabled(&self) -> bool {
        if self.client.is_some() {
            self.machines
                .views
                .get(&self.machines.selected)
                .and_then(|v| v["index"]["threadDefaults"]["prompt_suggestions_enabled"].as_bool())
                .unwrap_or(self.thread_defaults.prompt_suggestions_enabled)
        } else {
            self.thread_defaults.prompt_suggestions_enabled
        }
    }

    /// Ported condition by condition from KSG Code's `usePromptSuggestion.ts` and `ChatComposer.tsx`.
    pub fn offered_prompt_suggestion(&self) -> Option<&str> {
        if !self.prompt_suggestions_enabled()
            || self.demo
            || self.ownership_denied
            || self.unreachable_selected().is_some()
        {
            return None;
        }
        let t = self.current();
        if t.delegated
            || t.archived()
            || t.offline_blocked
            || t.status != Status::Completed
            || t.configuring
            || t.initializing
            || t.phase == "Stopping"
            || t.remote_send.is_some()
            || t.client_creating.is_some()
            || t.queued_edit.is_some()
            || t.request.is_some()
            || t.native.questionnaire.is_some()
            || !t.draft.text.is_empty()
            || t.draft.is_composing()
            || t.draft.pills().next().is_some()
            || !t.draft_images.is_empty()
            || !t.draft_context.is_empty()
            || t.draft_reply_to.is_some()
            || !crate::harness::supports_prompt_suggestions(&t.harness)
        {
            return None;
        }
        let run_id = t.run_id.as_deref()?;
        let suggestion = t.prompt_suggestion.as_ref()?;
        if suggestion.run_id != run_id || suggestion.text.trim().is_empty() {
            return None;
        }
        if self
            .dismissed_prompt_suggestions
            .contains(&(t.machine.clone(), t.id, run_id.to_owned()))
        {
            return None;
        }
        Some(suggestion.text.as_str())
    }

    pub fn dismiss_prompt_suggestion(&mut self) -> bool {
        if self.offered_prompt_suggestion().is_none() {
            return false;
        }
        let machine = self.current().machine.clone();
        let thread_id = self.current().id;
        if let Some(run_id) = self.current().run_id.clone() {
            self.dismissed_prompt_suggestions
                .insert((machine, thread_id, run_id));
            return true;
        }
        false
    }

    pub fn accept_prompt_suggestion(&mut self) -> bool {
        let Some(text) = self.offered_prompt_suggestion().map(str::to_owned) else {
            return false;
        };
        let t = self.current_mut();
        t.draft.anchor = 0;
        t.draft.focus = 0;
        t.draft.insert(&text);
        t.native.slash_index = 0;
        t.composer_source.on_doc_change(&t.draft);
        t.composer_rules.changed(&mut t.draft);
        t.reconcile_draft_context();
        t.typing_guard.on_type(std::time::Instant::now());
        t.update_submission_error();
        self.ensure_composer_commands();
        self.checkpoint();
        true
    }

    pub(super) fn dispatch_send(&mut self) -> bool {
        self.dispatch_send_full(false, false, None, None)
    }

    pub(crate) fn dispatch_send_with_options(
        &mut self,
        forced_fresh_context: Option<bool>,
        forced_compact_before_send: Option<bool>,
    ) -> bool {
        self.dispatch_send_full(
            false,
            false,
            forced_fresh_context,
            forced_compact_before_send,
        )
    }

    pub(super) fn dispatch_send_intent(
        &mut self,
        background: bool,
        approval_redirect: bool,
    ) -> bool {
        self.dispatch_send_full(background, approval_redirect, None, None)
    }

    pub(super) fn dispatch_send_full(
        &mut self,
        background: bool,
        approval_redirect: bool,
        forced_fresh_context: Option<bool>,
        forced_compact_before_send: Option<bool>,
    ) -> bool {
        // A send from a ghost tab keeps it, as KSG's ghost tab does on submit.
        let sent_from = self.threads.get(self.active).map(|t| t.id);
        let sent = self.dispatch_send_inner(
            background,
            approval_redirect,
            forced_fresh_context,
            forced_compact_before_send,
        );
        if sent && let Some(id) = sent_from {
            self.keep_tab(id);
        }
        sent
    }
    pub(super) fn dispatch_send_inner(
        &mut self,
        background: bool,
        approval_redirect: bool,
        forced_fresh_context: Option<bool>,
        forced_compact_before_send: Option<bool>,
    ) -> bool {
        if self.demo || self.ownership_denied || self.current().archived() {
            return false;
        }
        if let Err(error) = self.require_active_thread_owner(self.current().id) {
            self.current_mut().error = Some(error);
            return false;
        }
        if self.remote_task(self.current().id).is_some() {
            self.current_mut().submission_error = Some(
                "This is a remote task result. Open its thread on the target machine, or send a follow-up from its parent.".into(),
            );
            return false;
        }
        // KSG's useComposerSend refuses while readPendingEnvironmentChange is
        // set: the draft is still being addressed to its new machine.
        if self.machines.draft_retarget.is_some() {
            return false;
        }
        if let Some(reason) = self.attachment_send_block_reason() {
            self.current_mut().attachment_error = Some(reason);
            return false;
        }
        if self.has_pending_context_imports(&self.machines.selected, self.current().id) {
            self.current_mut().attachment_error = Some(
                "Still bringing attached context into this message. Send again once its chip resolves."
                    .into(),
            );
            return false;
        }
        if self.current().resume_pending() || self.current().responding_request.is_some() {
            return false;
        }
        if let Some(placement) = self.current().placement.as_ref()
            && placement.phase != placement::PlacementPhase::Ready
        {
            let setup_failed = self
                .current()
                .worktree_setup
                .as_ref()
                .is_some_and(|setup| setup.phase == WorktreeSetupPhase::Failed);
            let message = match placement.phase {
                placement::PlacementPhase::Pending => {
                    "This worker's checkout is still preparing. Nothing started; wait for the placement to finish."
                }
                placement::PlacementPhase::Failed if setup_failed => {
                    "Nothing started. Open the worker to retry or skip its setup script."
                }
                placement::PlacementPhase::Failed => {
                    "Nothing started. New work from its parent or Captain retries the same checkout."
                }
                placement::PlacementPhase::Ready => unreachable!(),
            }
            .to_owned();
            let thread = self.current_mut();
            thread.submission_error = Some(message.clone());
            thread.error = Some(message);
            return false;
        }
        if self.current().crew_draft.is_some() {
            if background || approval_redirect {
                self.current_mut().crew_draft_notice =
                    Some("Send this crew thread normally before starting background work.".into());
                return false;
            }
            return self.send_crew_draft();
        }
        if self.current().queued_edit.is_some() {
            let target_server_id = self.machine_identity(&self.machines.selected);
            let peers = self.peers.clone();
            let labels: Vec<(String, String)> = peers
                .iter()
                .map(|p| (p.server_id.clone(), p.label.clone()))
                .collect();
            self.current_mut()
                .save_queued_edit_with_peers(Some(&target_server_id), &peers, &|m| {
                    if m == "local" {
                        "Local".into()
                    } else {
                        labels
                            .iter()
                            .find(|(id, _)| id == m)
                            .map(|(_, l)| l.clone())
                            .unwrap_or_else(|| m.to_owned())
                    }
                });
            self.checkpoint();
            return false;
        }
        if self.current().approval_send.is_some() {
            return false;
        }
        if !self.headless
            && !self.current().request_held_by_typing_guard()
            && (self.current().native.questionnaire.is_some()
                || self
                    .current()
                    .request
                    .as_ref()
                    .is_some_and(|r| r["method"] != "confirm"))
        {
            return false;
        }
        if self.current().request.is_some() {
            if self.current().draft.text.trim() == "/model" {
                self.choose_slash("model".into());
                return false;
            }
            if self.client.is_none() && self.handle_composer_local_command() {
                return false;
            }
            if !self.approval_send_is_local_command()
                && !approval_redirect
                && !self.headless
                && !self.current().request_held_by_typing_guard()
            {
                if let Some(error) = self.approval_send_command_error() {
                    self.current_mut().error = Some(error);
                    return false;
                }
                return self.begin_approval_send(background);
            }
        }
        self.reconcile_current_access_mode();
        if !self
            .current()
            .draft_model_state
            .additional_models
            .is_empty()
        {
            return self.dispatch_multi_model_send();
        }
        let fresh_session_available = self.fresh_session_available(self.current());
        let draft_markdown = crate::composer_rich::markdown(&self.current().draft);
        let composer_holds_only_text = self.current().draft_images.is_empty()
            && self.current().draft_context.is_empty()
            && self.current().draft_reply_to.is_none()
            && self.current().draft.pills().next().is_none();
        if forced_fresh_context.is_none()
            && fresh_session_available
            && composer_holds_only_text
            && is_clear_command(&draft_markdown)
        {
            let thread_id = self.current().id;
            self.arm_fresh_session(thread_id);
            let t = self.current_mut();
            t.prompt_history.reset();
            t.typing_guard.on_blur_or_send();
            t.paste_history = Default::default();
            t.draft.replace_text("");
            t.submission_error = None;
            t.attachment_error = None;
            self.checkpoint();
            return false;
        }
        if composer_holds_only_text
            && self.usage_limits_command_available(self.current())
            && crate::usage_limits::is_usage_limits_command(&draft_markdown)
        {
            if self.open_usage_limits() {
                let t = self.current_mut();
                t.prompt_history.reset();
                t.typing_guard.on_blur_or_send();
                t.paste_history = Default::default();
                t.draft.replace_text("");
                t.submission_error = None;
                t.attachment_error = None;
                self.checkpoint();
            }
            return false;
        }
        // Read once: an arm made while this send is in flight belongs to the next message.
        let fresh_arm = if forced_fresh_context.is_some() {
            None
        } else if fresh_session_available {
            self.current().fresh_session_arm
        } else {
            None
        };
        let fresh_context = forced_fresh_context.unwrap_or(fresh_arm.is_some());
        let compact_before_send = forced_compact_before_send.unwrap_or_else(|| {
            !fresh_context
                && self
                    .resume_compaction_tokens(self.current(), lifecycle::now())
                    .is_some()
                && !self.keep_full_history(self.current())
                && !is_compact_command(&draft_markdown)
        });
        if self.client.is_some() {
            if self.current().draft.text.trim() == "/model" {
                self.choose_slash("model".into());
                return false;
            }
            if !self.current().has_draft_content() {
                return false;
            }
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
            if self.current().remote_send.is_some() {
                return false;
            }
            // KSG snapshots and clears at dispatch. A refusal merges it with later edits.
            let revision = uuid::Uuid::new_v4();
            let target_machine = self.machines.selected.clone();
            let target_server_id = self.machine_identity(&target_machine);
            let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
                &self.current().draft,
                &self.current().draft_images,
                &self.current().draft_context,
                self.current().draft_reply_to.as_ref(),
                Some(&target_machine),
                Some(&target_server_id),
                &self.peers,
                &|machine| self.machine_label(machine),
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
            if let Some(error) = self.missing_worktree_base_branch_error(self.current()) {
                self.current_mut().submission_error = Some(error);
                return false;
            }
            self.current_mut().submission_error = None;
            if self.current().client_draft {
                self.promote_client_draft();
                return self.current().client_creating.is_some();
            }
            let supports_fresh_cap = self.client.as_ref().is_some_and(|c| {
                c.supports_capability(&self.current().machine, crate::protocol::FRESH_CONTEXT)
            });
            let supports_vcs_cap = self.client.as_ref().is_some_and(|c| {
                c.supports_capability(&self.current().machine, crate::protocol::VCS_WORKTREES)
            });
            let mut mode = if (self.current().status.busy()
                && (self.current().follow_up_queues() || fresh_context))
                || self.current().native.queue_held
            {
                "queue"
            } else if self.current().status.busy() {
                "steer"
            } else {
                "prompt"
            };
            let mut params = json!({"threadId":self.current().id,"text":prompt.text,
                "attachments":prompt.images.as_ref(),"context":prompt.context,"draft":prompt.draft,"editorRevision":revision,"mode":mode});
            let sending_worktree_bootstrap = supports_vcs_cap
                && self.current().worktree_path.is_none()
                && self.current().messages.is_empty()
                && self.current().project.is_some()
                && !self.is_captain_home(self.current())
                && self.effective_env_mode(self.current()) == ThreadEnvMode::Worktree;
            if sending_worktree_bootstrap {
                params["envMode"] = json!(ThreadEnvMode::Worktree);
                if let Some(base) = &self.current().base_branch {
                    params["baseBranch"] = json!(base);
                }
                params["startFromOrigin"] = json!(self.current().effective_start_from_origin());
            }
            if fresh_context {
                params["freshContext"] = json!(true);
            }
            if compact_before_send {
                if supports_fresh_cap {
                    params["compactBeforeSend"] = json!(true);
                } else {
                    let _ = self.remote("thread.compact", json!({"instructions": Value::Null}));
                    mode = "queue";
                    params["mode"] = json!(mode);
                }
            }
            let result = self.remote("thread.send", params);
            return match result {
                Some(Ok(id)) => {
                    let snapshot = crate::composer_rich::Snapshot::capture_with_context(
                        &self.current().draft,
                        &self.current().draft_images,
                        &self.current().draft_context,
                    );
                    let thread = self.current_mut();
                    let reply_for_remote = thread.draft_reply_to.clone();
                    if sending_worktree_bootstrap {
                        thread.remote_bootstrap_snapshot =
                            Some((snapshot.clone(), reply_for_remote));
                    }
                    thread.remote_send = Some((id, snapshot));
                    thread.remote_send_fresh_arm = fresh_arm;
                    thread.remote_send_reply_to = thread.draft_reply_to.take();
                    thread.draft.replace_text("");
                    thread.draft_images.clear();
                    thread.draft_context.clear();
                    thread.retained_context = Default::default();
                    thread.typing_guard.on_blur_or_send();
                    self.usage_limits_notice_key = None;
                    self.composer_notices.remove("usage-limits");
                    true
                }
                Some(Err(error)) => {
                    self.storage_error = Some(error);
                    false
                }
                None => false,
            };
        }
        if self.network_blocked() {
            self.current_mut().error =
                Some("Waiting for the network after resume. Your draft has not been sent.".into());
            return false;
        }
        if self.current().status == Status::Paused {
            self.current_mut().status = Status::Ready;
            self.current_mut().error = None;
        }
        if self.current().configuring
            || self.current().native.operating()
            || self.current().phase == "Stopping"
            || !self.current().has_draft_content()
        {
            return false;
        }
        let text = draft_markdown.trim().to_owned();
        let mut words = text.splitn(2, char::is_whitespace);
        match words.next().filter(|_| {
            !self
                .current()
                .draft
                .pills()
                .any(|(r, _)| self.current().draft.text[..r.start].trim().is_empty())
                && !crate::composer_tokens::starts_with_file_path(
                    &text,
                    &self.current().draft_images,
                )
        }) {
            Some("/model") => {
                self.choose_slash("model".into());
                return false;
            }
            Some(command) if command.eq_ignore_ascii_case("/clear") => {}
            Some(command)
                if command.eq_ignore_ascii_case("/usage-limits")
                    && self.usage_limits_command_available(self.current()) => {}
            Some("/compact") if crate::harness::supports_compaction(&self.current().harness) => {
                if !self.current().draft_images.is_empty() {
                    self.current_mut().attachment_error =
                        Some("Compaction does not accept attachments. Your draft was kept.".into());
                    return false;
                }
                if self.current().can_compact() {
                    let instructions = words
                        .next()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    self.current_mut().draft.replace_text("");
                    self.compact_context(instructions);
                }
                return false;
            }
            Some(command @ ("/goal" | "/feedback")) if self.current().harness == "codex" => {
                if self.current().harness_state["codexNativeExtensions"] != true {
                    self.current_mut().error = Some("This Codex version has no verified native command protocol. Update Codex outside Shina.".into());
                    return false;
                }
                let arguments = words.next().unwrap_or("").trim().to_owned();
                if self.current().status.busy()
                    && command == "/goal"
                    && !matches!(arguments.as_str(), "" | "pause" | "clear")
                {
                    self.current_mut().error = Some("Wait for Codex to finish before running this command. Your draft was kept.".into());
                    return false;
                }
                if !self.current().draft_images.is_empty() {
                    self.current_mut().attachment_error = Some(
                        "Native commands do not accept attachments. Your draft was kept.".into(),
                    );
                    return false;
                }
                self.current_mut().native.pending_command =
                    Some(json!({"name":command.trim_start_matches('/'),"arguments":arguments}));
                self.current_mut().draft.replace_text("");
                if self.current().pi.is_none() {
                    self.connect_thread(self.active);
                } else {
                    self.current_mut().drive_session();
                }
                return false;
            }
            Some(command) if command.starts_with('/') => {
                let name = command.trim_start_matches('/');
                if !self
                    .current()
                    .native
                    .commands
                    .iter()
                    .any(|c| c.name == name)
                {
                    self.current_mut().error = Some(format!(
                        "Unknown {} command {command}. Use / to browse the workspace's commands.",
                        if self.current().harness == "pi" {
                            "Pi"
                        } else {
                            "ACP"
                        }
                    ));
                    return false;
                }
            }
            _ => {}
        }
        if self
            .current()
            .request
            .as_ref()
            .is_some_and(crate::approval::not_resumable)
        {
            self.current_mut().request = None;
        }
        if self.thread_admission_managed(self.active)
            && let Err(error) = self.check_thread_admission(self.active)
        {
            self.current_mut().submission_error = Some(error);
            return false;
        }
        self.release_task_results(self.current().id, captain_inbox::ReleaseActor::User);
        if let Some(sent) = self.try_worktree_bootstrap_send(fresh_context, fresh_arm) {
            if sent && self.thread_admission_managed(self.active) {
                let peer_worker = self.current().peer_origin.is_some();
                self.ensure_thread_admission(self.active, peer_worker, false);
            }
            return sent;
        }
        if compact_before_send && !self.current().status.busy() {
            if self.current().harness == "pi"
                && !self.current().model_provider.is_empty()
                && self.current().model_provider != "cpa"
            {
                self.current_mut().error = Some(
                    "Choose a CPA model before sending. Shina will not send prompts to a direct provider.".into(),
                );
                return false;
            }
            if self.current().draft_images.iter().any(Attachment::is_image)
                && self.current().supports_images == Some(false)
            {
                let err = self.current().image_model_error();
                self.current_mut().attachment_error = Some(err);
                return false;
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
                    return false;
                }
            };
            if !self.headless
                && let Some(error) = self.current().validate_prompt_length(&prompt)
            {
                self.current_mut().submission_error = Some(error);
                return false;
            }
            self.current_mut().submission_error = None;
            if self.thread_admission_managed(self.active) {
                let peer_worker = self.current().peer_origin.is_some();
                self.ensure_thread_admission(self.active, peer_worker, false);
            }
            let thread = self.current_mut();
            thread.prompt_history.reset();
            thread.typing_guard.on_blur_or_send();
            thread.paste_history = Default::default();
            thread.draft.replace_text("");
            thread.draft_images.clear();
            thread.draft_context.clear();
            thread.draft_reply_to = None;
            thread.retained_context = Default::default();
            thread.native.queue_held = false;
            thread.serial += 1;
            let queued_id = thread.serial;
            thread.native.queue.push_back(pi_session::QueuedMessage {
                id: queued_id,
                prompt,
            });
            thread.life.user_sent(lifecycle::now());
            let id = thread.id;
            let server = self.machines.selected.clone();
            self.set_keep_full_history(&server, id, false);
            self.invalidate_undo(id);
            self.compact_context(None);
            self.checkpoint();
            return true;
        }
        if self.current().status.busy() {
            if self.thread_admission_managed(self.active) {
                let peer_worker = self.current().peer_origin.is_some();
                self.ensure_thread_admission(self.active, peer_worker, false);
            }
            if !self.headless {
                if let Ok(prompt) = crate::composer_tokens::prompt_for_target_with_reply(
                    &self.current().draft,
                    &self.current().draft_images,
                    &self.current().draft_context,
                    self.current().draft_reply_to.as_ref(),
                    Some(&self.current().machine),
                    Some(&self.machine_identity(&self.machines.selected)),
                    &self.peers,
                    &|machine| self.machine_label(machine),
                ) && let Some(error) = self.current().validate_prompt_length(&prompt)
                {
                    self.current_mut().submission_error = Some(error);
                    return false;
                }
                self.current_mut().submission_error = None;
            }
            let local_server_id = self.machine_identity(&self.machines.selected);
            let peers = self.peers.clone();
            let labels: Vec<(String, String)> = peers
                .iter()
                .map(|p| (p.server_id.clone(), p.label.clone()))
                .collect();
            self.current_mut().send_during_run_with_fresh(
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
                fresh_context,
            );
            let dispatched = !self.current().has_draft_content();
            if dispatched {
                if fresh_context {
                    self.persist_transcripts = true;
                }
                let thread_id = self.current().id;
                let server = self.machines.selected.clone();
                self.set_keep_full_history(&server, thread_id, false);
                if let Some(arm) = fresh_arm {
                    self.disarm_fresh_session(thread_id, Some(arm));
                }
                self.checkpoint();
            }
            return dispatched;
        }
        if !self.headless {
            if let Ok(prompt) = crate::composer_tokens::prompt_for_target_with_reply(
                &self.current().draft,
                &self.current().draft_images,
                &self.current().draft_context,
                self.current().draft_reply_to.as_ref(),
                Some(&self.current().machine),
                Some(&self.machine_identity(&self.machines.selected)),
                &self.peers,
                &|machine| self.machine_label(machine),
            ) && let Some(error) = self.current().validate_prompt_length(&prompt)
            {
                self.current_mut().submission_error = Some(error);
                return false;
            }
            self.current_mut().submission_error = None;
        }
        self.usage_limits_notice_key = None;
        self.composer_notices.remove("usage-limits");
        let local_server_id = self.machine_identity(&self.machines.selected);
        let peers = self.peers.clone();
        let labels: Vec<(String, String)> = peers
            .iter()
            .map(|p| (p.server_id.clone(), p.label.clone()))
            .collect();
        let managed = self.thread_admission_managed(self.active);
        if managed {
            // Reserve while the provider is still idle. begin_prompt drives
            // an existing connection and marks Running before it returns.
            let peer_worker = self.current().peer_origin.is_some();
            self.ensure_thread_admission(self.active, peer_worker, false);
        }
        self.current_mut().send_with_fresh(
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
            fresh_context,
        );
        if self.current().has_draft_content() || self.current().status != Status::Running {
            if managed {
                self.admission_pump();
            }
            return false;
        }
        let id = self.current().id;
        let server = self.machines.selected.clone();
        self.set_keep_full_history(&server, id, false);
        self.invalidate_undo(id);
        if fresh_context {
            self.persist_transcripts = true;
        }
        if let Some(arm) = fresh_arm {
            self.disarm_fresh_session(id, Some(arm));
        }
        if managed {
            self.admission_pump();
        }
        if self.current().pi.is_none() && self.current().queued_prompt.is_some() {
            self.connect_thread(self.active);
        }
        self.checkpoint();
        true
    }

    pub fn retry(&mut self) {
        if self.forward("thread.retry", json!({"threadId":self.current().id})) {
            return;
        }
        if self.demo
            || self.network_blocked()
            || self.current().archived()
            || self.current().status.busy()
        {
            return;
        }
        if !self.current().has_draft_content()
            && let Some(prompt) = self.current().retry_prompt.clone()
        {
            self.current_mut().restore_prompt(&prompt);
        }
        if !self.current().has_draft_content() {
            self.current_mut().disconnect();
            self.current_mut().error = None;
            self.current_mut().status = Status::Ready;
            self.connect_thread(self.active);
        } else {
            self.send();
        }
    }

    pub fn refresh_catalog(&mut self) {
        self.refresh_provider_availability(true);
        if self.forward("catalog.refresh", json!({})) {
            return;
        }
        self.refresh_native_catalogs(true, false);
        if matches!(self.current().harness.as_str(), "codex" | "claude")
            && self.current().pi.is_some()
        {
            self.current_mut()
                .command("get_available_models", json!({}));
            self.current_mut().refresh_commands();
        }
        if !self.demo && self.runtime_enabled {
            self.catalog
                .refresh(self.data_dir.clone(), self.wake.clone());
        }
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
    }

    pub fn supports_model_catalog_refresh(&self) -> bool {
        self.client.as_ref().is_none_or(|client| {
            client.supports_capability(
                &self.machines.selected,
                crate::protocol::MODEL_CATALOG_REFRESH,
            )
        })
    }

    /// Ported from KSG Code's `ProviderModelPicker.tsx`: opening the picker refreshes the model
    /// catalog through the capability-gated `modelCatalogOnly` path, throttled by 5 minutes.
    pub fn refresh_catalog_on_open(&mut self) {
        self.refresh_provider_availability(false);
        if self.client.is_some() {
            if !self.supports_model_catalog_refresh() {
                return;
            }
            self.forward("catalog.refresh", json!({"modelCatalogOnly": true}));
            return;
        }
        self.refresh_native_catalogs(false, false);
        if !self.demo && self.runtime_enabled {
            self.catalog
                .refresh_with_mode(self.data_dir.clone(), self.wake.clone(), false);
        }
    }

    /// Explicit retry button bypasses the on-demand throttle and retry backoff.
    pub fn force_refresh_catalog(&mut self) {
        self.catalog.error = None;
        self.refresh_provider_availability(true);
        if self.forward("catalog.refresh", json!({"force": true})) {
            return;
        }
        self.refresh_native_catalogs(true, false);
        if matches!(self.current().harness.as_str(), "codex" | "claude")
            && self.current().pi.is_some()
        {
            self.current_mut()
                .command("get_available_models", json!({}));
            self.current_mut().refresh_commands();
        }
        if !self.demo && self.runtime_enabled {
            self.catalog
                .refresh_with_mode(self.data_dir.clone(), self.wake.clone(), true);
        }
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
    }

    /// Warm metadata on a command/skill trigger without sending a prompt. A failed startup
    /// or denied login stays failed until the user takes an explicit recovery action.
    pub fn ensure_composer_commands(&mut self) {
        if self.demo || !self.runtime_enabled && self.client.is_none() {
            return;
        }
        let t = self.current();
        if !crate::composer::trigger(&t.draft).is_some_and(|t| matches!(t.marker, '/' | '$'))
            || t.native.commands_loaded
            || t.native.commands_requested
            || t.native.commands_error.is_some()
            || t.initializing
            || t.error.is_some()
            || t.status == Status::Failed
            || t.archived()
        {
            return;
        }
        self.current_mut().native.commands_requested = true;
        if self.client.is_some() || self.current().pi.is_some() {
            self.current_mut().refresh_commands();
        } else {
            self.connect_thread(self.active);
        }
    }

    /// Changing a new thread's harness retires only its unused prewarmed process.
    pub fn choose_harness(&mut self, id: &str) {
        if self.forward(
            "thread.configure",
            json!({"threadId":self.current().id,"harness":id}),
        ) {
            return;
        }
        if !self.current().can_choose_harness() || self.current().harness == id {
            return;
        }
        if let Err(error) = self.validate_harness(id) {
            self.current_mut().error = Some(error);
            return;
        }
        let thread = self.current_mut();
        thread.disconnect();
        thread.harness = id.into();
        thread.harness_context_sent = false;
        thread.harness_state = json!({});
        thread.session = None;
        thread.selected_model = None;
        thread.selected_thinking = None;
        thread.model_provider.clear();
        thread.model_id.clear();
        thread.model = DEFAULT_MODEL.into();
        thread.thinking.clear();
        thread.models.clear();
        thread.thinking_levels.clear();
        thread.context_window = None;
        thread.supports_images = None;
        thread.context.begin_model_change();
        thread.native.commands.clear();
        thread.native.skills.clear();
        thread.native.commands_requested = false;
        thread.native.commands_loaded = false;
        thread.error = None;
        thread.prewarm_attempted = false;
        let access = self.effective_access_mode(self.current());
        self.current_mut().native.access = access;
        if id == "pi" {
            self.apply_catalog();
        } else {
            self.apply_native_catalogs();
        }
        self.checkpoint();
        self.refresh_native_catalogs(false, false);
    }

    pub(super) fn apply_catalog(&mut self) {
        for thread in &mut self.threads {
            if thread.harness != "pi" {
                continue;
            }
            if thread.models != self.catalog.models {
                thread.models.clone_from(&self.catalog.models);
            }
            if thread.model_id.is_empty()
                && thread.session.is_none()
                && let Some(model) = &self.catalog.default_model
            {
                thread.model_provider = model.provider.clone();
                thread.model_id = model.id.clone();
                thread.model = model.name.clone();
                thread.context_window = model.context_window;
                thread.supports_images = model.supports_images;
                if thread.selected_thinking.is_none() {
                    thread.thinking = self.catalog.default_thinking.clone();
                }
            }
            let metadata = thread
                .models
                .iter()
                .find(|m| m.id == thread.model_id)
                .map(|m| (m.name.clone(), m.context_window, m.supports_images));
            if let Some((name, window, images)) = metadata {
                thread.model = name;
                if window.is_some() {
                    thread.context_window = window;
                }
                if images.is_some() {
                    thread.supports_images = images;
                }
            }
            thread.refresh_thinking_levels();
        }
    }

    pub(super) fn prewarm_current(&mut self) -> bool {
        let thread = self.current();
        if self.remote_task(thread.id).is_some() {
            return false;
        }
        if self.runtime_enabled
            && !self.demo
            && !thread.archived()
            && self.require_active_thread_owner(thread.id).is_ok()
            && thread.pi.is_none()
            && !thread.prewarm_attempted
            && thread.status != Status::Failed
            && !(self.effective_env_mode(thread) == ThreadEnvMode::Worktree
                && thread.worktree_path.is_none())
            && (thread.has_draft_content() || (thread.session.is_some() && !thread.history_loaded))
        {
            self.connect_thread(self.active);
            return true;
        }
        false
    }
}
