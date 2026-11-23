use super::*;

impl State {
    /// Open the in-draft captain selector (from the composer footer or sidebar Captains "+").
    /// Ported from KSG Code's new-thread / composer picker flow: starts a ready draft in the
    /// current workspace if needed, focuses the composer, and opens the in-place captain selector.
    pub fn open_captain_selector(&mut self) {
        if self.current().captain.is_some() || !self.current().is_draft() {
            let project = self.current().project;
            self.new_thread_in(project);
        }
        self.captain_project = Some(self.current().project);
        self.captain_picker = true;
        self.captain_picker_query = TextEdit::default();
        self.captain_picker_query.focused = true;
        self.captain_picker_highlight = 0;
        self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Explicit);
    }

    pub fn choose_project_captain(&mut self, id: ProjectId) {
        if let Some(captain) = self.project(Some(id)).and_then(|p| p.captain) {
            self.projects.captain_target = Some(id);
            self.open_captain(captain);
        } else {
            self.new_thread_in(Some(id));
            self.projects.captain_target = Some(id);
            self.captain_project = Some(Some(id));
            self.captain_picker = true;
            self.captain_picker_query = TextEdit::default();
            self.captain_picker_query.focused = true;
            self.captain_picker_highlight = 0;
            self.restore_composer_focus(crate::ui::composer_ui::FocusReason::Explicit);
        }
    }
    pub fn toggle_thread_auto_settle(&mut self, id: usize) {
        let disabled = self
            .threads
            .iter()
            .find(|t| t.id == id)
            .is_some_and(|t| t.life.auto_settle_disabled_at.is_none());
        if self.forward(
            "thread.lifecycle",
            json!({"threadId":id,"action":"autoSettle","disabled":disabled}),
        ) {
            return;
        }
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.life.auto_settle_disabled_at = disabled.then(lifecycle::now);
        }
        self.checkpoint();
    }
    pub fn cycle_auto_settle_days(&mut self) {
        let days = match self.auto_settle_days {
            0 => 1,
            1 => 3,
            3 => 7,
            7 => 14,
            14 => 30,
            _ => 0,
        };
        if self.forward("settings.autoSettle", json!({"days":days})) {
            return;
        }
        self.auto_settle_days = days;
        self.sweep_lifecycle(lifecycle::now());
        self.checkpoint();
    }
    pub fn new_thread(&mut self) {
        let project = self.current().project;
        self.new_thread_in(project);
    }
    pub fn new_thread_in(&mut self, project: Option<ProjectId>) {
        if self.client.is_some() {
            self.new_client_draft(project);
            return;
        }
        let seed = self.composer_seed();
        self.create_thread_in(project, &[], &seed);
    }
    // Ported from KSG Code's useHandleNewThread composer model override.
    pub(crate) fn composer_seed(&self) -> Value {
        self.threads.get(self.active).map_or(Value::Null, |thread| {
            json!({"harness":thread.harness,"model":thread.selected_or_effective_model(),
                "thinking":thread.selected_or_effective_reasoning(),"access":thread.native.access.id(),
                "serviceTier":thread.harness_state["serviceTier"]})
        })
    }
    pub(crate) fn inherit_composer(&mut self, index: usize, seed: &Value) {
        if seed.is_null() {
            let harness = self.default_harness.clone();
            self.threads[index].harness = harness.clone();
            self.threads[index].native.access =
                AccessMode::parse(&self.thread_defaults.access).unwrap_or(AccessMode::Supervised);
            self.threads[index].native.access = self.effective_access_mode(&self.threads[index]);
            if let Some(selection) = self.thread_defaults.models.get(&harness)
                && !selection.provider.is_empty()
                && !selection.model.is_empty()
            {
                let thread = &mut self.threads[index];
                thread.selected_model = Some((selection.provider.clone(), selection.model.clone()));
                thread.model_provider = selection.provider.clone();
                thread.model_id = selection.model.clone();
                thread.model = selection.model.clone();
                thread.selected_thinking =
                    (!selection.effort.is_empty()).then(|| selection.effort.clone());
                thread.thinking = selection.effort.clone();
            }
            return;
        }
        let Some(harness) = seed["harness"]
            .as_str()
            .filter(|id| self.validate_harness(id).is_ok())
        else {
            return;
        };
        if let Some(access) = seed["access"].as_str() {
            self.threads[index].native.access =
                AccessMode::parse(access).unwrap_or(AccessMode::Supervised);
        }
        let cwd = self.catalog_workspace_cwd(index);
        let models = self.harness_models_for_cwd(harness, cwd.as_deref());
        let selection: Option<(String, String)> =
            serde_json::from_value(seed["model"].clone()).ok().flatten();
        let Some(model) = selection.as_ref().and_then(|(provider, id)| {
            models
                .iter()
                .find(|m| &m.provider == provider && &m.id == id)
        }) else {
            self.inherit_composer(index, &Value::Null);
            // Access inheritance does not depend on model discovery. A missing model
            // must never replace the source thread's read-only mode with a broader default.
            if let Some(access) = seed["access"].as_str() {
                self.threads[index].native.access =
                    AccessMode::parse(access).unwrap_or(AccessMode::Supervised);
            }
            return;
        };
        let thread = &mut self.threads[index];
        if thread.harness != harness {
            thread.prompt_suggestion = None;
        }
        thread.harness = harness.into();
        thread.models = models.clone();
        thread.model_provider = model.provider.clone();
        thread.model_id = model.id.clone();
        thread.model = model.name.clone();
        thread.context_window = model.context_window;
        thread.supports_images = model.supports_images;
        thread.selected_model = selection;
        thread.thinking_levels = model.thinking_levels.clone();
        if let Some(level) = seed["thinking"]
            .as_str()
            .filter(|level| model.thinking_levels.iter().any(|v| v == level))
        {
            thread.selected_thinking = Some(level.into());
            thread.thinking = level.into();
        } else if !thread.thinking_levels.contains(&thread.thinking) {
            thread.selected_thinking = None;
            thread.thinking.clear();
        }
        if harness == "codex"
            && let Some(tier) = seed["serviceTier"].as_str().filter(|tier| {
                model
                    .service_tiers
                    .iter()
                    .any(|option| option["id"] == *tier)
            })
        {
            thread.harness_state["serviceTier"] = json!(tier);
        }
        thread.native.access = resolve_compatible_access_mode(
            thread.native.access,
            supported_access_modes(&thread.harness),
        )
        .1;
        let clamped = self.effective_access_mode(&self.threads[index]);
        self.threads[index].native.access = clamped;
    }
    pub(super) fn create_thread_in(
        &mut self,
        project: Option<ProjectId>,
        occupied: &[usize],
        seed: &Value,
    ) {
        if project.is_some() && self.project(project).is_none() {
            return;
        }
        // T3 keeps one draft thread per logical project and reopens it instead of adding another.
        let reuse = self.threads.iter().position(|t| {
            t.project == project
                && !t.delegated
                && !t.captain_home
                && t.crew_captain.is_none()
                && t.blank_draft()
                && !occupied.contains(&t.id)
        });
        let index = reuse.unwrap_or_else(|| {
            let mut thread = Thread::new(self.next_id);
            thread.harness = self.default_harness.clone();
            thread.project = project;
            thread.env_mode = Some(self.effective_default_thread_env_mode());
            if let Some(project) = self.project(project) {
                thread.set_machine(project.machine.clone());
            }
            self.threads.push(thread);
            self.next_id += 1;
            self.threads.len() - 1
        });
        self.show_archive = false;
        self.project_picker = None;
        if match self.project_scope {
            ProjectScope::All => false,
            ProjectScope::Project(id) => project != Some(id),
            ProjectScope::NoProject => project.is_some(),
        } {
            self.project_scope = ProjectScope::All;
            self.scope_additional.clear();
        }
        if reuse.is_none() {
            self.apply_catalog();
        }
        if reuse.is_none() || self.threads[index].selected_model.is_none() {
            self.inherit_composer(index, seed);
            if reuse.is_none() {
                self.apply_sticky_composer_to_draft(index);
            }
        }
        self.open_kept_tab(index);
        self.sync_fallback_model(lifecycle::now() as i64);
    }
    pub fn open_captain(&mut self, captain: usize) {
        if self.forward("captain.open", json!({"captain":captain,"project":self.captain_project.unwrap_or(self.current().project),"assignProject":self.projects.captain_target})) {
            self.captain_picker = false;
            return;
        }
        if self.require_captain(captain).is_err() {
            return;
        }
        if self.captain_phase(captain) != CaptainPhase::Active {
            self.captain_project = None;
            self.projects.captain_target = None;
            if let Some(index) = self
                .captain_home(captain)
                .and_then(|home| self.threads.iter().position(|thread| thread.id == home))
            {
                self.open_kept_tab(index);
            }
            self.captain_picker = false;
            return;
        }
        let explicit = self.captain_project.take();
        let existing_home = self.captain_home(captain);
        let requested_project =
            explicit.unwrap_or(self.projects.captain_target.or(self.current().project));
        if !self.captain_scopes.contains_key(&captain) {
            let scope = if existing_home.is_some() {
                self.captain_scope(captain)
            } else {
                requested_project.into_iter().collect()
            };
            self.captain_scopes.insert(captain, scope);
        } else if self.projects.captain_target.is_some()
            && let Some(project) = requested_project
        {
            let mut scope = self.captain_scope(captain);
            if !scope.contains(&project) {
                scope.push(project);
                if let Err(error) = self.set_captain_scope(captain, scope) {
                    self.lifecycle_error = Some(error);
                    return;
                }
            }
        }
        let project_id = self.captain_scope(captain).first().copied();
        let machine = self
            .project(project_id)
            .map(|p| p.machine.clone())
            .unwrap_or_else(|| "local".into());
        if self
            .captain_home(captain)
            .and_then(|home| self.threads.iter().position(|t| t.id == home))
            .is_none()
        {
            let mut thread = Thread::new(self.next_id);
            thread.harness = self.default_harness.clone();
            self.next_id += 1;
            thread.title = self.captains[captain].name.clone();
            thread.captain = Some(captain);
            thread.captain_home = true;
            thread.native.access = AccessMode::ReadOnly;
            thread.project = project_id;
            thread.set_machine(machine);
            self.threads.push(thread);
        }
        let index = self
            .threads
            .iter()
            .position(|t| t.captain == Some(captain) && Some(t.id) == existing_home)
            .unwrap_or(self.threads.len() - 1);
        let home = self.threads[index].id;
        self.captain_homes.insert(captain, home);
        if let Some(project) = self
            .projects
            .entries
            .iter_mut()
            .find(|p| Some(p.id) == requested_project)
            && project.captain.is_none()
            && (existing_home.is_none() || self.projects.captain_target.is_some())
        {
            project.captain = Some(captain);
            project.captain_thread = Some(home);
        }
        self.sync_memory_links();
        self.show_archive = false;
        self.captain_picker = false;
        self.projects.captain_target = None;
        self.open_kept_tab(index);
        if !self.catalog.models.is_empty() {
            self.apply_catalog();
        }
        self.checkpoint();
    }

    pub fn begin_new_captain(&mut self) {
        let suggested = self
            .projects
            .entries
            .iter()
            .find(|p| {
                !p.hidden
                    && !self
                        .threads
                        .iter()
                        .any(|t| t.captain_home && t.project == Some(p.id))
            })
            .map(|p| p.id);
        if self.captain_project.is_none() {
            self.captain_project = Some(self.current().project.or(suggested));
        }
        self.captain_picker = false;
        let used_names: std::collections::HashSet<String> = self
            .picker_captains()
            .map(|captain| self.captains[captain].name.to_lowercase())
            .collect();
        let used_portraits: std::collections::HashSet<usize> = self
            .picker_captains()
            .map(|captain| self.captains[captain].portrait)
            .collect();
        let (portrait, suggested_name) = (0..crate::visuals::PORTRAITS)
            .find(|i| {
                !used_portraits.contains(i)
                    && !used_names.contains(&crate::visuals::CAPTAIN_NAMES[*i].to_lowercase())
            })
            .map(|i| (i, crate::visuals::CAPTAIN_NAMES[i].to_string()))
            .unwrap_or_else(|| {
                let mut idx = self.captains.len() + 1;
                let candidate = loop {
                    let c = format!("Captain {idx}");
                    if !used_names.contains(&c.to_lowercase()) {
                        break c;
                    }
                    idx += 1;
                };
                (self.captains.len() % crate::visuals::PORTRAITS, candidate)
            });
        let mut editor = CaptainEditor::new();
        editor.server = self.machines.selected.clone();
        editor.portrait = portrait;
        editor.name.replace_text(&suggested_name);
        self.captain_editor = Some(editor);
    }

    pub fn edit_captain(&mut self, captain: usize) {
        self.cancel_captain_flow();
        let Some(profile) = self.captains.get(captain) else {
            return;
        };
        let mut editor = CaptainEditor::new();
        editor.editing = Some(captain);
        editor.server = self.machines.selected.clone();
        editor.name.replace_text(&profile.name);
        editor.role.replace_text(&profile.role);
        editor.instructions.replace_text(&profile.instructions);
        editor.portrait = profile.portrait;
        self.captain_editor = Some(editor);
    }

    pub fn cancel_captain_flow(&mut self) {
        self.captain_picker = false;
        self.captain_editor = None;
        self.captain_project = None;
        self.projects.captain_target = None;
    }

    pub fn save_captain(&mut self) {
        if self
            .captain_editor
            .as_ref()
            .is_some_and(|editor| editor.promotion.is_some())
        {
            if let Err(error) = self.save_captain_promotion()
                && let Some(editor) = self.captain_editor.as_mut()
            {
                editor.error = Some(error);
            }
            return;
        }
        if self.client.is_some() {
            let target_server = self
                .captain_editor
                .as_ref()
                .map(|editor| editor.server.clone());
            if let Some(server) = target_server
                && server != self.machines.selected
            {
                if !self.machine_connected(&server) {
                    return;
                }
                let editor = self.captain_editor.take();
                let captain_project = self.captain_project;
                let captain_target = self.projects.captain_target;
                self.select_machine_without_draft(&server);
                self.captain_editor = editor;
                self.captain_project = captain_project;
                self.projects.captain_target = captain_target;
            }
            if let Some(editor) = &self.captain_editor {
                if editor.server != self.machines.selected {
                    return;
                }
                let params = json!({"captain":editor.editing,"record":editor.profile().to_value(),"project":self.captain_project,"assignProject":self.projects.captain_target});
                match self
                    .request_tab_command("captain.put", params, TabIntent::Open(None))
                    .expect("client")
                {
                    Ok(_) => self.captain_editor = None,
                    Err(error) => self.captain_editor.as_mut().unwrap().error = Some(error),
                }
            }
            return;
        }
        let Some(editor) = &self.captain_editor else {
            return;
        };
        let profile = editor.profile();
        let editing = editor.editing;
        let error = profile
            .validate()
            .err()
            .or_else(|| editing.and_then(|id| self.require_active_captain(id).err()))
            .or_else(|| {
                self.captains
                    .iter()
                    .enumerate()
                    .any(|(i, existing)| {
                        Some(i) != editing
                            && existing.name.to_lowercase() == profile.name.to_lowercase()
                    })
                    .then(|| {
                        "That captain name is already in the roster. Choose a different name."
                            .into()
                    })
            })
            .or_else(|| {
                editing
                    .filter(|&id| {
                        self.threads
                            .iter()
                            .any(|t| t.captain == Some(id) && t.status.busy())
                    })
                    .map(|_| {
                        "Stop this captain's running threads before editing its profile.".into()
                    })
            });
        if let Some(error) = error {
            self.captain_editor.as_mut().unwrap().error = Some(error);
            return;
        }
        let id = if let Some(id) = editing {
            if id >= self.captains.len() {
                return;
            }
            self.captains[id] = profile;
            // Restart idle connections on demand so the next send uses the edited profile.
            for thread in self.threads.iter_mut().filter(|t| t.captain == Some(id)) {
                thread.disconnect();
                if thread.captain_home && !thread.renamed {
                    thread.title = self.captains[id].name.clone();
                }
            }
            id
        } else {
            self.captains.push(profile);
            self.captains.len() - 1
        };
        self.captain_editor = None;
        self.open_captain(id);
        self.restore_composer_focus(crate::ui::composer_ui::FocusReason::MenuClosed);
    }
    /// The captain's most urgent live thread. Archived conversations do not vote.
    pub fn captain_status(&self, captain: usize) -> Status {
        self.captain_home(captain)
            .map(|home| self.home_status(home))
            .unwrap_or(Status::Ready)
    }
    pub fn home_status(&self, home: usize) -> Status {
        let group = self.home_group_ids(home);
        self.threads
            .iter()
            .filter(|t| !t.archived() && group.contains(&t.id))
            .filter(|t| t.id == home || t.request.as_ref().is_none_or(|r| r["method"] == "confirm"))
            .map(|t| t.status)
            .max_by_key(|s| s.urgency())
            .unwrap_or(Status::Ready)
    }
    /// Steps the zoom one notch in (`1`), out (`-1`), or back to 100% (`0`).
    pub fn step_zoom(&mut self, direction: i32) {
        let index = ZOOM_STEPS
            .iter()
            .position(|&z| (z - self.zoom).abs() < 0.001)
            .unwrap_or(4);
        self.zoom = match direction {
            0 => 1.0,
            d if d > 0 => ZOOM_STEPS[(index + 1).min(ZOOM_STEPS.len() - 1)],
            _ => ZOOM_STEPS[index.saturating_sub(1)],
        };
        self.checkpoint();
    }
}
