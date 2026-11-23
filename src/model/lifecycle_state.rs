use super::*;

impl State {
    pub fn remove_project(&mut self, id: ProjectId) {
        if self.forward("project.remove", json!({"projectId":id})) {
            return;
        }
        if let Some(project) = self.projects.entries.iter_mut().find(|p| p.id == id) {
            project.hidden = true;
        }
        self.checkpoint();
    }
    pub fn begin_rename(&mut self, id: usize) {
        if let Some(thread) = self.threads.iter().find(|t| t.id == id) {
            self.rename_edit.replace_text(&thread.title);
            self.rename_edit.focused = true;
            self.rename_thread = Some(id);
            self.row_menu = None;
            self.thread_menu = false;
        }
    }
    pub fn save_thread_name(&mut self) {
        if self.client.is_some() {
            if let Some(id) = self.rename_thread {
                self.forward(
                    "thread.rename",
                    json!({"threadId":id,"title":self.rename_edit.text}),
                );
                self.rename_thread = None;
            }
            return;
        }
        let name = self.rename_edit.text.trim();
        if name.is_empty() || name.chars().any(char::is_control) {
            return;
        }
        if let Some(thread) = self
            .threads
            .iter_mut()
            .find(|t| Some(t.id) == self.rename_thread)
        {
            thread.title = name.chars().take(200).collect();
            thread.renamed = true;
        }
        self.rename_thread = None;
        self.checkpoint();
    }
    pub fn request_delete(&mut self, id: usize) {
        self.delete_roots = if self.selected_threads.contains(&id) {
            self.selected_threads.clone()
        } else {
            vec![id]
        };
        self.delete_thread = Some(id);
        self.row_menu = None;
        self.thread_menu = false;
        if !self.preferences.confirm_delete {
            self.confirm_delete();
        }
    }
    pub fn deletion_ids(&self, id: usize) -> std::collections::HashSet<usize> {
        let mut ids = std::collections::HashSet::from([id]);
        loop {
            let before = ids.len();
            for thread in &self.threads {
                if thread.parent.is_some_and(|parent| ids.contains(&parent)) {
                    ids.insert(thread.id);
                }
            }
            if ids.len() == before {
                return ids;
            }
        }
    }
    /// Explicit confirmation deletes the thread and its delegated descendants, never files.
    pub fn confirm_delete(&mut self) {
        if self.client.is_some() {
            let roots = if self.delete_roots.is_empty() {
                self.delete_thread.into_iter().collect()
            } else {
                std::mem::take(&mut self.delete_roots)
            };
            let local: std::collections::HashSet<_> = roots
                .iter()
                .copied()
                .filter(|id| self.threads.iter().any(|t| t.id == *id && t.client_draft))
                .collect();
            if !local.is_empty() {
                self.delete_with_undo(local.clone(), Action::Delete);
            }
            let roots: Vec<_> = roots.into_iter().filter(|id| !local.contains(id)).collect();
            if let Some(id) = roots.first() {
                self.forward("thread.delete", json!({"threadId":id,"threadIds":roots}));
            }
            self.delete_thread = None;
            return;
        }
        let Some(id) = self.delete_thread.take() else {
            return;
        };
        let roots = if self.delete_roots.is_empty() {
            vec![id]
        } else {
            std::mem::take(&mut self.delete_roots)
        };
        let ids: std::collections::HashSet<_> = roots
            .into_iter()
            .flat_map(|id| self.deletion_ids(id))
            .collect();
        self.delete_with_undo(ids, Action::Delete);
    }
    pub(super) fn delete_with_undo(
        &mut self,
        ids: std::collections::HashSet<usize>,
        action: Action,
    ) {
        if let Some(error) = ids
            .iter()
            .find_map(|id| self.require_active_thread_owner(*id).err())
        {
            self.lifecycle_error = Some(error);
            return;
        }
        self.deleted_threads.clear();
        self.deleted_records = Some(DeletedRecords {
            tasks: self
                .tasks
                .iter()
                .filter(|task| {
                    ids.contains(&task.parent_thread_id) || ids.contains(&task.child_thread_id)
                })
                .cloned()
                .collect(),
            reports: self
                .captain_reports
                .iter()
                .filter(|report| ids.contains(&report.thread_id))
                .cloned()
                .collect(),
            answers: self
                .captain_question_answers
                .iter()
                .filter(|answer| ids.contains(&answer.thread_id))
                .cloned()
                .collect(),
            events: self
                .captain_inbox
                .iter()
                .filter(|event| event.thread_id.is_some_and(|t| ids.contains(&t)))
                .cloned()
                .collect(),
            homes: self
                .captain_homes
                .iter()
                .filter(|(_, home)| ids.contains(home))
                .map(|(captain, home)| (*captain, *home))
                .collect(),
            projects: self
                .projects
                .entries
                .iter()
                .filter_map(|project| {
                    project
                        .captain_thread
                        .filter(|id| ids.contains(id))
                        .map(|home| (project.id, home))
                })
                .collect(),
        });
        self.undo = None;
        for id in &ids {
            if let Some(index) = self.threads.iter().position(|t| t.id == *id) {
                self.remember(Action::Delete, index);
            }
        }
        self.remove_threads(&ids);
        if let Some(notice) = &mut self.undo {
            notice.action = action;
        }
    }
    /// Removes threads and their records, then selects the survivor and saves. The caller
    /// handles undo and tab refusals.
    pub(super) fn remove_threads(&mut self, ids: &std::collections::HashSet<usize>) {
        if let Some(error) = ids
            .iter()
            .find_map(|id| self.require_active_thread_owner(*id).err())
        {
            self.lifecycle_error = Some(error);
            return;
        }
        let current = self.current().id;
        let local_only = self.client.is_some()
            && self
                .threads
                .iter()
                .filter(|t| ids.contains(&t.id))
                .all(|t| t.client_draft);
        let project = self.current().project;
        let seed = self.composer_seed();
        let neighbour = self
            .tabs
            .iter()
            .position(|id| *id == current)
            .and_then(|position| {
                self.tabs[position + 1..]
                    .iter()
                    .find(|id| !ids.contains(id))
                    .or_else(|| {
                        self.tabs[..position]
                            .iter()
                            .rev()
                            .find(|id| !ids.contains(id))
                    })
                    .copied()
            });
        let mut parents = Vec::new();
        let mut panel_keys = Vec::new();
        for thread in &mut self.threads {
            if ids.contains(&thread.id) {
                panel_keys.push(crate::right_panel::key(&thread.machine, thread.id));
                thread.crew_report_run = None;
                let _ = thread.stop_task();
                thread.disconnect();
                if let Some(parent) = thread.parent.filter(|id| !ids.contains(id)) {
                    parents.push((parent, thread.id));
                }
                if self.runtime_enabled
                    && let Ok(server) = crate::mcp::server(self.wake.clone())
                {
                    server.revoke(thread.id);
                }
            }
        }
        let mut removed = Vec::new();
        self.threads.retain_mut(|thread| {
            if ids.contains(&thread.id) {
                // Keep the disconnected in-memory transcript for the five-second delete undo.
                if self.undo.as_ref().is_some_and(|n| {
                    n.action == Action::Delete && n.entries.iter().any(|e| e.id == thread.id)
                }) {
                    removed.push(std::mem::replace(thread, Thread::new(0)));
                }
                false
            } else {
                true
            }
        });
        self.deleted_threads.extend(removed);
        // Panel state is per thread: a deleted thread's goes with it.
        for key in panel_keys {
            self.right_panels.forget(&key);
        }
        self.captain_homes.retain(|_, home| !ids.contains(home));
        self.sync_memory_links();
        self.tasks.retain(|task| {
            !ids.contains(&task.parent_thread_id) && !ids.contains(&task.child_thread_id)
        });
        let removed_reports: std::collections::HashSet<String> = self
            .captain_reports
            .iter()
            .filter(|report| ids.contains(&report.thread_id))
            .map(|report| report.id.clone())
            .collect();
        self.captain_reports
            .retain(|report| !ids.contains(&report.thread_id));
        let removed_answers: std::collections::HashSet<String> = self
            .captain_question_answers
            .iter()
            .filter(|answer| ids.contains(&answer.thread_id))
            .map(|answer| answer.id.clone())
            .collect();
        self.captain_question_answers
            .retain(|answer| !ids.contains(&answer.thread_id));
        let removed_events: std::collections::HashSet<String> = self
            .captain_inbox
            .iter()
            .filter(|event| event.thread_id.is_some_and(|t| ids.contains(&t)))
            .map(|event| event.id.clone())
            .collect();
        self.captain_inbox
            .retain(|event| !event.thread_id.is_some_and(|t| ids.contains(&t)));
        let removed_deliveries: std::collections::HashSet<&str> = removed_reports
            .iter()
            .map(String::as_str)
            .chain(removed_answers.iter().map(String::as_str))
            .chain(removed_events.iter().map(String::as_str))
            .collect();
        for thread in &mut self.threads {
            thread.pending_injections.retain(|injection| {
                !tasks::delivery_id(&injection.text)
                    .is_some_and(|id| removed_deliveries.contains(id))
            });
        }
        self.tabs.retain(|id| !ids.contains(id));
        if self.ghost_tab.is_some_and(|ghost| ids.contains(&ghost)) {
            self.ghost_tab = None;
            self.ghost_anchor = None;
        }
        for project in &mut self.projects.entries {
            if project.captain_thread.is_some_and(|id| ids.contains(&id)) {
                project.captain_thread = None;
            }
        }
        for (parent, id) in parents {
            if let Some(thread) = self.threads.iter_mut().find(|t| t.id == parent) {
                thread.enqueue_injection(format!(
                    "Shina: task #{id} was deleted by the user. Status: cancelled."
                ));
            }
        }
        let waiters = std::mem::take(&mut self.task_waiters);
        for waiter in waiters {
            if ids.contains(&waiter.thread) {
                let _ = waiter
                    .reply
                    .send(Err("The task was deleted by the user.".into()));
            } else {
                self.task_waiters.push(waiter);
            }
        }
        self.active = self
            .threads
            .iter()
            .position(|t| t.id == current)
            .or_else(|| self.threads.iter().position(|t| Some(t.id) == neighbour))
            .unwrap_or(0);
        if ids.contains(&current) && neighbour.is_none() {
            if self.client.is_some() {
                self.new_client_draft_with_seed(project, &seed);
            } else {
                self.create_thread_in(project, &[], &seed);
            }
        } else if ids.contains(&current) {
            self.tab_epoch += 1;
            self.focus_tab(self.active, !local_only);
        }
        self.checkpoint();
    }
    pub fn pin_thread(&mut self, id: usize) {
        if self.lifecycle_blocker(id).is_some() {
            return;
        }
        if self.forward("thread.lifecycle", json!({"threadId":id,"action":"pin"})) {
            return;
        }
        if self
            .threads
            .iter()
            .any(|t| t.id == id && self.is_captain_home(t))
        {
            return;
        }
        let Some(index) = self
            .threads
            .iter()
            .position(|t| t.id == id && t.machine == "local")
        else {
            return;
        };
        if self.threads[index].pinned() {
            self.remember(Action::Unpin, index);
            self.threads[index].life.pinned_at = None;
            self.threads[index].life.pin_order_key = None;
        } else {
            self.invalidate_undo(id);
            let key = self.first_pin_key(id);
            self.threads[index].life.pin(lifecycle::now(), Some(key));
        }
        self.threads[index].working = Working::default();
        self.checkpoint();
    }
    pub fn archive_thread(&mut self, id: usize) {
        if !self.headless
            && self.preferences.confirm_archive
            && self.threads.iter().any(|t| t.id == id && !t.archived())
        {
            if self.settings.archive.is_none() {
                self.settings.archive_targets.clear();
            }
            if !self.settings.archive_targets.contains(&id) {
                self.settings.archive_targets.push(id);
            }
            self.settings.archive = Some(id);
            return;
        }
        self.archive_confirmed(id);
    }
    pub fn archive_confirmed(&mut self, id: usize) {
        self.archive_thread_record(id, false);
    }
    pub(super) fn archive_thread_record(&mut self, id: usize, keep_blank: bool) {
        if self
            .threads
            .iter()
            .any(|t| t.id == id && t.client_draft && t.blank_draft())
        {
            self.delete_with_undo(std::collections::HashSet::from([id]), Action::Archive);
            return;
        }
        if self.forward(
            "thread.lifecycle",
            json!({"threadId":id,"action":"archive","keepBlank":keep_blank}),
        ) {
            return;
        }
        let Some(index) = self
            .threads
            .iter()
            .position(|t| t.id == id && t.machine == "local")
        else {
            return;
        };
        if let Err(error) = self.require_active_thread_owner(id) {
            self.lifecycle_error = Some(error);
            return;
        }
        if self.lifecycle_busy(id) {
            self.lifecycle_error = Some("Stop this thread and its crew before archiving.".into());
            return;
        }
        // A blank draft has nothing to keep, so archiving it never leaves an archive row.
        if self.threads[index].blank_draft() && !keep_blank {
            self.delete_with_undo(std::collections::HashSet::from([id]), Action::Archive);
            return;
        }
        if self.threads[index].archived() {
            self.invalidate_undo(id);
            self.threads[index].life.archived_at = None;
        } else {
            self.remember(Action::Archive, index);
            self.threads[index].life.archived_at = Some(lifecycle::now());
            self.threads[index].disconnect();
            self.remove_tab(id, false);
        }
        self.threads[index].working = Working::default();
        self.checkpoint();
    }
    pub(super) fn first_pin_key(&mut self, except: usize) -> String {
        let mut pins: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.pinned() && t.id != except)
            .map(|(i, _)| i)
            .collect();
        pins.sort_by(|a, b| {
            lifecycle::compare(
                Section::Pinned,
                &self.threads[*a].life,
                &self.threads[*b].life,
                0,
                0,
            )
        });
        let after = pins
            .first()
            .and_then(|i| self.threads[*i].life.pin_order_key.as_deref());
        if let Some(key) = lifecycle::pin_key_between(None, after) {
            return key;
        }
        let keys = lifecycle::spread_pin_keys(pins.len() + 1);
        for (index, key) in pins.into_iter().zip(keys.iter().skip(1)) {
            self.threads[index].life.pin_order_key = Some(key.clone());
        }
        keys[0].clone()
    }
    pub fn lifecycle_busy(&self, id: usize) -> bool {
        self.has_remote_captain_turn(id)
            || self.threads.iter().any(|t| {
                (t.id == id || t.parent == Some(id))
                    && (t.status.busy() || t.request.is_some() || t.configuring || t.queued())
            })
    }
    pub fn conversation_blocker(&self, thread: &Thread) -> Option<String> {
        if self.demo {
            Some("Demo data".into())
        } else if thread.archived() {
            Some("Archived".into())
        } else if thread.configuring || thread.initializing {
            Some("Still starting".into())
        } else if thread.status.busy() {
            Some("While working".into())
        } else if !thread.can_change_conversation() {
            Some(if thread.harness == "codex" {
                "Not supported by Codex".into()
            } else {
                "Not supported by this provider".into()
            })
        } else if thread.session.is_none() {
            Some("No earlier message yet".into())
        } else {
            None
        }
    }

    pub fn rewind_blocker(&self, thread: &Thread) -> Option<String> {
        self.conversation_blocker(thread).or_else(|| {
            if !thread.can_rewind_conversation() {
                Some("Not supported by Codex".into())
            } else if !thread
                .messages
                .iter()
                .any(|message| message.role == "Shina")
                && thread.life.latest_user_message_at.is_none()
            {
                Some("No earlier message yet".into())
            } else {
                None
            }
        })
    }

    pub fn lifecycle_blocker(&self, id: usize) -> Option<String> {
        let Some(t) = self.threads.iter().find(|t| t.id == id) else {
            return Some("Thread unavailable".into());
        };
        if t.machine != "local" {
            Some("Remote thread".into())
        } else if self.is_captain_home(t) {
            Some("Captain home".into())
        } else if !t.life.settled() && self.lifecycle_busy(id) {
            Some("While working".into())
        } else {
            None
        }
    }

    pub fn snooze_blocker(&self, id: usize) -> Option<String> {
        let Some(t) = self.threads.iter().find(|t| t.id == id) else {
            return Some("Thread unavailable".into());
        };
        if t.machine != "local" {
            Some("Remote thread".into())
        } else if t.life.snoozed(lifecycle::now()) {
            Some("Already snoozed".into())
        } else if self.row_pending(t) {
            Some("Needs your reply first".into())
        } else if self.is_captain_home(t) {
            Some("Captain home".into())
        } else if t.archived() {
            Some("Archived".into())
        } else if t.queued() {
            Some("While working".into())
        } else {
            None
        }
    }

    pub fn unread_blocker(&self, thread: &Thread) -> Option<String> {
        thread
            .life
            .last_completed_at
            .is_none()
            .then(|| "No finished reply yet".into())
    }
    pub(super) fn invalidate_undo(&mut self, id: usize) {
        if let Some(notice) = &mut self.undo {
            notice.entries.retain(|e| e.id != id);
        }
        if self.undo.as_ref().is_some_and(|n| n.entries.is_empty()) {
            self.undo = None;
        }
    }
    pub(super) fn remember(&mut self, action: Action, index: usize) {
        if action != Action::Delete {
            self.deleted_threads.clear();
            self.deleted_records = None;
        }
        let t = &self.threads[index];
        let mut steps = lifecycle::plan_undo(action, &t.life, &t.draft.text);
        if action == Action::Discard && !t.draft_images.is_empty() {
            steps.push(UndoStep::RestoreImages(t.draft_images.clone()));
        }
        if matches!(
            action,
            Action::Settle | Action::Snooze | Action::Archive | Action::Discard | Action::Delete
        ) && let Some(position) = self.tabs.iter().position(|id| *id == t.id)
        {
            steps.push(UndoStep::RestoreTab(position, self.current().id == t.id));
        }
        UndoNotice::push(&mut self.undo, action, t.id, steps, lifecycle::now());
    }
    pub fn settle_thread(&mut self, id: usize) {
        if let Some(reason) = self.lifecycle_blocker(id) {
            self.lifecycle_error = Some(reason);
            return;
        }
        if self.forward("thread.lifecycle", json!({"threadId":id,"action":"settle"})) {
            return;
        }
        if self
            .threads
            .iter()
            .any(|t| t.id == id && self.is_captain_home(t))
        {
            return;
        }
        let Some(i) = self
            .threads
            .iter()
            .position(|t| t.id == id && t.machine == "local")
        else {
            return;
        };
        if self.threads[i].life.settled() {
            self.invalidate_undo(id);
            self.threads[i].life.unsettle(lifecycle::now(), true);
        } else {
            self.remember(Action::Settle, i);
            self.threads[i].life.settle(lifecycle::now());
            self.threads[i].disconnect();
            self.advance_parked(id);
        }
        self.threads[i].working = Working::default();
        self.checkpoint();
    }
    pub(super) fn advance_parked(&mut self, id: usize) {
        self.remove_tab(id, true);
    }
    /// Ported from KSG Code's Sidebar settle advance and useThreadActions archive fallback.
    /// Existing tabs take precedence. Only the last tab can open a replacement.
    pub(super) fn remove_tab(&mut self, id: usize, settle: bool) {
        if !self.tabs.contains(&id) {
            return;
        }
        let anchor = if self.ghost_tab == Some(id) {
            self.ghost_anchor.take()
        } else {
            None
        };
        self.drop_ghost(id);
        if self.current().id != id {
            self.tabs.retain(|tab| *tab != id);
            return;
        }
        self.tab_epoch += 1;
        // A worker ghost returns focus to the captain it was opened from.
        if let Some(anchor) = anchor
            && self.tabs.contains(&anchor)
            && let Some(index) = self.threads.iter().position(|t| t.id == anchor)
        {
            self.tabs.retain(|tab| *tab != id);
            self.focus_thread(index);
            return;
        }
        if let Some(neighbour) = tab_neighbour(&self.tabs, id)
            && let Some(index) = self.threads.iter().position(|t| t.id == neighbour)
        {
            self.tabs.retain(|tab| *tab != id);
            self.focus_thread(index);
            return;
        }
        let project = self.current().project;
        let order = self.sidebar_threads();
        let start = order
            .iter()
            .position(|i| self.threads[*i].id == id)
            .unwrap_or(0);
        for offset in 1..=if settle { order.len() } else { 0 } {
            let i = order[(start + offset) % order.len()];
            let t = &self.threads[i];
            if self.valid_after_park(t, id) {
                self.tabs.retain(|tab| *tab != id);
                self.show_thread(i);
                return;
            }
        }
        if self.client.is_some() {
            self.tabs.retain(|tab| *tab != id);
            self.new_client_draft(project);
            return;
        }
        self.tabs.retain(|tab| *tab != id);
        self.new_thread_in(project);
    }
    pub(super) fn valid_after_park(&self, t: &Thread, parked: usize) -> bool {
        t.id != parked
            && !t.archived()
            && !matches!(
                self.thread_section(t, lifecycle::now()),
                Section::Snoozed | Section::Settled
            )
    }
    pub fn can_snooze(&self, id: usize) -> bool {
        self.snooze_blocker(id).is_none()
    }
    pub fn open_snooze(&mut self, id: usize) {
        self.open_snooze_from(id, None, None);
    }
    pub fn open_snooze_from(&mut self, id: usize, anchor: Option<String>, at: Option<(f32, f32)>) {
        if self.snooze_blocker(id).is_some() {
            return;
        }
        self.row_menu = None;
        self.row_menu_at = None;
        self.thread_menu = false;
        self.snooze_menu = Some((
            id,
            lifecycle::snooze_choices_from(self.thread_defaults.snooze_presets()),
        ));
        self.snooze_anchor = anchor;
        self.snooze_at = at;
        self.snooze_targets = if self.selected_threads.contains(&id) {
            self.selected_threads.clone()
        } else {
            vec![id]
        };
        self.custom_snooze_open = false;
        self.lifecycle_error = None;
    }
    pub fn snooze_thread(&mut self, id: usize, until: u64) {
        if self.forward(
            "thread.lifecycle",
            json!({"threadId":id,"action":"snooze","until":until}),
        ) {
            self.snooze_menu = None;
            self.snooze_anchor = None;
            self.snooze_at = None;
            return;
        }
        if until <= lifecycle::now() || !self.can_snooze(id) {
            self.lifecycle_error =
                Some("Choose a future snooze time after answering pending requests.".into());
            return;
        }
        let i = self.threads.iter().position(|t| t.id == id).unwrap();
        self.remember(Action::Snooze, i);
        self.threads[i].life.snooze(lifecycle::now(), until);
        self.threads[i].working = Working::default();
        self.snooze_menu = None;
        self.snooze_anchor = None;
        self.snooze_at = None;
        self.advance_parked(id);
        self.checkpoint();
    }
    pub fn unsnooze_thread(&mut self, id: usize) {
        if self.forward(
            "thread.lifecycle",
            json!({"threadId":id,"action":"unsnooze"}),
        ) {
            return;
        }
        self.invalidate_undo(id);
        if let Some(t) = self
            .threads
            .iter_mut()
            .find(|t| t.id == id && t.machine == "local")
        {
            t.life.unsnooze(lifecycle::now());
            t.working = Working::default();
        }
        self.checkpoint();
    }
    pub fn discard_draft(&mut self, id: usize) {
        if self.threads.iter().any(|t| t.id == id && t.client_draft) {
            self.delete_with_undo(std::collections::HashSet::from([id]), Action::Discard);
            return;
        }
        if self.client.is_some() {
            if let Some(index) = self.threads.iter().position(|t| t.id == id) {
                if self.threads[index].blank_draft() {
                    self.forward(
                        "thread.lifecycle",
                        json!({"threadId":id,"action":"discard"}),
                    );
                    return;
                }
                self.remember(Action::Discard, index);
                self.threads[index].draft.replace_text("");
                self.threads[index].draft_images.clear();
                if self.threads[index].is_draft() {
                    // Draft text belongs to this device. Keep the server record so local Undo
                    // can restore it without sending the discarded text to the server.
                    self.forward(
                        "thread.lifecycle",
                        json!({"threadId":id,"action":"archive","keepBlank":true}),
                    );
                    self.threads[index].life.archived_at = Some(lifecycle::now());
                } else {
                    self.remove_tab(id, false);
                }
                self.checkpoint();
            }
            return;
        }
        let Some(i) = self.threads.iter().position(|t| t.id == id) else {
            return;
        };
        // Keep an empty draft only in the short undo window, never as an archive row.
        if self.threads[i].blank_draft()
            && self.threads[i].messages.is_empty()
            && self.threads[i].unused_session()
        {
            self.delete_with_undo(std::collections::HashSet::from([id]), Action::Discard);
            return;
        }
        self.remember(Action::Discard, i);
        self.threads[i].draft.replace_text("");
        self.threads[i].draft_images.clear();
        self.threads[i].attachment_error = None;
        if self.threads[i].messages.is_empty() && self.threads[i].unused_session() {
            self.threads[i].life.archived_at = Some(lifecycle::now());
        }
        self.remove_tab(id, false);
        self.checkpoint();
    }
    pub(super) fn client_undo_is_local(&self) -> bool {
        self.undo.as_ref().is_some_and(|notice| {
            self.deleted_records.is_some()
                || notice.entries.iter().all(|entry| {
                    self.threads
                        .iter()
                        .any(|t| t.id == entry.id && t.client_draft)
                })
                || notice.action == Action::Discard
                    && notice.entries.iter().any(|entry| {
                        entry
                            .steps
                            .iter()
                            .any(|step| matches!(step, UndoStep::RestoreDraft(_)))
                    })
        })
    }
    pub fn undo_thread_action(&mut self) {
        let at = lifecycle::now();
        if self
            .undo
            .as_ref()
            .is_some_and(|notice| notice.expires <= at)
        {
            self.undo = None;
            self.deleted_threads.clear();
            self.deleted_records = None;
            return;
        }
        if self.client.is_some() && self.undo.is_some() && !self.client_undo_is_local() {
            self.forward("thread.undo", json!({"expected":self.undo}));
            return;
        }
        let inactive_owner = self
            .undo
            .as_ref()
            .and_then(|notice| {
                notice
                    .entries
                    .iter()
                    .find_map(|entry| self.require_active_thread_owner(entry.id).err())
            })
            .or_else(|| {
                self.deleted_threads.iter().find_map(|thread| {
                    thread
                        .crew_captain
                        .or_else(|| thread.captain_home.then_some(thread.captain).flatten())
                        .and_then(|captain| self.require_active_captain(captain).err())
                })
            });
        if let Some(error) = inactive_owner {
            self.lifecycle_error = Some(error);
            return;
        }
        let Some(notice) = self.undo.take().filter(|n| n.expires > at) else {
            return;
        };
        if self.deleted_records.is_some() {
            self.threads.append(&mut self.deleted_threads);
            if let Some(records) = self.deleted_records.take() {
                for task in records.tasks {
                    if !self.tasks.iter().any(|t| t.id == task.id)
                        && self.threads.iter().any(|t| t.id == task.parent_thread_id)
                        && self.threads.iter().any(|t| t.id == task.child_thread_id)
                    {
                        self.tasks.push(task);
                    }
                }
                for report in records.reports {
                    if self.threads.iter().any(|t| t.id == report.thread_id)
                        && !self.captain_reports.iter().any(|r| r.id == report.id)
                    {
                        self.captain_reports.push(report);
                    }
                }
                for answer in records.answers {
                    if self.threads.iter().any(|t| t.id == answer.thread_id)
                        && !self
                            .captain_question_answers
                            .iter()
                            .any(|a| a.id == answer.id)
                    {
                        self.captain_question_answers.push(answer);
                    }
                }
                for event in records.events {
                    if event
                        .thread_id
                        .is_none_or(|id| self.threads.iter().any(|t| t.id == id))
                        && !self.captain_inbox.iter().any(|e| e.id == event.id)
                    {
                        self.captain_inbox.push(event);
                    }
                }
                for (captain, home) in records.homes {
                    if self.threads.iter().any(|t| t.id == home) {
                        self.captain_homes.entry(captain).or_insert(home);
                    }
                }
                for (id, home) in records.projects {
                    if let Some(project) = self.projects.entries.iter_mut().find(|p| p.id == id)
                        && project.captain_thread.is_none()
                    {
                        project.captain_thread = Some(home);
                    }
                }
                self.sync_memory_links();
            }
        }
        let mut restore_tabs = Vec::new();
        let mut restore_drafts = Vec::new();
        let mut focus = None;
        for entry in notice.entries {
            let Some(i) = self.threads.iter().position(|t| t.id == entry.id) else {
                continue;
            };
            for step in entry.steps {
                if self.threads[i].machine != "local" {
                    break;
                }
                let t = &mut self.threads[i];
                match step {
                    UndoStep::Unsettle => t.life.unsettle(at, true),
                    UndoStep::Unsnooze => t.life.unsnooze(at),
                    UndoStep::Pin(key) => t.life.pin(at, key),
                    UndoStep::Snooze(until) if until > at => t.life.snooze(at, until),
                    UndoStep::Snooze(_) => {}
                    UndoStep::Unarchive => t.life.archived_at = None,
                    UndoStep::RestoreDraft(draft) => {
                        t.draft.replace_text(&draft);
                        t.life.archived_at = None;
                        restore_drafts.push(entry.id);
                    }
                    UndoStep::RestoreImages(images) => {
                        t.draft_images = images;
                        t.attachment_error =
                            attachments::validate_prompt_images(&t.draft_images).err();
                    }
                    UndoStep::RestoreTab(position, was_active) => {
                        restore_tabs.push((entry.id, position));
                        if was_active {
                            focus = Some(entry.id);
                        }
                    }
                }
            }
        }
        if self.client.is_some() && notice.action == Action::Discard {
            for id in restore_drafts {
                self.forward(
                    "thread.lifecycle",
                    json!({"threadId":id,"action":"unarchive"}),
                );
            }
        }
        self.restore_tabs(restore_tabs, focus);
        self.checkpoint();
    }
    pub(super) fn restore_tabs(&mut self, mut tabs: Vec<(usize, usize)>, preferred: Option<usize>) {
        tabs.sort_by_key(|(_, position)| *position);
        let mut focus = None;
        for (id, position) in tabs {
            if let Some(index) = self.threads.iter().position(|t| t.id == id) {
                self.tabs.retain(|tab| *tab != id);
                self.tabs.insert(position.min(self.tabs.len()), id);
                focus = Some(index);
            }
        }
        self.ghost_tab = self.ghost_tab.filter(|id| self.tabs.contains(id));
        if self.ghost_tab.is_none() {
            self.ghost_anchor = None;
        }
        if let Some(index) = preferred
            .and_then(|id| {
                self.threads
                    .iter()
                    .position(|t| t.id == id && self.tabs.contains(&id))
            })
            .or(focus)
        {
            self.tab_epoch += 1;
            self.focus_thread(index);
        }
    }
    pub fn drop_thread(&mut self, id: usize, to: Section, before: Option<usize>) {
        if self.forward(
            "thread.drop",
            json!({"threadId":id,"to":format!("{to:?}"),"before":before}),
        ) {
            return;
        }
        if self
            .threads
            .iter()
            .any(|t| t.id == id && self.is_captain_home(t))
        {
            return;
        }
        let Some(i) = self
            .threads
            .iter()
            .position(|t| t.id == id && t.machine == "local")
        else {
            return;
        };
        let from = self.thread_section(&self.threads[i], lifecycle::now());
        match lifecycle::plan_drop(from, to) {
            DropPlan::Pin | DropPlan::ReorderPin => {
                self.invalidate_undo(id);
                let mut pins: Vec<_> = self
                    .threads
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.pinned() && t.id != id)
                    .map(|(i, _)| i)
                    .collect();
                pins.sort_by(|a, b| {
                    lifecycle::compare(
                        Section::Pinned,
                        &self.threads[*a].life,
                        &self.threads[*b].life,
                        0,
                        0,
                    )
                });
                let place = before
                    .and_then(|id| pins.iter().position(|i| self.threads[*i].id == id))
                    .unwrap_or(pins.len());
                let a = place
                    .checked_sub(1)
                    .and_then(|p| self.threads[pins[p]].life.pin_order_key.as_deref());
                let b = pins
                    .get(place)
                    .and_then(|p| self.threads[*p].life.pin_order_key.as_deref());
                let usable = (place == 0 || a.is_some()) && (place == pins.len() || b.is_some());
                let key = usable.then(|| lifecycle::pin_key_between(a, b)).flatten();
                self.threads[i].life.pin(lifecycle::now(), key.clone());
                if key.is_none() {
                    pins.insert(place, i);
                    for (i, key) in pins.into_iter().zip(lifecycle::spread_pin_keys(
                        self.threads.iter().filter(|t| t.pinned()).count(),
                    )) {
                        self.threads[i].life.pin_order_key = Some(key);
                    }
                }
            }
            DropPlan::Unpin => self.pin_thread(id),
            DropPlan::Unsettle | DropPlan::Settle => self.settle_thread(id),
            DropPlan::Wake => self.unsnooze_thread(id),
            DropPlan::None => return,
        }
        self.threads[i].working = Working::default();
        self.checkpoint();
    }
    /// Called each minute and after run-end events. PR integration can call this same hook.
    pub fn sweep_lifecycle(&mut self, at: u64) -> bool {
        self.last_sweep = at;
        let eligible: Vec<_> = self
            .threads
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                if t.is_draft() || t.machine != "local" {
                    return None;
                }
                t.life
                    .auto_settle_at(
                        at,
                        self.auto_settle_days,
                        self.lifecycle_busy(t.id),
                        t.captain_home || t.delegated || self.is_captain_home(t),
                    )
                    .map(|stamp| (i, stamp))
            })
            .collect();
        for (i, stamp) in &eligible {
            self.threads[*i].life.settle(*stamp);
            self.threads[*i].disconnect();
            self.threads[*i].working = Working::default();
        }
        !eligible.is_empty()
    }
}
