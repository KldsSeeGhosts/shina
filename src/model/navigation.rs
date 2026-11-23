use super::*;

impl State {
    /// Headless snapshots read a supplied isolated profile without starting Pi or saving.
    pub fn load_snapshot(&mut self) {
        let result = (|| {
            if let Some(index) = crate::store::load_global_readonly(&self.data_dir, None)? {
                if index["version"] != 3
                    && index["version"] != 4
                    && index["version"] != crate::identity::INDEX_VERSION
                {
                    return Err("Unsupported index version.".into());
                }
                self.restore_global(&index)?;
            }
            self.load_memory(false)?;
            if let Some(ui) = crate::store::load(&self.data_dir.join("ui-state.json"))? {
                self.restore_ui(&ui)?;
            }
            if self.keybindings_path().exists() {
                let file_state =
                    crate::keybindings::load_runtime_keybindings_file(&self.keybindings_path());
                self.preferences.keybindings = file_state.rules;
                self.settings.keybindings_issues = file_state.issues;
                self.keybindings_watcher
                    .record_stamp(&self.keybindings_path());
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            self.storage_error = Some(error);
        }
    }
    pub fn new(workspace: PathBuf, demo: bool) -> Self {
        let state = Self::construct(workspace, demo);
        // Preserve desktop startup timing. Headless construction never touches these.
        let _ = &*state.artwork;
        let _ = &*state.portraits;
        let _ = &*state.attachment_cache;
        state
    }
    pub(super) fn construct(workspace: PathBuf, demo: bool) -> Self {
        let workspace = if workspace.as_os_str().is_empty() {
            workspace
        } else {
            crate::projects::normalize_path(&workspace)
        };
        let art_path = workspace.clone();
        let portrait_path = workspace.clone();
        let art = Arc::new(std::sync::OnceLock::new());
        let portrait_art = art.clone();
        let artwork = crate::runtime::UiResource::new(move || {
            art.get_or_init(|| crate::visuals::Artwork::load(&art_path))
                .clone()
        });
        let attachment_cache =
            crate::runtime::UiResource::new(|| rune::ImageCache::new(Arc::new(|| {})));
        let mut state = Self {
            client: None,
            machines: Default::default(),
            peers: Vec::new(),
            peer_runtime: Default::default(),
            headless: false,
            persistence_scope: None,
            scheduler: admission::Scheduler::default(),
            provider_update_drivers: Default::default(),
            owner: None,
            command_receipts: Vec::new(),
            command_intents: Default::default(),
            server_epoch: uuid::Uuid::new_v4().to_string(),
            server_seq: 0,
            persist_transcripts: false,
            dismissed_prompt_suggestions: Default::default(),
            client_views: json!({}),
            projected_native_catalogs: Default::default(),
            apply_stats: Default::default(),
            next_client_draft: usize::MAX,
            tab_epoch: 0,
            tab_intents: Default::default(),
            requested_tab: None,
            requested_tab_restore: None,
            undo_tabs: Default::default(),
            deleted_threads: Vec::new(),
            deleted_records: None,
            client_recover: Default::default(),
            recovered_inputs: Default::default(),
            background: Default::default(),
            attachment_uploads: Default::default(),
            data_dir: crate::store::directory()
                .unwrap_or_else(|| std::env::temp_dir().join("Shina")),
            html_windows: Default::default(),
            html_lane_width: crate::html_render::COLUMN_WIDTH,
            launch_project: (!workspace.as_os_str().is_empty()).then_some(workspace.clone()),
            threads: vec![Thread::new(1)],
            active: 0,
            tabs: vec![1],
            // The strip is opt-in for users; the test suite predates it and
            // exercises the strip everywhere, so the test build defaults it on.
            tabs_enabled: cfg!(test),
            ghost_tab: None,
            ghost_anchor: None,
            nav_history: NavHistory::default(),
            nav_open: false,
            next_id: 2,
            dark: false,
            zoom: 1.0,
            thread_menu: false,
            row_menu: None,
            tab_menu: None,
            tab_menu_at: None,
            settings_menu: false,
            settings: Default::default(),
            preferences: Default::default(),
            keybindings_watcher: Default::default(),
            thread_defaults: Default::default(),
            fleet: Default::default(),
            row_menu_at: None,
            rename_thread: None,
            rename_edit: TextEdit::default(),
            delete_thread: None,
            delete_roots: vec![],
            confirm_remove_attachment: None,
            answer_remove_scope: None,
            pending_context_imports: Vec::new(),
            context_import_rxs: Vec::new(),
            project_scope: ProjectScope::All,
            scope_additional: vec![],
            selected_threads: vec![],
            selection_anchor: None,
            shelves: [false; 3],
            settled_visible: 10,
            needs_you_page: 0,
            undo: None,
            toasts: Vec::new(),
            next_toast_id: 1,
            composer_notices: crate::composer_notices::Store::default(),
            fresh_arm_seq: 0,
            keep_full_history_threads: Default::default(),
            plan_menu: None,
            plan_copied: None,
            plan_expanded: Default::default(),
            unwrapped_code_blocks: Default::default(),
            code_wrap_version: 0,
            plan_save_dialog: None,
            plan_save_last_path: None,
            next_plan_save_request_id: 0,
            opened_draft: None,
            assistant_selection: None,
            citation_highlight: None,
            citation_comment: None,
            snooze_menu: None,
            snooze_anchor: None,
            snooze_at: None,
            snooze_targets: vec![],
            custom_snooze_open: false,
            snooze_edit: TextEdit::default(),
            stash: Default::default(),
            stash_menu: false,
            stash_highlight: 0,
            stash_pulse: 0,
            lifecycle_error: None,
            auto_settle_days: 3,
            default_harness: "codex".into(),
            drag_thread: None,
            drag_target: None,
            drop_rects: Default::default(),
            viewport_height: 720.0,
            viewport_width: 1280.0,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            right_panel_width: crate::right_panel::DEFAULT_WIDTH,
            right_panels: Default::default(),
            right_panel_resizing: false,
            diffs: Default::default(),
            diff_blockers: Default::default(),
            diff_details: None,
            session_notices: Default::default(),
            diff_hide_tests: false,
            diff_tree_open: false,
            diff_tree_width: None,
            diff_tree_resizing: false,
            next_diff_request_id: 0,
            pending_diff_requests: Default::default(),
            diff_rxs: Vec::new(),
            ui_store: None,
            last_sweep: 0,
            last_lifecycle_clock: 0,
            project_picker: None,
            picker_query: TextEdit::default(),
            picker_highlight: 0,
            picker_browsing: false,
            quick_switcher: false,
            quick_switcher_query: TextEdit::default(),
            quick_switcher_highlight: 0,
            show_archive: false,
            sidebar: true,
            compact_sidebar: false,
            search: TextEdit::default(),
            demo,
            wake: Arc::new(|| {}),
            store: None,
            storage_error: None,
            portraits: crate::runtime::UiResource::new(move || {
                crate::portraits::Portraits::new(
                    portrait_art
                        .get_or_init(|| crate::visuals::Artwork::load(&portrait_path))
                        .clone(),
                )
            }),
            artwork,
            attachment_cache,
            attachment_video_posters: Default::default(),
            attachment_poster_clock: Default::default(),
            attachment_drop_hover: false,
            copied_message: None,
            changed_files: Default::default(),
            transcript_preview: None,
            attachment_preview: None,
            pasted_text_names: None,
            transcript_layout: Default::default(),
            composer_ui: Default::default(),
            captain_picker: false,
            captain_picker_query: TextEdit::default(),
            captain_picker_highlight: 0,
            env_mode_picker: false,
            base_branch_picker: false,
            base_branch_query: TextEdit::default(),
            code_picker: Default::default(),
            vcs_branches: Default::default(),
            vcs_branch_rxs: Vec::new(),
            worktree_bootstrap_rxs: Vec::new(),
            worktree_setup_rxs: Vec::new(),
            placement_rxs: Vec::new(),
            checkout_rxs: Vec::new(),
            multi_model_batches: Vec::new(),
            #[cfg(test)]
            test_git_context: None,
            #[cfg(test)]
            test_connect_workspaces: Vec::new(),
            #[cfg(test)]
            test_connect_sessions: Vec::new(),
            #[cfg(test)]
            test_connect_pi: None,
            #[cfg(test)]
            test_connect_error: None,
            next_vcs_request_id: 0,
            pending_vcs_requests: Default::default(),
            captain_project: None,
            captains: Vec::new(),
            captain_scopes: Default::default(),
            repositories: Vec::new(),
            repository_bindings: Vec::new(),
            captain_repository_scopes: Default::default(),
            captain_remote_scopes: Default::default(),
            captain_homes: Default::default(),
            captain_books: Default::default(),
            captain_lifecycles: Default::default(),
            captain_report_policies: Default::default(),
            captain_reports: Vec::new(),
            captain_question_answers: Vec::new(),
            captain_recovery_modes: Default::default(),
            captain_inbox: Vec::new(),
            captain_waits: Vec::new(),
            captain_drafts: Default::default(),
            crew_pending: Default::default(),
            lifecycle_pending: Default::default(),
            promotion_pending: Default::default(),
            captain_policy_pending: Default::default(),
            captain_policy_errors: Default::default(),
            captain_recovery_pending: Default::default(),
            captain_recovery_errors: Default::default(),
            crew_question_drafts: Default::default(),
            crew_questions_captain_open: false,
            doc_discard: None,
            context_audience_crew: false,
            context_memories_expanded: false,
            context_earlier_expanded: false,
            crew_errors: Default::default(),
            memory: Arc::default(),
            captain_editor: None,
            brief: BriefUi::default(),
            settings_picker: None,
            open_option_menu: None,
            model_picker_tab: None,
            expanded_legacy_instances: Default::default(),
            model_favorites: None,
            model_favorites_migrated: false,
            thinking_drag: None,
            model_search: TextEdit::default(),
            thread_find: ThreadFindState::default(),
            sidebar_search: SidebarSearchState::default(),
            sidebar_index: Default::default(),
            sidebar_preview: Default::default(),
            device_ui_dirty: false,
            device_draft_cache: Default::default(),
            preview: SidebarPreview::default(),
            flash_tool: None,
            pending_tool_reveal: None,
            agents_filter: Default::default(),
            agents_expanded: Default::default(),
            details: DetailsCard::default(),
            mention_search: crate::ui::composer_menu::MentionSearch::default(),
            file_preview_seq: 0,
            pending_open_urls: Vec::new(),
            next_usage_limits_request_id: 0,
            pending_usage_limits_request: None,
            usage_limits_notice_key: None,
            timeline_far_from_bottom: std::collections::HashMap::new(),
            timeline_at_bottom: std::collections::HashMap::new(),
            timeline_positions: std::cell::RefCell::default(),
            updater: Default::default(),
            update_popover: false,
            update_details: false,
            restart_for_update: false,
            restart_approved: false,
            restart_notice: None,
            task_waiters: vec![],
            projects: Default::default(),
            catalog: Catalog::default(),
            codex_catalog: Default::default(),
            claude_catalog: Default::default(),
            antigravity_catalog: Default::default(),
            cursor_catalog: Default::default(),
            acp_catalogs: Default::default(),
            workspace_model_scans: Default::default(),
            harnesses: crate::harness::Registry::read(),
            provider_availability: Default::default(),
            refresh_optional_catalogs: false,
            managed_policy: if cfg!(test) {
                None
            } else {
                read_managed_policy()
            },
            runtime_enabled: false,
            durable_task_controls: false,
            task_operations_dirty: false,
            peer_task_intents_dirty: false,
            ownership_denied: false,
            runtime_stop_requested: false,
            tasks: Vec::new(),
            task_plans: Vec::new(),
            identity: crate::identity::Catalog::local(
                &crate::store::directory().unwrap_or_default(),
            ),
            execution: crate::execution::Book::default(),
            execution_command_device: None,
            task_operations: Vec::new(),
            peer_task_intents: Vec::new(),
            recovery_notice: None,
            run_marker: None,
        };
        if !workspace.as_os_str().is_empty() {
            state.projects.entries.push(Project::new(1, workspace));
            state.projects.next_id = 2;
            state.threads[0].project = Some(1);
        } else {
            state.projects.next_id = 1;
        }
        state.threads[0].native.access = state.effective_access_mode(&state.threads[0]);
        state.identity = crate::identity::Catalog::local(&state.data_dir);
        if demo {
            state.populate_demo();
        }
        state
    }
    pub fn current(&self) -> &Thread {
        &self.threads[self.active]
    }
    pub fn current_mut(&mut self) -> &mut Thread {
        &mut self.threads[self.active]
    }
    #[cfg(test)]
    pub(crate) fn use_pi_for_test(&mut self) {
        // Keep synthetic tests independent of the configured default and native CLIs.
        self.default_harness = "pi".into();
        self.current_mut().harness = "pi".into();
    }
    /// The key the current thread's right-panel state lives under: machine plus thread id, so
    /// the same numeric id on two hosts stays separate.
    pub fn panel_key(&self) -> String {
        crate::right_panel::key(&self.machines.selected, self.current().id)
    }
    /// The current thread's panel state; a default (closed, empty) one when it has none.
    pub fn right_panel(&self) -> crate::right_panel::ThreadPanel {
        let mut panel = self.right_panels.get(&self.panel_key());
        // Only a Captain home has the first tab, its Brief. A thread's own details are the
        // floating card, so the panel opens on Work there.
        if panel.view == crate::right_panel::PanelView::Overview && !self.has_brief_tab() {
            panel.view = crate::right_panel::PanelView::Work;
        }
        panel
    }
    /// Whether the Inspector shows the Brief tab: the open thread is a Captain's home.
    pub fn has_brief_tab(&self) -> bool {
        let t = self.current();
        self.is_captain_home(t)
            && t.captain
                .is_some_and(|captain| captain < self.captains.len())
    }
    /// The top bar's details button: hide or show the docked card where it fits, else open
    /// or close its popover.
    pub fn details_toggle(&mut self) {
        if self.details.fits.get() {
            self.details.docked = !self.details.docked;
            self.details.popover = false;
            self.checkpoint();
        } else {
            self.details.popover = !self.details.popover;
        }
    }
    /// Whether a captain backs the current thread, so the Inspector's Work tab lists its
    /// crew: captain homes and crew, delegated or peer workers. Plain threads have none.
    pub fn thread_has_crew(&self) -> bool {
        let t = self.current();
        self.is_captain_home(t)
            || t.crew_captain.is_some()
            || t.delegated
            || t.peer_origin.is_some()
    }

    pub fn preview_allowed(&self) -> bool {
        self.drag_thread.is_none()
            && self.row_menu.is_none()
            && !self.thread_menu
            && self.tab_menu.is_none()
            && !self.quick_switcher
            && self.rename_thread.is_none()
            && self.delete_thread.is_none()
            && self.confirm_remove_attachment.is_none()
            && self.project_picker.is_none()
            && self.captain_editor.is_none()
            && self.settings_picker.is_none()
            && !self.captain_picker
            && !self.projects.clone_open
            && !self.current().native.access_picker
            && self.current().native.conversation.is_none()
            && self.plan_menu.is_none()
            && self.plan_save_dialog.is_none()
            && self.snooze_menu.is_none()
            && !self.settings_menu
            && self.settings.archive.is_none()
            && !self.machines.picker
            && !self.machines.dialog
            && !self.fleet.open
            && !self.fleet.popover
            && !self.update_popover
            && !self.env_mode_picker
            && !self.base_branch_picker
            && !self.stash_menu
            && !self.composer_ui.overflow.get()
            && self.code_picker.open.is_none()
            && self.attachment_preview.is_none()
            && self.transcript_preview.is_none()
            && self.citation_comment.is_none()
            && !self.brief.confirm_reset
            && !self.brief.profile_open
            && self.brief.recruit.is_none()
            && self.brief.moving.is_none()
            && self.brief.more.is_none()
            && self.brief.lifecycle.is_none()
            && self.doc_discard.is_none()
    }

    /// Expand the originating tool and its containing work fold, then reveal the turn.
    pub fn reveal_tool(&mut self, tool_id: String) {
        let t = self.current_mut();
        let Some(index) = t.tools.iter().position(|tool| tool.id == tool_id) else {
            return;
        };
        let message = t.tools[index].anchor.message.min(t.messages.len());
        let start = t.messages[..message]
            .iter()
            .rposition(|m| m.role != "Pi")
            .map_or(0, |i| i + 1);
        let in_live_turn = t.status.busy() && t.messages[start..].iter().all(|m| m.role == "Pi");
        if in_live_turn && t.live_work_collapsed {
            t.live_work_collapsed = false;
            t.timeline_cache.get_mut().touch_disclosure();
        }
        t.tools[index].expanded = true;
        for tool in &mut t.tools[..=index] {
            if tool.anchor.message >= start {
                tool.group_expanded = true;
            }
        }
        if let Some(work) = t.work.iter_mut().find(|w| w.start == start) {
            work.expanded = true;
        } else {
            t.work.push(AssistantWork {
                start,
                stopped: false,
                stopped_order: 0,
                expanded: true,
                elapsed: None,
                outcome: None,
            });
        }
        t.timeline_cache.get_mut().touch_message(message);
        let row = crate::ui::timeline_tool_row(t, index).unwrap_or_else(|| {
            t.messages.get(start).map_or_else(
                || format!("turn-live-{}", t.id),
                |message| format!("turn-{}-{}", t.id, message.id),
            )
        });
        self.pending_tool_reveal = Some(row);
        self.flash_tool = Some((tool_id, Instant::now()));
    }
    /// The visible Inspector tab while the panel is open.
    pub fn right_panel_active_view(&self) -> Option<crate::right_panel::PanelView> {
        let panel = self.right_panel();
        panel.open.then_some(panel.view)
    }
    /// Whether the Inspector covers the conversation column at the current viewport width.
    pub fn inspector_covers(&self) -> bool {
        let panel = self.right_panel();
        panel.maximized
            || self.viewport_width < 1050.0
            || crate::right_panel::covers(self.viewport_width - 16.0)
    }
    /// `rightPanel.toggle`: hide an open panel, show it when it has surfaces, and open the
    /// Inspector on its last tab (or Overview on a fresh panel), as KSG's
    /// `toggleRightPanel`.
    pub fn right_panel_toggle(&mut self) {
        let key = self.panel_key();
        if self.right_panels.get(&key).open || self.right_panels.get(&key).viewer.is_some() {
            self.right_panels.toggle_visibility(&key);
        } else {
            self.right_panels.show(&key);
        }
    }
    /// `diff.toggle`: KSG's `onToggleDiff`, the Diff surface's toggle.
    pub fn right_panel_toggle_diff(&mut self) {
        let key = self.panel_key();
        self.right_panels
            .toggle(&key, crate::right_panel::SingletonKind::Diff);
    }
    /// `threadPanel.toggle`: the thread details card, as in KSG Code. A Captain's home
    /// toggles its Brief tab instead.
    pub fn right_panel_toggle_overview(&mut self) {
        if !self.has_brief_tab() {
            self.details_toggle();
            return;
        }
        let key = self.panel_key();
        if self.right_panel_active_view() == Some(crate::right_panel::PanelView::Overview) {
            self.right_panels.close(&key);
        } else {
            self.right_panels
                .open(&key, crate::right_panel::SingletonKind::Overview);
        }
    }
    /// `rightPanel.toggleMaximized`: KSG gates it on the panel being open.
    pub fn right_panel_toggle_maximized(&mut self) {
        let key = self.panel_key();
        let panel = self.right_panels.get(&key);
        if panel.open {
            self.right_panels.set_maximized(&key, !panel.maximized);
        }
    }
    /// `rightPanel.close`: closes the open viewer first, then the panel. Returns false
    /// when neither exists so the shortcut can fall through, as KSG's handler does.
    pub fn right_panel_close_active(&mut self) -> bool {
        let key = self.panel_key();
        let panel = self.right_panels.get(&key);
        if panel.viewer.is_some() {
            self.request_close_viewer(&key);
            return true;
        }
        if panel.open {
            self.right_panels.close(&key);
            return true;
        }
        false
    }
    pub fn expire_undo(&mut self) -> bool {
        let at = lifecycle::now();
        let Some(notice) = &self.undo else {
            return false;
        };
        if notice.expires <= at {
            self.undo = None;
            self.deleted_threads.clear();
            self.deleted_records = None;
            return false;
        }
        true
    }
    pub fn undo_key(&mut self, event: &rune::KeyEvent, text_field: bool) -> bool {
        let at = lifecycle::now();
        let Some(notice) = &mut self.undo else {
            return false;
        };
        if notice.expires <= at {
            self.undo = None;
            self.deleted_threads.clear();
            self.deleted_records = None;
            return false;
        }
        if event.is_command('z') && !event.mods.shift && (!text_field || notice.owns_text_key) {
            self.undo_thread_action();
            return true;
        }
        if !matches!(event.key, rune::Key::Modifier { .. }) {
            notice.owns_text_key = false;
        }
        false
    }
    /// Plain navigation, as KSG Code's ghost tab: an already-open tab gets
    /// focus; a new thread replaces the current ghost in place, or opens as a
    /// fresh ghost right after the active kept tab.
    pub fn show_thread(&mut self, index: usize) {
        self.show_thread_inner(index, None);
    }
    pub(super) fn show_thread_inner(&mut self, index: usize, anchor: Option<Option<usize>>) {
        if index >= self.threads.len() {
            return;
        }
        let id = self.threads[index].id;
        if !self.tabs_enabled {
            // The strip is off: `focus_thread` collapses `tabs` to the open
            // thread's one slot and pushes the history entry.
            self.tab_epoch += 1;
            self.requested_tab = None;
            self.requested_tab_restore = None;
            self.brief.record = None;
            self.focus_thread(index);
            return;
        }
        if !self.tabs.contains(&id) {
            let active = self.threads.get(self.active).map_or(0, |t| t.id);
            if let Some(ghost) = self
                .ghost_tab
                .and_then(|ghost| self.tabs.iter().position(|tab| *tab == ghost))
            {
                self.tabs[ghost] = id;
            } else {
                insert_tab(&mut self.tabs, active, id);
            }
            self.ghost_tab = Some(id);
            self.ghost_anchor = anchor.unwrap_or(None);
        } else if let Some(anchor) = anchor
            && self.ghost_tab == Some(id)
        {
            // Only the ghost tab takes an anchor; a kept worker re-focusing
            // leaves another ghost's anchor alone.
            self.ghost_anchor = anchor;
        }
        self.tab_epoch += 1;
        self.requested_tab = None;
        self.requested_tab_restore = None;
        self.brief.record = None;
        self.focus_thread(index);
    }
    /// An explicit open: the tab is kept, never the ghost slot.
    pub fn open_kept_tab(&mut self, index: usize) {
        if index >= self.threads.len() {
            return;
        }
        let active = self.threads.get(self.active).map_or(0, |t| t.id);
        let id = self.threads[index].id;
        if self.tabs_enabled {
            insert_tab(&mut self.tabs, active, id);
            if self.ghost_tab == Some(id) {
                self.ghost_tab = None;
                self.ghost_anchor = None;
            }
        }
        self.tab_epoch += 1;
        self.requested_tab = None;
        self.requested_tab_restore = None;
        self.brief.record = None;
        self.focus_thread(index);
    }
    /// A worker opened from its captain sits right after the captain's tab as
    /// a ghost, and closing it returns focus to the captain.
    pub fn show_worker_ghost(&mut self, index: usize) {
        let anchor = self
            .threads
            .get(index)
            .and_then(|t| t.crew_captain)
            .and_then(|captain| self.captain_home(captain));
        // When the captain itself is the ghost (opened from the sidebar but
        // never kept), the worker's ghost slot would replace it and closing
        // could never return to it. Keep the captain's tab first.
        if let Some(captain_id) = anchor {
            self.keep_tab(captain_id);
        }
        self.show_thread_inner(index, Some(anchor));
        // Only a fresh ghost moves next to its captain; a worker that already
        // holds a kept tab keeps its place in the strip.
        let id = self.threads.get(index).map(|t| t.id);
        if let (Some(id), Some(anchor_id)) = (id, anchor)
            && self.ghost_tab == Some(id)
            && let Some(position) = self.tabs.iter().position(|tab| *tab == id)
            && let Some(anchor_pos) = self.tabs.iter().position(|tab| *tab == anchor_id)
            && position != anchor_pos + 1
        {
            self.tabs.remove(position);
            let anchor_pos = self
                .tabs
                .iter()
                .position(|tab| *tab == anchor_id)
                .unwrap_or(self.tabs.len() - 1);
            self.tabs.insert((anchor_pos + 1).min(self.tabs.len()), id);
        }
    }
    /// A send, a double-click or "Keep tab" makes the ghost permanent.
    pub fn keep_tab(&mut self, id: usize) {
        if !self.tabs_enabled {
            return;
        }
        if self.ghost_tab == Some(id) {
            self.ghost_tab = None;
            self.ghost_anchor = None;
            self.tab_epoch += 1;
            self.checkpoint();
        }
    }
    pub fn is_ghost_tab(&self, id: usize) -> bool {
        self.ghost_tab == Some(id)
    }
    /// The tab menu's bulk closes, as KSG Code's thread tab menu.
    pub fn close_other_tabs(&mut self, id: usize) {
        for tab in self.tabs.clone() {
            if tab != id {
                self.close_tab(tab);
            }
        }
        if let Some(index) = self.threads.iter().position(|t| t.id == id) {
            self.show_thread(index);
        }
    }
    pub fn close_tabs_to_right(&mut self, id: usize) {
        let Some(position) = self.tabs.iter().position(|tab| *tab == id) else {
            return;
        };
        for tab in self.tabs[position + 1..].to_vec() {
            self.close_tab(tab);
        }
    }
    pub fn close_all_tabs(&mut self) {
        for tab in self.tabs.clone() {
            self.close_tab(tab);
        }
    }
    /// Removing a tab's ghost bookkeeping without touching the strip.
    pub(super) fn drop_ghost(&mut self, id: usize) {
        if self.ghost_tab == Some(id) {
            self.ghost_tab = None;
            self.ghost_anchor = None;
        }
    }
    /// Focus an existing tab. Background reducers must not open tabs.
    pub(super) fn focus_thread(&mut self, index: usize) {
        if !self.tabs_enabled
            && let Some(id) = self.threads.get(index).map(|t| t.id)
            && self.tabs != [id]
        {
            // The strip is off: one slot, the open thread's.
            self.tabs.clear();
            self.tabs.push(id);
            self.ghost_tab = None;
            self.ghost_anchor = None;
        }
        if !self.nav_open
            && let Some(thread) = self.threads.get(index)
        {
            let id = thread.id;
            let machine = self.machines.selected.clone();
            // A launch leaves history empty; the thread open before this focus
            // is where Back should land, so seed it first.
            if self.nav_history.is_empty()
                && let Some(current) = self.threads.get(self.active).map(|t| t.id)
                && current != id
            {
                self.nav_history.push(&machine, current);
            }
            self.nav_history.push(&machine, id);
        }
        self.focus_tab(index, true);
    }
    pub(crate) fn focus_tab(&mut self, index: usize, visit: bool) {
        debug_assert!(self.tabs.contains(&self.threads[index].id));
        let previous = self.active;
        if previous != index && previous < self.threads.len() {
            self.threads[previous].draft.focused = false;
            self.threads[previous].cancel_questionnaire_advance();
        }
        self.active = index;
        // A retired record's sheet belongs to the thread it was opened from.
        self.brief.record = None;
        self.brief.confirm_reset = false;
        self.brief.recruit = None;
        self.brief.more = None;
        self.plan_menu = None;
        self.env_mode_picker = false;
        self.base_branch_picker = false;
        self.assistant_selection = None;
        self.citation_highlight = None;
        self.citation_comment = None;
        if visit && !self.demo {
            self.forward("thread.visit", json!({"threadId":self.current().id}));
        }
        self.ensure_vcs_branches(false);
        self.selected_threads.clear();
        let id = self.current().id;
        if self.thread_find.open
            && (self.thread_find.server != self.machines.selected
                || self.thread_find.thread_id != id)
        {
            self.close_thread_find();
        }
        if self.opened_draft.map(|(opened, _)| opened) != Some(id) {
            self.capture_opened_draft();
        }
        let current_key = (
            self.machines.selected.clone(),
            id,
            self.current().harness.clone(),
        );
        if self.usage_limits_notice_key.as_ref() != Some(&current_key) {
            self.usage_limits_notice_key = None;
            self.composer_notices.remove("usage-limits");
        }
        self.current_mut().life.last_visited_at = Some(lifecycle::now());
        if !self.headless {
            self.composer_thread_opened();
        }
        if !self.in_scope(self.current()) {
            self.project_scope = ProjectScope::All;
            self.scope_additional.clear();
        }
        self.sync_fresh_session_notice();
        self.sync_fallback_model(lifecycle::now() as i64);
        self.checkpoint();
    }
    /// Closing a blank draft also deletes it. Written drafts survive without a tab.
    pub fn close_tab(&mut self, id: usize) {
        if !self.tabs_enabled {
            return;
        }
        if !self.tabs.contains(&id) {
            return;
        }
        let Some(index) = self.threads.iter().position(|t| t.id == id) else {
            return;
        };
        if self.threads[index].blank_draft() {
            if self.threads[index].client_draft {
                self.remove_threads(&std::collections::HashSet::from([id]));
                return;
            }
            if self.forward("thread.delete", json!({"threadId":id,"blankOnly":true})) {
                return;
            }
            self.remove_threads(&std::collections::HashSet::from([id]));
            return;
        }
        if self.tabs.len() <= 1 {
            return;
        }
        self.remove_tab(id, false);
        self.checkpoint();
    }
    /// Whether the named thread still exists on the named machine.
    pub(super) fn thread_exists_on(&self, server: &str, id: usize) -> bool {
        thread_exists_on(&self.threads, &self.machines, server, id)
    }
    /// `navigation.back`: open the entry before the cursor, skipping and
    /// pruning entries whose threads no longer exist.
    pub fn nav_back(&mut self) {
        loop {
            let Some((server, id)) = self.nav_history.back().cloned() else {
                return;
            };
            if !self.thread_exists_on(&server, id) {
                // Drop the dead entry and every entry beyond it, then try the
                // new back target.
                let threads = &self.threads;
                let machines = &self.machines;
                self.nav_history
                    .retain(|server, id| thread_exists_on(threads, machines, server, id));
                continue;
            }
            self.nav_history.step_back();
            self.nav_open = true;
            self.open_machine_thread(&server, id);
            self.nav_open = false;
            return;
        }
    }
    /// `navigation.forward`: open the entry after the cursor.
    pub fn nav_forward(&mut self) {
        loop {
            let Some((server, id)) = self.nav_history.forward().cloned() else {
                return;
            };
            if !self.thread_exists_on(&server, id) {
                let threads = &self.threads;
                let machines = &self.machines;
                self.nav_history
                    .retain(|server, id| thread_exists_on(threads, machines, server, id));
                continue;
            }
            self.nav_history.step_forward();
            self.nav_open = true;
            self.open_machine_thread(&server, id);
            self.nav_open = false;
            return;
        }
    }
    pub fn project(&self, id: Option<ProjectId>) -> Option<&Project> {
        self.projects.entries.iter().find(|p| Some(p.id) == id)
    }
}
