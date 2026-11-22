//! Harness settings and delegation metadata, kept outside the shared model reducer.
use super::*;

#[derive(Default)]
pub(super) struct NativeModelMetadata {
    models: Vec<AgentModel>,
    cwd: Option<PathBuf>,
    models_cwd: Option<PathBuf>,
    workspace_snapshots: std::collections::BTreeMap<PathBuf, Vec<AgentModel>>,
    update_required_models: Vec<crate::model_picker::UpdateRequiredModel>,
}

impl NativeModelMetadata {
    fn models_for_workspace(&self, cwd: Option<&std::path::Path>) -> Option<&[AgentModel]> {
        crate::session::models_owned_by(
            &self.models,
            &self.workspace_snapshots,
            self.cwd.as_deref(),
            self.models_cwd.as_deref(),
            cwd,
        )
    }
}

#[derive(Clone, Copy)]
enum WorkspaceScope<'a> {
    Any,
    Thread {
        cwd: Option<&'a std::path::Path>,
        projectless: bool,
    },
}

#[derive(Default)]
enum ModelRefs<'a> {
    #[default]
    Empty,
    Slice(&'a [AgentModel]),
    Combined(Vec<&'a AgentModel>),
}

impl<'a> ModelRefs<'a> {
    fn iter(&self) -> impl Iterator<Item = &'a AgentModel> + Clone {
        let (models, combined) = match self {
            Self::Empty => (&[][..], &[][..]),
            Self::Slice(models) => (*models, &[][..]),
            Self::Combined(combined) => (&[][..], combined.as_slice()),
        };
        models.iter().chain(combined.iter().copied())
    }
}

#[derive(Default)]
struct PickerModelRefs<'a> {
    models: ModelRefs<'a>,
    default: Option<AgentModel>,
    retained: Option<AgentModel>,
}

impl PickerModelRefs<'_> {
    fn iter(&self) -> impl Iterator<Item = &AgentModel> + Clone {
        self.models
            .iter()
            .chain(&self.default)
            .chain(&self.retained)
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static PICKER_MAP_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn picker_option(model: &AgentModel) -> crate::model_picker::PickerModelOption {
    crate::model_picker::PickerModelOption {
        slug: model.id.clone(),
        name: model.name.clone(),
        short_name: model.short_name.clone(),
        sub_provider: model.sub_provider.clone(),
        aliases: model.aliases.clone(),
        badge: model.badge.clone(),
        is_default: model.is_default,
        is_custom: model.is_custom,
        is_legacy: model.is_legacy,
        is_unavailable: model.is_unavailable,
    }
}

fn antigravity_manifest_models() -> &'static [AgentModel] {
    static MODELS: std::sync::LazyLock<Vec<AgentModel>> =
        std::sync::LazyLock::new(crate::antigravity::manifest_models);
    &MODELS
}

impl Thread {
    pub fn set_service_tier(&mut self, tier: String) {
        if self.forward("thread.configure", json!({"serviceTier":tier})) {
            return;
        }
        if self.harness != "codex" || self.status.busy() || self.configuring {
            return;
        }
        if self
            .models
            .iter()
            .find(|model| model.id == self.model_id)
            .is_some_and(|model| {
                model
                    .service_tiers
                    .iter()
                    .any(|option| option["id"] == tier)
            })
        {
            self.harness_state["serviceTier"] = json!(tier);
        }
    }
    /// Native app-server usage is push-based, including refreshes after compaction.
    pub(super) fn apply_codex(&mut self, record: &Value) -> bool {
        if !matches!(self.harness.as_str(), "codex" | "claude") || record["type"] != "context_usage"
        {
            return false;
        }
        if self.context.phase != crate::context::ContextPhase::Compacting {
            self.context.cancel_request();
            self.context.absorb(&record["data"]);
            self.refresh_estimated_breakdown();
            self.native.stats = record["data"].clone();
        }
        true
    }

    pub(crate) fn context_harness_text(&self, text: &str) -> String {
        if self.harness == "codex" {
            text.replace("Pi", "Codex")
        } else if self.harness == "claude" {
            text.replace("Pi", "Claude")
        } else {
            text.into()
        }
    }

    pub fn has_started_session(&self) -> bool {
        !self.messages.is_empty()
            || self.life.latest_user_message_at.is_some()
            || self.native.fork_source.is_some()
            || self.native.previous_session.is_some()
            || (self.session.is_some() && !self.never_sent)
            || self.run_id.is_some()
    }

    pub fn thread_model_selection(&self) -> Option<crate::model_selection::ModelSelection> {
        let model = self
            .selected_model
            .as_ref()
            .map(|(_, id)| id.clone())
            .or_else(|| (!self.model_id.is_empty()).then(|| self.model_id.clone()))?;
        let mut options = Vec::new();
        let reasoning_id = if self.harness == "pi" {
            "thinking"
        } else if self.harness == "codex" {
            "reasoningEffort"
        } else {
            "effort"
        };
        let reasoning = self
            .selected_thinking
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| (!self.thinking.is_empty()).then_some(self.thinking.as_str()));
        if let Some(level) = reasoning {
            options.push(crate::model_selection::ProviderOptionSelection {
                id: reasoning_id.into(),
                value: crate::model_selection::OptionValue::Str(level.to_owned()),
            });
        }
        if let Some(tier) = self.harness_state["serviceTier"].as_str() {
            options.push(crate::model_selection::ProviderOptionSelection {
                id: "serviceTier".into(),
                value: crate::model_selection::OptionValue::Str(tier.to_owned()),
            });
        }
        Some(crate::model_selection::ModelSelection {
            instance_id: self.harness.clone(),
            model,
            options,
        })
    }

    pub fn set_option(&mut self, id: String, value: crate::model_selection::OptionValue) {
        if self.forward(
            "thread.configure",
            json!({"options": [{"id": id, "value": value}]}),
        ) {
            return;
        }
        // A reasoning choice stays recorded while a configure is in flight: the newest
        // wanted level renders at once and the acknowledgement path reconciles it in
        // order, instead of dropping a rapid second pick or sending a concurrent change.
        let reasoning = matches!(
            id.as_str(),
            "thinking" | "effort" | "reasoningEffort" | "reasoning" | "thought_level"
        ) && value.as_str().is_some()
            && matches!(self.harness.as_str(), "pi" | "codex" | "claude");
        let can_configure = if reasoning {
            self.can_choose_thinking()
        } else {
            !self.status.busy() && !self.configuring
        };
        if !can_configure {
            return;
        }
        self.model_options.retain(|o| o.id != id);
        self.model_options
            .push(crate::model_selection::ProviderOptionSelection {
                id: id.clone(),
                value: value.clone(),
            });
        // KSG `resolveClaudeCatalogContextWindowTokens`: a Claude model's window is
        // the selected contextWindow option's tokens, not a fixed catalog value.
        if self.harness == "claude" && id == "contextWindow" {
            let catalog =
                crate::claude_catalog::bundled_claude_model_catalog(crate::lifecycle::now());
            let selection = crate::model_selection::ModelSelection {
                instance_id: self.harness.clone(),
                model: self.model_id.clone(),
                options: self.model_options.clone(),
            };
            if let Some(tokens) =
                crate::claude_catalog::resolve_claude_catalog_context_window_tokens(
                    &catalog,
                    Some(&selection),
                )
            {
                self.context_window = Some(tokens);
            }
        }
        if reasoning && let Some(level) = value.as_str() {
            self.selected_thinking = Some(level.to_owned());
            if self.configuring {
                return;
            }
            if self.pi.is_none() || self.initializing {
                self.thinking = level.to_owned();
            } else if self.command("set_thinking_level", json!({"level": level})) {
                self.configuring = true;
                self.pending_thinking = Some(level.to_owned());
            }
            return;
        }
        if id == "serviceTier"
            && let Some(tier) = value.as_str()
        {
            self.harness_state["serviceTier"] = json!(tier);
            if self.harness == "codex" && self.pi.is_some() && !self.initializing {
                self.command("set_service_tier", json!({"tier": tier}));
            }
            return;
        }
        if self.pi.is_some() && !self.initializing {
            let val_json = match &value {
                crate::model_selection::OptionValue::Bool(b) => json!(b),
                crate::model_selection::OptionValue::Str(s) => json!(s),
            };
            self.command("set_option", json!({"optionId": id, "value": val_json}));
        }
    }
}

impl State {
    pub(super) fn update_projected_native_models(&mut self, next: &Value) {
        let Some(catalogs) = next["nativeCatalogs"].as_object() else {
            self.projected_native_catalogs.clear();
            return;
        };
        self.projected_native_catalogs
            .retain(|id, _| catalogs.contains_key(id));
        for (harness, next) in catalogs {
            let previous = &self.client_views["nativeCatalogs"][harness];
            let models_changed = previous["models"] != next["models"];
            let updates_changed = previous["updateRequiredModels"] != next["updateRequiredModels"];
            let cwd_changed = previous["cwd"] != next["cwd"];
            let models_cwd_changed = previous["modelsCwd"] != next["modelsCwd"];
            let snapshots_changed = previous["workspaceSnapshots"] != next["workspaceSnapshots"];
            if models_changed
                || updates_changed
                || cwd_changed
                || models_cwd_changed
                || snapshots_changed
                || !self.projected_native_catalogs.contains_key(harness)
            {
                let cached = self
                    .projected_native_catalogs
                    .entry(harness.clone())
                    .or_default();
                if models_changed {
                    cached.models =
                        serde_json::from_value(next["models"].clone()).unwrap_or_default();
                }
                if updates_changed {
                    cached.update_required_models =
                        serde_json::from_value(next["updateRequiredModels"].clone())
                            .unwrap_or_default();
                }
                if cwd_changed || models_cwd_changed {
                    cached.cwd = next["cwd"].as_str().map(PathBuf::from);
                    cached.models_cwd = if next.get("modelsCwd").is_some() {
                        next["modelsCwd"].as_str().map(PathBuf::from)
                    } else {
                        cached.cwd.clone()
                    };
                }
                if snapshots_changed {
                    cached.workspace_snapshots =
                        serde_json::from_value(next["workspaceSnapshots"].clone())
                            .unwrap_or_default();
                }
            }
        }
    }

    pub(crate) fn pi_catalog_status(&self) -> (bool, Option<&str>) {
        if self.client.is_some() {
            let catalog = &self.client_views["catalog"];
            return (
                catalog["loading"] == true || self.catalog.loading(),
                self.catalog
                    .error
                    .as_deref()
                    .or_else(|| catalog["error"].as_str()),
            );
        }
        (self.catalog.loading(), self.catalog.error.as_deref())
    }
    pub(crate) fn native_catalog_status(&self, harness: &str) -> (bool, Option<&str>) {
        if self.client.is_some() {
            let catalog = &self.client_views["nativeCatalogs"][harness];
            return (catalog["loading"] == true, catalog["error"].as_str());
        }
        let catalog = match harness {
            "codex" => &self.codex_catalog,
            "claude" => &self.claude_catalog,
            "antigravity" => &self.antigravity_catalog,
            "cursor" => &self.cursor_catalog,
            _ => match self.acp_catalogs.get(harness) {
                Some(catalog) => catalog,
                None => return (false, None),
            },
        };
        let catalog = self.acp_catalogs.get(harness).unwrap_or(catalog);
        (catalog.loading(), catalog.error.as_deref())
    }

    /// The canonical catalog for a native or ACP harness; an ACP entry wins over
    /// the built-in catalog, matching `native_catalog_status`.
    pub(super) fn native_catalog(&self, harness: &str) -> Option<&crate::session::NativeCatalog> {
        let builtin = match harness {
            "codex" => Some(&self.codex_catalog),
            "claude" => Some(&self.claude_catalog),
            "antigravity" => Some(&self.antigravity_catalog),
            "cursor" => Some(&self.cursor_catalog),
            _ => None,
        };
        self.acp_catalogs.get(harness).or(builtin)
    }

    pub(super) fn native_catalog_mut(
        &mut self,
        harness: &str,
    ) -> Option<&mut crate::session::NativeCatalog> {
        if let Some(catalog) = self.acp_catalogs.get_mut(harness) {
            return Some(catalog);
        }
        match harness {
            "codex" => Some(&mut self.codex_catalog),
            "claude" => Some(&mut self.claude_catalog),
            "antigravity" => Some(&mut self.antigravity_catalog),
            "cursor" => Some(&mut self.cursor_catalog),
            _ => None,
        }
    }

    pub(super) fn refresh_provider_availability(&mut self, force: bool) {
        if !self.demo && (self.runtime_enabled || self.client.is_some()) {
            self.provider_availability.refresh(self.wake.clone(), force);
        }
    }

    pub(super) fn poll_provider_availability(&mut self) -> bool {
        let changed = self.provider_availability.poll(&mut self.harnesses);
        self.refresh_provider_availability(false);
        changed
    }

    pub(super) fn builtin_cli_available(&self, id: &str) -> bool {
        if self.client.is_some() {
            // Prefer the selected server's current discovery, including removals.
            if let Some(ids) = self.client_views["harnessIds"].as_array() {
                return ids.iter().any(|value| value == id);
            }
            if self.machines.selected != "local" {
                return self.client_views["nativeCatalogs"][id]["installed"] == true;
            }
        }
        self.provider_availability
            .last
            .as_ref()
            .is_some_and(|state| match id {
                "codex" => true,
                "claude" => state.claude,
                "antigravity" => state.antigravity,
                "cursor" => state.cursor,
                _ => false,
            })
    }

    /// `interval` marks the event-loop pass: healthy catalogs that opt out keep
    /// their snapshot, matching KSG's per-provider `refreshOnInterval`.
    pub(super) fn refresh_native_catalogs(&mut self, force: bool, interval: bool) -> bool {
        if self.demo || !self.runtime_enabled || self.client.is_some() {
            return false;
        }
        // An explicit refresh must include a CLI that has just been installed.
        self.refresh_optional_catalogs |= force;
        let optional_force = !self.provider_availability.loading()
            && std::mem::take(&mut self.refresh_optional_catalogs);
        let cwd = self.working_directory();
        // Unsent scratch threads have no directory yet. Use the existing profile
        // directory without creating a thread directory or a missing project.
        let acp_cwd = if cwd.is_dir() { &cwd } else { &self.data_dir };
        let wake = &self.wake;
        let mut changed = false;
        if !self.provider_update_drivers.contains("codex") {
            changed |= self
                .codex_catalog
                .refresh("codex", &cwd, wake.clone(), force);
        }
        if !self.provider_update_drivers.contains("claude") {
            changed |= self
                .claude_catalog
                .refresh("claude", &cwd, wake.clone(), force);
        }
        if !self.refresh_optional_catalogs
            && self.builtin_cli_available("antigravity")
            && !self
                .harnesses
                .agents
                .iter()
                .any(|agent| agent.id == "antigravity")
            && !self.provider_update_drivers.contains("antigravity")
        {
            changed |= self.antigravity_catalog.refresh(
                "antigravity",
                acp_cwd,
                wake.clone(),
                optional_force,
            );
        }
        if !self.refresh_optional_catalogs
            && self.builtin_cli_available("cursor")
            && !self
                .harnesses
                .agents
                .iter()
                .any(|agent| agent.id == "cursor")
            && !self.provider_update_drivers.contains("cursor")
        {
            changed |= self
                .cursor_catalog
                .refresh("cursor", acp_cwd, wake.clone(), optional_force);
        }
        self.acp_catalogs
            .retain(|id, _| self.harnesses.agents.iter().any(|agent| agent.id == *id));
        for agent in &self.harnesses.agents {
            changed |= self
                .acp_catalogs
                .entry(agent.id.clone())
                .or_default()
                .refresh_acp(agent, acp_cwd, wake.clone(), optional_force, interval);
        }
        if changed {
            self.apply_native_catalogs();
        }
        changed
    }

    pub(crate) fn catalog_workspace_cwd(&self, index: usize) -> Option<PathBuf> {
        if let Ok(dir) = self.thread_directory(index) {
            return Some(dir);
        }
        let thread = self.threads.get(index)?;
        if !thread.client_draft {
            return None;
        }
        if let Some(worktree_path) = &thread.worktree_path {
            return Some(PathBuf::from(worktree_path));
        }
        self.project(thread.project)
            .map(|project| project.path.clone())
    }

    /// A chat with no project runs in `<data_dir>/chats/<id>`, a folder Shina
    /// creates for it.
    fn scratch_chat_dir(&self, dir: &std::path::Path) -> bool {
        dir.parent() == Some(self.data_dir.join("chats").as_path())
    }

    pub(super) fn apply_native_catalogs(&mut self) {
        if self.client.is_some() {
            let directories: Vec<_> = (0..self.threads.len())
                .map(|index| self.catalog_workspace_cwd(index))
                .collect();
            let updates: Vec<_> = (0..self.threads.len())
                .map(|index| {
                    let thread = &self.threads[index];
                    if !thread.client_draft
                        || !self.projected_native_catalogs.contains_key(&thread.harness)
                    {
                        return None;
                    }
                    let harness = thread.harness.as_str();
                    let cwd = directories[index].as_deref();
                    let models = self
                        .projected_native_catalogs
                        .get(harness)
                        .and_then(|meta| meta.models_for_workspace(cwd))
                        .map(|models| models.to_vec());
                    // A client draft's reasoning levels always reflect its current
                    // workspace snapshot or workspace-scoped rows. An empty snapshot
                    // or a refresh that drops the selected model clears stale levels.
                    let levels = match &models {
                        Some(models) => models
                            .iter()
                            .find(|model| {
                                model.provider == thread.model_provider
                                    && model.id == thread.model_id
                            })
                            .map(AgentModel::reasoning_levels)
                            .unwrap_or_default(),
                        None => self
                            .harness_model_refs_for_workspace(
                                harness,
                                cwd,
                                thread.project.is_none(),
                            )
                            .iter()
                            .find(|model| {
                                model.provider == thread.model_provider
                                    && model.id == thread.model_id
                            })
                            .map(AgentModel::reasoning_levels)
                            .unwrap_or_default(),
                    };
                    Some((harness.to_owned(), models, levels))
                })
                .collect();
            let projects = &self.projects.entries;
            for (thread, update) in self.threads.iter_mut().zip(updates) {
                let Some((harness, models, levels)) = update else {
                    continue;
                };
                let catalog = &self.client_views["nativeCatalogs"][&harness];
                if let Some(models) = models {
                    if harness == "cursor" {
                        thread.cursor_catalog_models = Some(models.clone());
                    }
                    if thread.models != models {
                        thread.models = models;
                    }
                } else {
                    if harness == "cursor" {
                        thread.cursor_catalog_models = None;
                    }
                    if !thread.models.is_empty() {
                        thread.models.clear();
                    }
                }
                if thread.thinking_levels != levels {
                    thread.thinking_levels = levels;
                }
                thread.harness_state["codexNativeExtensions"] = catalog["extensions"].clone();
                if catalog["ready"] == true
                    && projects
                        .iter()
                        .find(|project| Some(project.id) == thread.project)
                        .is_some_and(|project| {
                            catalog["cwd"]
                                .as_str()
                                .is_some_and(|cwd| std::path::Path::new(cwd) == project.path)
                        })
                {
                    let commands = catalog["commands"].as_array().cloned().unwrap_or_default();
                    thread.native.skills = crate::composer_skills::from_commands(&commands);
                    thread.native.commands = commands
                        .iter()
                        .filter_map(|command| {
                            Some(pi_session::SlashCommand {
                                name: command["name"].as_str()?.into(),
                                description: command["description"].as_str().unwrap_or("").into(),
                                source: command["source"].as_str().unwrap_or(&harness).into(),
                            })
                        })
                        .collect();
                    thread.native.commands_loaded = true;
                }
            }
            return;
        }
        let directories: Vec<_> = (0..self.threads.len())
            .map(|index| self.thread_directory(index).ok())
            .collect();
        for (index, thread) in self.threads.iter_mut().enumerate() {
            let acp_catalog = self.acp_catalogs.get(&thread.harness);
            let catalog = if let Some(catalog) = acp_catalog {
                catalog
            } else {
                match thread.harness.as_str() {
                    "codex" => &self.codex_catalog,
                    "claude" => &self.claude_catalog,
                    "antigravity" => &self.antigravity_catalog,
                    "cursor" => &self.cursor_catalog,
                    _ => continue,
                }
            };
            let workspace_models = catalog.models_for_workspace(directories[index].as_deref());
            if thread.harness == "cursor" {
                let next = workspace_models.map(|models| models.to_vec());
                if thread.cursor_catalog_models != next {
                    thread.cursor_catalog_models = next;
                }
            }
            if thread.pi.is_some() && (thread.harness != "cursor" || acp_catalog.is_some()) {
                continue;
            }
            if thread.harness_state["catalogRevision"].as_u64() == Some(catalog.revision) {
                continue;
            }
            thread.harness_state["catalogRevision"] = json!(catalog.revision);
            if thread.pi.is_none()
                && acp_catalog.is_none()
                && let Some(error) = &catalog.error
                && crate::usage_limits::authentication_error(error)
            {
                thread.error = Some(error.clone());
                thread.status = Status::Failed;
            }
            if thread.pi.is_none() && !catalog.commands_ready() {
                thread.native.commands_loaded = false;
                thread.native.commands.clear();
                thread.native.skills.clear();
            }
            if let Some(models) = workspace_models {
                if thread.models != models {
                    thread.models = models.to_vec();
                }
                thread.refresh_thinking_levels();
            }
            if thread.pi.is_none() && catalog.commands_ready() && directories[index] == catalog.cwd
            {
                thread.native.skills = crate::composer_skills::from_commands(&catalog.commands);
                thread.native.commands = catalog
                    .commands
                    .iter()
                    .filter_map(|command| {
                        Some(pi_session::SlashCommand {
                            name: command["name"].as_str()?.into(),
                            description: command["description"].as_str().unwrap_or("").into(),
                            source: command["source"].as_str().unwrap_or(&thread.harness).into(),
                        })
                    })
                    .collect();
                thread.native.commands_loaded = true;
                thread.native.commands_requested = false;
                thread.harness_state["codexNativeExtensions"] = json!(catalog.extensions);
            }
        }
        self.sync_fallback_model(crate::lifecycle::now() as i64);
    }

    fn cursor_catalog_models_for_cwd(
        &self,
        cwd: Option<&std::path::Path>,
    ) -> Option<Vec<AgentModel>> {
        if self.client.is_some() {
            return self
                .projected_native_catalogs
                .get("cursor")
                .and_then(|catalog| catalog.models_for_workspace(cwd))
                .map(|models| models.to_vec());
        }
        let catalog = self.native_catalog("cursor")?;
        catalog
            .models_for_workspace(cwd)
            .map(|models| models.to_vec())
    }

    pub(crate) fn sync_cursor_catalog_models(&mut self, index: usize) {
        if self.threads[index].harness != "cursor" {
            return;
        }
        let cwd = self.catalog_workspace_cwd(index);
        let next = self.cursor_catalog_models_for_cwd(cwd.as_deref());
        if self.threads[index].cursor_catalog_models != next {
            self.threads[index].cursor_catalog_models = next;
        }
    }

    pub(super) fn validate_harness(&self, id: &str) -> Result<(), String> {
        if matches!(id, "pi" | "codex" | "claude") {
            Ok(())
        } else if matches!(id, "antigravity" | "cursor")
            && !self.harnesses.agents.iter().any(|agent| agent.id == id)
        {
            // Built-in ACP providers validate against their CLI, not the registry.
            self.builtin_cli_available(id).then_some(()).ok_or_else(|| {
                if id == "cursor" {
                    crate::cursor::MISSING.into()
                } else {
                    "Antigravity is unavailable. Put `agy` on PATH.".into()
                }
            })
        } else {
            self.harnesses.find(id).map(drop)
        }
    }

    fn harness_model_refs(&self, harness: &str) -> ModelRefs<'_> {
        self.harness_model_refs_in_scope(harness, WorkspaceScope::Any)
    }

    fn harness_model_refs_for_cwd(
        &self,
        harness: &str,
        cwd: Option<&std::path::Path>,
    ) -> ModelRefs<'_> {
        let projectless =
            cwd.is_some_and(|dir| !self.projects.entries.iter().any(|p| p.path == dir));
        self.harness_model_refs_for_workspace(harness, cwd, projectless)
    }

    fn harness_model_refs_for_workspace(
        &self,
        harness: &str,
        cwd: Option<&std::path::Path>,
        projectless: bool,
    ) -> ModelRefs<'_> {
        self.harness_model_refs_in_scope(harness, WorkspaceScope::Thread { cwd, projectless })
    }

    fn harness_model_refs_in_scope(
        &self,
        harness: &str,
        scope: WorkspaceScope<'_>,
    ) -> ModelRefs<'_> {
        let provider = if harness == "pi" { "cpa" } else { harness };
        let projected_opt = self
            .projected_native_catalogs
            .get(harness)
            .and_then(|c| match scope {
                WorkspaceScope::Any => (!c.models.is_empty()).then_some(c.models.as_slice()),
                WorkspaceScope::Thread { cwd, .. } => c.models_for_workspace(cwd),
            });
        let projected = projected_opt.unwrap_or(&[]);
        let acp = self.acp_catalogs.get(harness);
        let projected_acp = self.client_views["nativeCatalogs"][harness]["name"].is_string();
        if projected_acp {
            if let Some(models) = projected_opt {
                return ModelRefs::Slice(models);
            }
        } else if let Some(catalog) = acp {
            let acp_models = match scope {
                WorkspaceScope::Any => {
                    (!catalog.models.is_empty()).then_some(catalog.models.as_slice())
                }
                WorkspaceScope::Thread { cwd, .. } => catalog.models_for_workspace(cwd),
            };
            if let Some(models) = acp_models {
                // An advertised catalog replaces earlier results. Don't merge stale rows from
                // other threads back into the provider's picker tab.
                return ModelRefs::Slice(models);
            }
        }
        if harness == "cursor" && acp.is_none() && !projected_acp {
            if self.client.is_some() {
                if let Some(models) = projected_opt {
                    return ModelRefs::Slice(models);
                }
            } else {
                let cursor_models = match scope {
                    WorkspaceScope::Any => (!self.cursor_catalog.models.is_empty())
                        .then_some(self.cursor_catalog.models.as_slice()),
                    WorkspaceScope::Thread { cwd, .. } => {
                        self.cursor_catalog.models_for_workspace(cwd)
                    }
                };
                if let Some(models) = cursor_models {
                    return ModelRefs::Slice(models);
                }
            }
        }
        fn catalog_slice<'a>(
            cat: &'a crate::session::NativeCatalog,
            scope: WorkspaceScope<'_>,
        ) -> &'a [AgentModel] {
            match scope {
                WorkspaceScope::Any => cat.models.as_slice(),
                WorkspaceScope::Thread { cwd, .. } => cat.models_for_workspace(cwd).unwrap_or(&[]),
            }
        }
        let catalog = match harness {
            "pi" => self.catalog.models.as_slice(),
            "codex" => catalog_slice(&self.codex_catalog, scope),
            "claude" => catalog_slice(&self.claude_catalog, scope),
            "antigravity" if acp.is_none() && !projected_acp => {
                catalog_slice(&self.antigravity_catalog, scope)
            }
            "cursor" if acp.is_none() && !projected_acp => {
                catalog_slice(&self.cursor_catalog, scope)
            }
            _ => &[],
        };
        let default = self
            .catalog
            .default_model
            .as_ref()
            .filter(|default| harness == "pi" && !catalog.iter().any(|m| m.id == default.id));
        // Another thread's cached rows count only for the workspace that produced
        // them, and an unknown workspace borrows none. On the local machine only,
        // projectless Codex and Claude chats share cached rows across their own
        // scratch folders because those folders hold no provider config. A linked
        // machine or any registered project stays isolated to its exact folder.
        // Pi's CPA catalog is machine-wide and has no owner folder.
        let scratch_shared = self.machines.selected == "local"
            && matches!(harness, "codex" | "claude")
            && acp.is_none()
            && !projected_acp;
        let same_workspace = |index: usize| match scope {
            WorkspaceScope::Any => true,
            WorkspaceScope::Thread { cwd: None, .. } => false,
            WorkspaceScope::Thread {
                cwd: Some(dir),
                projectless,
            } => self.catalog_workspace_cwd(index).is_some_and(|cached| {
                cached == dir
                    || (scratch_shared
                        && projectless
                        && self.threads[index].project.is_none()
                        && self.threads[index].machine == "local"
                        && !self.projects.entries.iter().any(|p| p.path == dir)
                        && !self.projects.entries.iter().any(|p| p.path == cached)
                        && self.scratch_chat_dir(&cached)
                        && self.scratch_chat_dir(dir))
            }),
        };
        let mut thread_models = self
            .threads
            .iter()
            .enumerate()
            .filter(|(idx, t)| {
                t.harness == harness
                    && !t.models.is_empty()
                    && !(t.client_draft && self.projected_native_catalogs.contains_key(harness))
                    && (harness == "pi" || same_workspace(*idx))
            })
            .map(|(_, t)| t)
            .peekable();
        if (projected.is_empty() || catalog.is_empty())
            && default.is_none()
            && thread_models.peek().is_none()
        {
            if projected.is_empty() && catalog.is_empty() && harness == "antigravity" {
                return ModelRefs::Slice(antigravity_manifest_models());
            }
            return ModelRefs::Slice(if projected.is_empty() {
                catalog
            } else {
                projected
            });
        }
        let mut models: Vec<_> = projected.iter().chain(catalog).chain(default).collect();
        let mut seen: std::collections::HashSet<&str> =
            models.iter().map(|model| model.id.as_str()).collect();
        for thread in thread_models {
            for model in &thread.models {
                if model.provider == provider && seen.insert(&model.id) {
                    models.push(model);
                }
            }
        }
        if harness == "antigravity" && models.is_empty() {
            return ModelRefs::Slice(antigravity_manifest_models());
        }
        ModelRefs::Combined(models)
    }

    pub(super) fn harness_models(&self, harness: &str) -> Vec<AgentModel> {
        self.harness_model_refs(harness).iter().cloned().collect()
    }

    pub(super) fn harness_models_for_cwd(
        &self,
        harness: &str,
        cwd: Option<&std::path::Path>,
    ) -> Vec<AgentModel> {
        self.harness_model_refs_for_cwd(harness, cwd)
            .iter()
            .cloned()
            .collect()
    }

    pub(super) fn harness_models_for_thread(&self, index: usize, harness: &str) -> Vec<AgentModel> {
        let cwd = self.catalog_workspace_cwd(index);
        let projectless = self.threads.get(index).is_some_and(|t| t.project.is_none());
        self.harness_model_refs_for_workspace(harness, cwd.as_deref(), projectless)
            .iter()
            .cloned()
            .collect()
    }

    pub(super) fn default_harness_model(
        &self,
        harness: &str,
        cwd: Option<&std::path::Path>,
    ) -> Option<&AgentModel> {
        let refs = self.harness_model_refs_for_cwd(harness, cwd);
        refs.iter()
            .find(|model| model.is_default)
            .or_else(|| refs.iter().next())
    }

    pub(super) fn harness_model(
        &self,
        harness: &str,
        model_id: &str,
        cwd: Option<&std::path::Path>,
    ) -> Option<&AgentModel> {
        self.harness_model_refs_for_cwd(harness, cwd)
            .iter()
            .find(|model| model.id == model_id)
    }

    pub(super) fn add_harness_capabilities(
        &self,
        caller_index: Option<usize>,
        capabilities: &mut Value,
    ) {
        let harnesses = self
            .harness_ids()
            .iter()
            .map(|harness| self.harness_capability(harness, caller_index))
            .collect::<Vec<_>>();
        capabilities["harnesses"] = json!(harnesses);
        let scoped = |harness: &str| match caller_index {
            Some(index) => self
                .workspace_models(index, harness)
                .unwrap_or_else(|| self.harness_models_for_thread(index, harness)),
            None => self.harness_models(harness),
        };
        let codex_models = scoped("codex");
        let claude_models = scoped("claude");
        let cursor_models = scoped("cursor");
        let values = capabilities["models"].as_array_mut().expect("models array");
        for model in codex_models
            .into_iter()
            .chain(claude_models)
            .chain(cursor_models)
        {
            if !values
                .iter()
                .any(|m| m["provider"] == model.provider && m["id"] == model.id)
            {
                values.push(
                    json!({"harness":model.provider,"provider":model.provider,"id":model.id,
                    "name":model.name,"reasoning_levels":model.thinking_levels,"service_tiers":model.service_tiers}),
                );
            }
        }
        for value in values.iter_mut() {
            if value.get("harness").is_none() {
                value["harness"] = if value["provider"] == "cpa" {
                    json!("pi")
                } else {
                    value["provider"].clone()
                };
            }
        }
        capabilities["ready"] = json!(!values.is_empty());
        capabilities["runtime"]["providers"] = json!(["cpa", "codex", "claude", "cursor"]);
    }

    /// One default for new lone threads and captain homes. Workers inherit their parent.
    pub fn cycle_default_harness(&mut self) {
        let ids = self.harness_ids();
        let position = ids
            .iter()
            .position(|id| *id == self.default_harness)
            .unwrap_or(0);
        let next = ids[(position + 1) % ids.len()].clone();
        // In client mode the server owns this setting; its projection brings it back.
        if self.forward("settings.defaultHarness", json!({"harness": next})) {
            return;
        }
        self.default_harness = next;
        self.checkpoint();
    }

    /// Harness ids a thread can use: built-ins, then registered ACP agents.
    pub(crate) fn harness_ids(&self) -> Vec<String> {
        let mut ids = vec!["pi".to_string(), "codex".to_string(), "claude".to_string()];
        if self.builtin_cli_available("antigravity")
            && !self.client_views["nativeCatalogs"]["antigravity"]["name"].is_string()
            && !self
                .harnesses
                .agents
                .iter()
                .any(|agent| agent.id == "antigravity")
        {
            ids.push("antigravity".to_string());
        }
        if self.builtin_cli_available("cursor")
            && !self.client_views["nativeCatalogs"]["cursor"]["name"].is_string()
            && !self
                .harnesses
                .agents
                .iter()
                .any(|agent| agent.id == "cursor")
        {
            ids.push("cursor".to_string());
        }
        ids.extend(self.harnesses.agents.iter().map(|agent| agent.id.clone()));
        if let Some(projected) = self.client_views["harnessIds"].as_array() {
            for id in projected.iter().filter_map(Value::as_str) {
                if !ids.iter().any(|known| known == id) {
                    ids.push(id.into());
                }
            }
        }
        ids
    }

    pub fn supports_model_favorites(&self) -> bool {
        self.client.as_ref().is_none_or(|client| {
            client.supports_capability(&self.machines.selected, crate::protocol::MODEL_FAVORITES)
        })
    }

    pub fn effective_model_favorites(&self) -> &[crate::model_picker::ModelFavorite] {
        if self.supports_model_favorites() {
            self.model_favorites
                .as_deref()
                .unwrap_or(&self.preferences.model_favorites)
        } else {
            &self.preferences.model_favorites
        }
    }

    pub fn toggle_model_favorite(&mut self, provider: &str, model: &str) {
        let currently_favorite = self
            .effective_model_favorites()
            .iter()
            .any(|fav| fav.provider == provider && fav.model == model);
        let next = !currently_favorite;
        if self.client.is_some() && self.supports_model_favorites() {
            if let Some(list) = &mut self.model_favorites {
                crate::model_picker::toggle_model_favorite_in_list(list, provider, model);
            } else {
                let mut list = self.preferences.model_favorites.clone();
                crate::model_picker::toggle_model_favorite_in_list(&mut list, provider, model);
                self.model_favorites = Some(list);
            }
            self.forward(
                "settings.modelFavorites",
                json!({"provider": provider, "model": model, "favorite": next}),
            );
            return;
        }
        if self.client.is_some() {
            crate::model_picker::toggle_model_favorite_in_list(
                &mut self.preferences.model_favorites,
                provider,
                model,
            );
            self.checkpoint();
            return;
        }
        if self.model_favorites.is_none() && !self.preferences.model_favorites.is_empty() {
            self.model_favorites = Some(self.preferences.model_favorites.clone());
        }
        let list = self.model_favorites.get_or_insert_with(Vec::new);
        crate::model_picker::toggle_model_favorite_in_list(list, provider, model);
        self.checkpoint();
    }

    /// KSG's `threadSupportsProviderHandoff`: local threads hand off natively; remote
    /// ones need the owning server to advertise `provider-handoff-v1`.
    pub fn provider_handoff_supported(&self, thread: &Thread) -> bool {
        thread.remote.is_none()
            || self.client.as_ref().is_some_and(|client| {
                client.supports_capability(&thread.machine, crate::protocol::PROVIDER_HANDOFF)
            })
    }

    pub fn locked_provider_for_current(&self) -> Option<String> {
        let t = self.current();
        if t.has_started_session() && !self.provider_handoff_supported(t) {
            Some(t.harness.clone())
        } else {
            None
        }
    }

    pub fn model_disabled_reason_for(&self, instance_id: &str, _model: &str) -> Option<String> {
        let t = self.current();
        crate::model_picker::get_model_disabled_reason(
            t.has_started_session(),
            self.provider_handoff_supported(t),
            &t.harness,
            instance_id,
        )
        .or_else(|| {
            // Cross-provider picks are allowed in principle but blocked right now.
            (t.has_started_session() && instance_id != t.harness)
                .then(|| t.handoff_block_reason())
                .flatten()
        })
    }

    pub fn provider_instance_entries(&self) -> Vec<crate::model_picker::ProviderInstanceEntry> {
        use crate::model_picker::{ProviderAuthStatus, ProviderInstanceEntry, ProviderStatusKind};
        let mut entries = Vec::new();
        let mut seen: Vec<(String, String)> = Vec::new();

        // Pi
        seen.push(("pi".into(), "Pi".into()));
        let pi_error = self.catalog.error.clone();
        let pi_auth = if pi_error
            .as_deref()
            .is_some_and(crate::usage_limits::authentication_error)
        {
            ProviderAuthStatus::Unauthenticated
        } else if !self.catalog.models.is_empty() {
            ProviderAuthStatus::Authenticated
        } else {
            ProviderAuthStatus::Unknown
        };
        entries.push(ProviderInstanceEntry {
            instance_id: "pi".into(),
            driver_kind: "pi".into(),
            display_name: "Pi".into(),
            continuation_group_key: None,
            enabled: true,
            installed: true,
            status: if pi_error.is_some() {
                ProviderStatusKind::Error
            } else {
                ProviderStatusKind::Ready
            },
            auth_status: pi_auth,
            can_setup: false,
            is_available: true,
            is_acp: false,
            message: pi_error,
            compatibility_broken_message: None,
            update_required_models: vec![],
        });

        // Codex & Claude
        for (id, name) in [("codex", "Codex"), ("claude", "Claude")] {
            seen.push((id.into(), name.into()));
            let (
                installed,
                error,
                auth_status,
                update_required_models,
                has_models,
                provider_message,
            ) = if self.client.is_some() {
                let snap = &self.client_views["nativeCatalogs"][id];
                let models_empty = snap["models"].as_array().is_none_or(Vec::is_empty);
                let err = snap["error"].as_str().map(str::to_owned);
                let installed = snap["installed"].as_bool().unwrap_or(!models_empty);
                let auth = snap["authStatus"]
                    .as_str()
                    .map(ProviderAuthStatus::parse)
                    .unwrap_or_else(|| {
                        if err
                            .as_deref()
                            .is_some_and(crate::usage_limits::authentication_error)
                        {
                            ProviderAuthStatus::Unauthenticated
                        } else if !models_empty {
                            ProviderAuthStatus::Authenticated
                        } else {
                            ProviderAuthStatus::Unknown
                        }
                    });
                let updates = self
                    .projected_native_catalogs
                    .get(id)
                    .map(|catalog| catalog.update_required_models.clone())
                    .unwrap_or_default();
                let message = snap["providerMessage"].as_str().map(str::to_owned);
                (installed, err, auth, updates, !models_empty, message)
            } else {
                let catalog = if id == "codex" {
                    &self.codex_catalog
                } else {
                    &self.claude_catalog
                };
                let installed = if id == "claude" {
                    catalog.installed || self.builtin_cli_available("claude")
                } else {
                    true
                };
                let err = catalog.error.clone().or_else(|| {
                    (id == "claude" && !installed).then(|| crate::claude::MISSING.into())
                });
                let auth = catalog
                    .auth_status
                    .as_deref()
                    .map(ProviderAuthStatus::parse)
                    .unwrap_or_else(|| {
                        if err
                            .as_deref()
                            .is_some_and(crate::usage_limits::authentication_error)
                        {
                            ProviderAuthStatus::Unauthenticated
                        } else if !catalog.models.is_empty() {
                            ProviderAuthStatus::Authenticated
                        } else {
                            ProviderAuthStatus::Unknown
                        }
                    });
                (
                    installed,
                    err,
                    auth,
                    catalog.update_required_models.clone(),
                    !catalog.models.is_empty(),
                    catalog.provider_message.clone(),
                )
            };
            let is_available = id != "claude" || installed;
            let status = if let Some(explicit) = (self.client.is_some())
                .then(|| self.client_views["nativeCatalogs"][id]["status"].as_str())
                .flatten()
            {
                ProviderStatusKind::parse(explicit)
            } else if !installed || error.is_some() {
                if error
                    .as_deref()
                    .is_some_and(|e| e.contains("timed out") || e.contains("not available"))
                    && has_models
                {
                    ProviderStatusKind::Warning
                } else if !installed || error.is_some() {
                    ProviderStatusKind::Error
                } else {
                    ProviderStatusKind::Ready
                }
            } else {
                ProviderStatusKind::Ready
            };
            entries.push(ProviderInstanceEntry {
                instance_id: id.into(),
                driver_kind: id.into(),
                display_name: name.into(),
                continuation_group_key: None,
                enabled: true,
                installed,
                status,
                auth_status,
                can_setup: false,
                is_available,
                is_acp: false,
                message: error.or(provider_message),
                compatibility_broken_message: None,
                update_required_models,
            });
        }

        // Built-in Antigravity (KSG provider instance): `agy` on PATH, no ACP tag.
        if self.builtin_cli_available("antigravity")
            && !self.client_views["nativeCatalogs"]["antigravity"]["name"].is_string()
            && !self
                .harnesses
                .agents
                .iter()
                .any(|agent| agent.id == "antigravity")
        {
            entries.push(ProviderInstanceEntry {
                instance_id: "antigravity".into(),
                driver_kind: "antigravity".into(),
                display_name: "Antigravity".into(),
                continuation_group_key: None,
                enabled: true,
                installed: true,
                status: ProviderStatusKind::Ready,
                auth_status: ProviderAuthStatus::Unknown,
                can_setup: true,
                is_available: true,
                is_acp: false,
                message: None,
                compatibility_broken_message: None,
                update_required_models: vec![],
            });
        }

        // Built-in Cursor (`agent acp`): `cursor-agent`/`agent` resolved from env or PATH.
        if self.builtin_cli_available("cursor")
            && !self.client_views["nativeCatalogs"]["cursor"]["name"].is_string()
            && !self
                .harnesses
                .agents
                .iter()
                .any(|agent| agent.id == "cursor")
        {
            let error = self.native_catalog_status("cursor").1.map(str::to_owned);
            entries.push(ProviderInstanceEntry {
                instance_id: "cursor".into(),
                driver_kind: "cursor".into(),
                display_name: "Cursor".into(),
                continuation_group_key: None,
                enabled: true,
                installed: true,
                status: if error.is_some() {
                    ProviderStatusKind::Warning
                } else {
                    ProviderStatusKind::Ready
                },
                auth_status: ProviderAuthStatus::Unknown,
                can_setup: false,
                is_available: true,
                is_acp: false,
                message: error,
                compatibility_broken_message: None,
                update_required_models: vec![],
            });
        }

        // ACP agents
        let projected_agents = self.client_views["nativeCatalogs"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(id, value)| Some((id.as_str(), value["name"].as_str()?)));
        let local_agents = self
            .harnesses
            .agents
            .iter()
            .map(|agent| (agent.id.as_str(), agent.name.as_str()));
        for (id, name) in projected_agents.chain(local_agents) {
            if seen
                .iter()
                .any(|(earlier_id, earlier_name)| earlier_id == id && earlier_name == name)
            {
                continue;
            }
            seen.push((id.into(), name.into()));
            let error = self.native_catalog_status(id).1.map(str::to_owned);
            entries.push(ProviderInstanceEntry {
                instance_id: id.into(),
                driver_kind: id.into(),
                display_name: name.into(),
                continuation_group_key: None,
                enabled: true,
                installed: true,
                status: if error.is_some() {
                    ProviderStatusKind::Warning
                } else {
                    ProviderStatusKind::Ready
                },
                auth_status: ProviderAuthStatus::Unknown,
                can_setup: false,
                is_available: true,
                is_acp: true,
                message: error,
                compatibility_broken_message: None,
                update_required_models: vec![],
            });
        }

        entries
    }

    pub fn picker_models_by_instance(
        &self,
    ) -> std::collections::BTreeMap<String, Vec<crate::model_picker::PickerModelOption>> {
        self.picker_models_for_entries(&self.provider_instance_entries())
    }

    pub(crate) fn picker_models_for_entries(
        &self,
        entries: &[crate::model_picker::ProviderInstanceEntry],
    ) -> std::collections::BTreeMap<String, Vec<crate::model_picker::PickerModelOption>> {
        #[cfg(test)]
        PICKER_MAP_BUILDS.with(|count| count.set(count.get() + 1));
        entries
            .iter()
            .map(|entry| {
                let models = self.picker_model_refs(entry);
                (
                    entry.instance_id.clone(),
                    models.iter().map(picker_option).collect(),
                )
            })
            .collect()
    }

    fn composer_model_refs(&self, instance: &str) -> ModelRefs<'_> {
        let t = self.current();
        let cwd = self.catalog_workspace_cwd(self.active);
        if (instance != t.harness || t.pi.is_none() || instance == "cursor")
            && let Some(dir) = cwd.as_deref()
            && let Some(models) = if self.client.is_some() {
                self.projected_native_catalogs
                    .get(instance)
                    .and_then(|catalog| catalog.workspace_snapshots.get(dir))
                    .map(Vec::as_slice)
            } else {
                self.native_catalog(instance)
                    .and_then(|catalog| catalog.workspace_snapshots.get(dir))
                    .map(Vec::as_slice)
            }
        {
            return ModelRefs::Slice(models);
        }
        if instance == "cursor"
            && !self
                .harnesses
                .agents
                .iter()
                .any(|agent| agent.id == instance)
            && !self.client_views["nativeCatalogs"][instance]["name"].is_string()
        {
            let catalog = if self.client.is_some() {
                self.projected_native_catalogs
                    .get(instance)
                    .and_then(|catalog| catalog.models_for_workspace(cwd.as_deref()))
            } else {
                self.cursor_catalog.models_for_workspace(cwd.as_deref())
            };
            if let Some(models) = catalog.filter(|models| !models.is_empty()) {
                return ModelRefs::Slice(models);
            }
        }
        let skip_draft_cached_models =
            t.client_draft && self.projected_native_catalogs.contains_key(instance);
        if instance == t.harness
            && !t.models.is_empty()
            && !skip_draft_cached_models
            && (t.pi.is_some() || !self.acp_catalogs.contains_key(instance))
        {
            ModelRefs::Slice(&t.models)
        } else {
            self.harness_model_refs_for_workspace(instance, cwd.as_deref(), t.project.is_none())
        }
    }

    fn picker_model_refs(
        &self,
        entry: &crate::model_picker::ProviderInstanceEntry,
    ) -> PickerModelRefs<'_> {
        let t = self.current();
        let models = self.composer_model_refs(&entry.instance_id);
        let default = (entry.is_acp && models.iter().next().is_none()).then(|| AgentModel {
            provider: entry.instance_id.clone(),
            id: "default".into(),
            name: "Default".into(),
            is_default: true,
            ..Default::default()
        });
        // Retain active thread's model if unavailable / uncatalogued. Aliases
        // count as catalogued: a saved `opus` selects Claude Opus 5.5's row.
        let retained = (entry.instance_id == t.harness
            && !t.model_id.is_empty()
            && !models.iter().chain(&default).any(|m| {
                m.id == t.model_id
                    || m.aliases
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case(&t.model_id))
            }))
        .then(|| AgentModel {
            provider: if t.harness == "pi" {
                "cpa".into()
            } else {
                t.harness.clone()
            },
            id: t.model_id.clone(),
            name: if t.model.is_empty() {
                t.model_id.clone()
            } else {
                t.model.clone()
            },
            is_unavailable: true,
            ..Default::default()
        });
        PickerModelRefs {
            models,
            default,
            retained,
        }
    }

    pub(crate) fn picker_model_option(
        &self,
        entry: &crate::model_picker::ProviderInstanceEntry,
        id: &str,
    ) -> Option<crate::model_picker::PickerModelOption> {
        self.picker_model_refs(entry)
            .iter()
            .find(|m| m.id == id)
            .map(picker_option)
    }

    pub fn open_model_picker(&mut self, picker: SettingsPicker) {
        self.settings_picker = Some(picker);
        self.open_option_menu = None;
        self.thinking_drag = None;
        if picker == SettingsPicker::Model {
            self.model_search.replace_text("");
            let entries = self.provider_instance_entries();
            let options_by_instance = self.picker_models_for_entries(&entries);
            let favorites = self.effective_model_favorites().to_vec();
            let t = self.current();
            let active_instance = t.harness.clone();
            let active_model = t.model_id.clone();
            let locked = self.locked_provider_for_current();
            let initial_tab = crate::model_picker::initial_model_picker_tab(
                &entries,
                &options_by_instance,
                &favorites,
                &active_instance,
                &active_model,
                locked.as_deref(),
            );
            self.model_picker_tab = Some(initial_tab);
            self.expanded_legacy_instances.clear();
            let active_is_legacy = options_by_instance
                .get(&active_instance)
                .and_then(|opts| opts.iter().find(|o| o.slug == active_model))
                .is_some_and(|o| o.is_legacy);
            let active_is_fav = favorites
                .iter()
                .any(|f| f.provider == active_instance && f.model == active_model);
            if active_is_legacy && !active_is_fav {
                self.expanded_legacy_instances.insert(active_instance);
            }
        }
        self.refresh_catalog_on_open();
    }

    pub fn project_default_model_selection(
        &self,
        harness: &str,
    ) -> Option<crate::model_selection::ModelSelection> {
        let def = self.thread_defaults.models.get(harness)?;
        if def.model.trim().is_empty() {
            return None;
        }
        let mut options = Vec::new();
        if !def.effort.trim().is_empty() {
            let reasoning_id = if harness == "pi" {
                "thinking"
            } else if harness == "codex" {
                "reasoningEffort"
            } else {
                "effort"
            };
            options.push(crate::model_selection::ProviderOptionSelection {
                id: reasoning_id.into(),
                value: crate::model_selection::OptionValue::Str(def.effort.clone()),
            });
        }
        Some(crate::model_selection::ModelSelection {
            instance_id: harness.to_owned(),
            model: def.model.clone(),
            options,
        })
    }

    pub fn resolved_composer_model_selection(
        &self,
    ) -> (
        String,
        String,
        Vec<crate::model_selection::ProviderOptionSelection>,
    ) {
        self.resolve_composer_model(&self.provider_instance_entries())
    }

    pub(crate) fn resolve_composer_model(
        &self,
        entries: &[crate::model_picker::ProviderInstanceEntry],
    ) -> (
        String,
        String,
        Vec<crate::model_selection::ProviderOptionSelection>,
    ) {
        let t = self.current();
        let locked_provider = self.locked_provider_for_current();
        let thread_sel = t.thread_model_selection();
        let project_sel = self.project_default_model_selection(&t.harness);
        let draft_active = t.draft_model_state.active_provider.as_deref();
        let runtime_id = t.has_started_session().then_some(t.harness.as_str());
        let thread_id = thread_sel.as_ref().map(|s| s.instance_id.as_str());
        let project_id = project_sel.as_ref().map(|s| s.instance_id.as_str());
        let resolved_prov = crate::model_selection::resolve_composer_provider_selection(
            entries,
            &[draft_active, runtime_id, thread_id, project_id],
            locked_provider.as_deref(),
            runtime_id.or(thread_id),
        );
        let selected_instance_id = resolved_prov
            .selected_instance_id
            .clone()
            .unwrap_or_else(|| t.harness.clone());
        let options_for = |id: &str| {
            entries
                .iter()
                .find(|entry| entry.instance_id == id)
                .map(|entry| self.picker_model_refs(entry))
                .unwrap_or_default()
        };
        let selected = options_for(&selected_instance_id);
        let requested = resolved_prov
            .requested_instance_id
            .as_deref()
            .filter(|id| *id != selected_instance_id)
            .map(options_for)
            .unwrap_or_default();
        let empty = PickerModelRefs::default();
        let eff = crate::model_selection::derive_effective_composer_model_state(
            Some(&t.draft_model_state),
            entries,
            |id| {
                (if id == Some(selected_instance_id.as_str()) {
                    &selected
                } else if id.is_some() && id == resolved_prov.requested_instance_id.as_deref() {
                    &requested
                } else {
                    &empty
                })
                .iter()
                .map(|model| crate::model_selection::ModelOptionRef {
                    slug: &model.id,
                    name: &model.name,
                    aliases: &model.aliases,
                    is_default: model.is_default,
                    is_custom: model.is_custom,
                })
            },
            &resolved_prov.requested_driver_kind,
            Some(&selected_instance_id),
            resolved_prov.requested_instance_id.as_deref(),
            thread_sel.as_ref(),
            project_sel.as_ref(),
        );
        let opts = eff
            .model_options
            .get(&selected_instance_id)
            .cloned()
            .unwrap_or_default();
        (selected_instance_id, eff.selected_model, opts)
    }

    fn current_options_for_sticky(&self) -> Vec<crate::model_selection::ProviderOptionSelection> {
        use crate::model_selection::{OptionValue, ProviderOptionSelection};
        let t = self.current();
        let mut opts = t
            .draft_model_state
            .model_selection_by_provider
            .get(&t.harness)
            .map(|s| s.options.clone())
            .unwrap_or_default();
        for opt in &t.model_options {
            opts.retain(|o| o.id != opt.id);
            opts.push(opt.clone());
        }
        let reasoning_id = if t.harness == "pi" {
            "thinking"
        } else if t.harness == "codex" {
            "reasoningEffort"
        } else {
            "effort"
        };
        if !t.thinking.is_empty() {
            opts.retain(|o| o.id != reasoning_id);
            opts.push(ProviderOptionSelection {
                id: reasoning_id.into(),
                value: OptionValue::Str(t.thinking.clone()),
            });
        }
        if let Some(tier) = t.harness_state["serviceTier"].as_str() {
            opts.retain(|o| o.id != "serviceTier");
            opts.push(ProviderOptionSelection {
                id: "serviceTier".into(),
                value: OptionValue::Str(tier.to_owned()),
            });
        }
        opts
    }

    pub fn select_model_from_picker(&mut self, instance_id: &str, model_id: &str) {
        let entries = self.provider_instance_entries();
        let models_by_instance = self.picker_models_for_entries(&entries);
        let mut draft_state = self.current().draft_model_state.clone();
        let mut sticky = self.preferences.sticky_composer.clone();
        let Some(next) = crate::model_selection::select_model_from_picker(
            &mut draft_state,
            &mut sticky,
            &entries,
            &models_by_instance,
            instance_id,
            model_id,
        ) else {
            return;
        };
        if self.current().harness != instance_id {
            self.choose_harness(instance_id);
        }
        let provider = if instance_id == "pi" {
            "cpa".to_owned()
        } else {
            instance_id.to_owned()
        };
        if instance_id == "cursor" {
            self.sync_cursor_catalog_models(self.active);
        }
        self.current_mut().set_model(provider, next.model.clone());
        for opt in &next.options {
            self.current_mut()
                .set_option(opt.id.clone(), opt.value.clone());
        }
        draft_state.additional_models.clear();
        self.current_mut().draft_model_state = draft_state;
        self.current_mut().fallback_active = None;
        self.current_mut().fallback_explicit_override = true;
        self.composer_notices.sync("fallback-model", None);
        self.preferences.sticky_composer = sticky;
        self.reconcile_current_access_mode();
        self.checkpoint();
    }

    pub fn multiple_models_enabled(&self) -> bool {
        let thread = self.current();
        !self.demo
            && thread.is_draft()
            && !thread.worktree_bootstrapping()
            && !thread.native.queue_held
            && matches!(
                self.composer_vcs_controls(),
                crate::model::ComposerVcsControls::Editable { .. }
            )
    }

    pub fn is_multi_model_active(&self) -> bool {
        self.multiple_models_enabled()
            && !self
                .current()
                .draft_model_state
                .additional_models
                .is_empty()
    }

    pub fn multi_model_selections(&self) -> Vec<crate::model_selection::ModelSelection> {
        let (instance_id, model, options) = self.resolved_composer_model_selection();
        self.multi_model_selections_from(crate::model_selection::ModelSelection {
            instance_id,
            model,
            options,
        })
    }

    pub(crate) fn multi_model_selections_from(
        &self,
        primary: crate::model_selection::ModelSelection,
    ) -> Vec<crate::model_selection::ModelSelection> {
        if !self.is_multi_model_active() {
            return vec![primary];
        }
        crate::model_selection::multi_model_selections(
            &primary,
            &self.current().draft_model_state.additional_models,
        )
    }

    pub fn toggle_model_in_multi_selection(&mut self, instance_id: &str, model_id: &str) {
        if !self.multiple_models_enabled() {
            return;
        }
        let (primary_instance, primary_model, primary_options) =
            self.resolved_composer_model_selection();
        let primary = crate::model_selection::ModelSelection {
            instance_id: primary_instance,
            model: primary_model,
            options: primary_options,
        };
        let entries = self.provider_instance_entries();
        let models_by_instance = self.picker_models_for_entries(&entries);
        let additional = self.current().draft_model_state.additional_models.clone();
        match crate::model_selection::toggle_multi_model_selection(
            &primary,
            &additional,
            &entries,
            &models_by_instance,
            instance_id,
            model_id,
        ) {
            crate::model_selection::MultiModelToggleResult::Unchanged => {}
            crate::model_selection::MultiModelToggleResult::Updated {
                new_primary,
                additional,
            } => {
                if let Some(next_primary) = new_primary {
                    self.select_model_from_picker(&next_primary.instance_id, &next_primary.model);
                    if !next_primary.options.is_empty() {
                        for opt in &next_primary.options {
                            self.current_mut()
                                .set_option(opt.id.clone(), opt.value.clone());
                        }
                        crate::model_selection::set_model_selection(
                            &mut self.current_mut().draft_model_state,
                            Some(next_primary),
                            true,
                            true,
                        );
                    }
                    self.current_mut().draft_model_state.additional_models = additional;
                } else {
                    if self
                        .current()
                        .draft_model_state
                        .model_selection_by_provider
                        .get(&primary.instance_id)
                        .is_none()
                    {
                        crate::model_selection::set_model_selection(
                            &mut self.current_mut().draft_model_state,
                            Some(primary),
                            true,
                            true,
                        );
                    } else {
                        self.current_mut()
                            .draft_model_state
                            .model_selection_explicit = true;
                    }
                    self.current_mut().draft_model_state.additional_models = additional;
                }
                if !self
                    .current()
                    .draft_model_state
                    .additional_models
                    .is_empty()
                    && self.current().base_branch.is_none()
                    && let Some(key) = self.thread_vcs_repo_key(self.current())
                    && let Some(entry) = self.vcs_branches.get(&key)
                    && let Some(default_branch) =
                        crate::branch_toolbar::default_worktree_base_branch(&entry.refs, None)
                {
                    self.current_mut().base_branch = Some(default_branch);
                }
                self.checkpoint();
            }
        }
    }

    pub fn set_composer_thinking_sticky(&mut self, level: String) {
        let prompt_ultrathink =
            crate::descriptors::read_prompt_ultrathink(&self.current().draft.text);
        if prompt_ultrathink == crate::descriptors::PromptUltrathink::Body {
            return;
        }
        if prompt_ultrathink == crate::descriptors::PromptUltrathink::Prefix {
            let stripped = crate::descriptors::strip_ultrathink_prefix(&self.current().draft.text);
            self.current_mut().draft.replace_text(&stripped);
        }
        self.current_mut().set_thinking(level);
        let instance_id = self.current().harness.clone();
        let model = self.current().model_id.clone();
        let opts = self.current_options_for_sticky();
        let mut draft_state = self.current().draft_model_state.clone();
        let mut sticky = self.preferences.sticky_composer.clone();
        crate::model_selection::set_provider_model_options(
            &mut draft_state,
            &mut sticky,
            &instance_id,
            &instance_id,
            (!model.is_empty()).then_some(model.as_str()),
            opts,
            true,
        );
        self.current_mut().draft_model_state = draft_state;
        self.preferences.sticky_composer = sticky;
        self.checkpoint();
    }

    pub fn set_composer_service_tier_sticky(&mut self, tier: String) {
        self.set_composer_option_sticky(
            "serviceTier".into(),
            crate::model_selection::OptionValue::Str(tier),
        );
    }

    pub fn supports_model_options(&self) -> bool {
        self.client.as_ref().is_none_or(|client| {
            client.supports_capability(&self.current().machine, crate::protocol::MODEL_OPTIONS)
        })
    }

    pub fn set_composer_option_sticky(
        &mut self,
        id: String,
        value: crate::model_selection::OptionValue,
    ) {
        let descriptors = self.active_model_descriptors();
        let controls = crate::descriptors::split_column_and_footer_controls(&descriptors);
        let is_primary_reasoning = controls.effort.as_ref().is_some_and(|e| e.id() == id)
            || matches!(
                id.as_str(),
                "thinking" | "effort" | "reasoningEffort" | "reasoning" | "thought_level"
            );
        if self.current().remote.is_some() && !self.supports_model_options() {
            if is_primary_reasoning && let Some(level) = value.as_str() {
                self.set_composer_thinking_sticky(level.to_owned());
            } else if id == "serviceTier"
                && let Some(tier) = value.as_str()
            {
                self.current_mut().set_service_tier(tier.to_owned());
            } else {
                self.current_mut().error = Some(
                    "This machine does not support model options yet. Update it to use this setting."
                        .into(),
                );
            }
            return;
        }
        if is_primary_reasoning {
            let prompt_ultrathink =
                crate::descriptors::read_prompt_ultrathink(&self.current().draft.text);
            if prompt_ultrathink == crate::descriptors::PromptUltrathink::Body {
                return;
            }
            if prompt_ultrathink == crate::descriptors::PromptUltrathink::Prefix {
                let stripped =
                    crate::descriptors::strip_ultrathink_prefix(&self.current().draft.text);
                self.current_mut().draft.replace_text(&stripped);
            }
        }
        self.current_mut().set_option(id, value);
        let instance_id = self.current().harness.clone();
        let model = self.current().model_id.clone();
        let opts = self.current_options_for_sticky();
        let mut draft_state = self.current().draft_model_state.clone();
        let mut sticky = self.preferences.sticky_composer.clone();
        crate::model_selection::set_provider_model_options(
            &mut draft_state,
            &mut sticky,
            &instance_id,
            &instance_id,
            (!model.is_empty()).then_some(model.as_str()),
            opts,
            true,
        );
        self.current_mut().draft_model_state = draft_state;
        self.preferences.sticky_composer = sticky;
        self.checkpoint();
    }

    pub fn active_model_descriptors(&self) -> Vec<crate::descriptors::ProviderOptionDescriptor> {
        let (resolved_instance, resolved_model_id, resolved_opts) =
            self.resolved_composer_model_selection();
        self.model_descriptors(&resolved_instance, &resolved_model_id, &resolved_opts)
    }

    pub(crate) fn model_descriptors(
        &self,
        resolved_instance: &str,
        resolved_model_id: &str,
        resolved_opts: &[crate::model_selection::ProviderOptionSelection],
    ) -> Vec<crate::descriptors::ProviderOptionDescriptor> {
        let t = self.current();
        let provider = if resolved_instance == "pi" {
            "cpa"
        } else {
            resolved_instance
        };
        let mut caps = self
            .composer_model_refs(resolved_instance)
            .iter()
            .find(|m| m.provider == provider && m.id == resolved_model_id)
            .map(|m| m.capabilities.clone())
            .unwrap_or_default();
        if resolved_instance == "pi"
            && resolved_instance == t.harness
            && resolved_model_id == t.model_id
            && !t.thinking_levels.is_empty()
            && let Some(crate::descriptors::ProviderOptionDescriptor::Select { options, .. }) = caps
                .option_descriptors
                .iter_mut()
                .find(|d| d.id() == "thinking")
        {
            let existing: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
            let live: Vec<&str> = t.thinking_levels.iter().map(String::as_str).collect();
            if existing != live {
                *options = t
                    .thinking_levels
                    .iter()
                    .map(|lvl| crate::descriptors::ProviderOptionChoice {
                        id: lvl.clone(),
                        label: crate::descriptors::pi_thinking_label(lvl).into(),
                        description: None,
                        is_default: lvl == "medium",
                    })
                    .collect();
            }
        }
        let mut selections =
            crate::descriptors::with_implicit_fast_mode_default(&caps, Some(resolved_opts));
        for opt in &t.model_options {
            if !selections.iter().any(|s| s.id == opt.id) {
                selections.push(opt.clone());
            }
        }
        crate::descriptors::get_provider_option_descriptors(&caps, &selections)
    }

    pub(super) fn apply_sticky_composer_to_draft(&mut self, index: usize) {
        let mut sticky = self.preferences.sticky_composer.clone();
        if sticky.sticky_active_provider.is_none()
            && sticky.sticky_model_selection_by_provider.is_empty()
        {
            return;
        }
        let cwd = self.catalog_workspace_cwd(index);
        let projectless = self.threads[index].project.is_none();
        sticky
            .sticky_model_selection_by_provider
            .retain(|instance_id, sel| {
                let provider = if instance_id == "pi" {
                    "cpa"
                } else {
                    instance_id.as_str()
                };
                self.harness_model_refs_for_workspace(instance_id, cwd.as_deref(), projectless)
                    .iter()
                    .any(|m| m.provider == provider && m.id == sel.model)
            });
        if let Some(active) = sticky.sticky_active_provider.as_deref()
            && !sticky
                .sticky_model_selection_by_provider
                .contains_key(active)
        {
            sticky.sticky_active_provider = None;
        }
        crate::model_selection::apply_sticky_state(
            &mut self.threads[index].draft_model_state,
            &sticky,
        );
        if self.threads[index].selected_model.is_none()
            && let Some(active) = &sticky.sticky_active_provider
            && let Some(sel) = sticky.sticky_model_selection_by_provider.get(active)
            && self.validate_harness(active).is_ok()
        {
            let provider = if active == "pi" {
                "cpa".to_owned()
            } else {
                active.clone()
            };
            let models = self.harness_models_for_thread(index, active);
            if models
                .iter()
                .any(|m| m.provider == provider && m.id == sel.model)
            {
                let thread = &mut self.threads[index];
                thread.harness = active.clone();
                thread.models = models;
                thread.set_model(provider, sel.model.clone());
            }
        }
        let harness = self.threads[index].harness.clone();
        let model_id = self.threads[index].model_id.clone();
        let remembered = sticky
            .sticky_options_by_model_by_provider
            .get(&harness)
            .and_then(|by_model| by_model.get(&model_id))
            .or_else(|| {
                sticky
                    .sticky_model_selection_by_provider
                    .get(&harness)
                    .map(|sel| &sel.options)
            });
        if let Some(opts) = remembered {
            let thread = &mut self.threads[index];
            if thread.selected_thinking.is_none()
                && let Some(level) = opts
                    .iter()
                    .find(|o| matches!(o.id.as_str(), "thinking" | "effort" | "reasoningEffort"))
                    .and_then(|o| o.value.as_str())
                    .map(str::to_owned)
            {
                thread.set_thinking(level);
            }
            if thread.harness_state["serviceTier"].is_null()
                && let Some(tier) = opts
                    .iter()
                    .find(|o| o.id == "serviceTier")
                    .and_then(|o| o.value.as_str())
                    .map(str::to_owned)
            {
                thread.set_service_tier(tier);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_harness_availability_uses_the_servers_installation_proof() {
        let mut state = State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        for harness in ["codex", "claude"] {
            state.client_views["nativeCatalogs"][harness] = json!({
                "installed":true, "ready":true, "error":null,
                "models":[{"provider":harness,"id":"fixture"}]
            });
            let entry = state
                .provider_instance_entries()
                .into_iter()
                .find(|entry| entry.instance_id == harness)
                .unwrap();
            assert!(entry.installed && entry.is_available);
            assert_eq!(entry.status, crate::model_picker::ProviderStatusKind::Ready);
            state.client_views["nativeCatalogs"][harness] = json!({
                "installed":false, "error":"Not installed on the selected machine.", "models":[]
            });
            let entry = state
                .provider_instance_entries()
                .into_iter()
                .find(|entry| entry.instance_id == harness)
                .unwrap();
            assert!(
                !entry.installed,
                "a local CLI must not override remote installation status"
            );
        }
    }

    #[test]
    fn native_client_models_are_reused_until_the_owning_catalog_changes() {
        let mut state = State::new(PathBuf::new(), false);
        let (client, _) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        state.machines.selected = "peer".into();
        state.provider_availability.last = Some(crate::harness::Availability {
            antigravity: true,
            ..Default::default()
        });
        state.current_mut().harness = "cursor".into();
        state.current_mut().model_id = "alias".into();
        let projected = |id: &str| {
            json!({
                "harnessIds":["pi","codex","claude","cursor"],
                "nativeCatalogs":{"cursor":{"installed":true, "models":[AgentModel {
                    provider: "cursor".into(),
                    id: id.into(),
                    name: "Fixture".into(),
                    aliases: vec!["alias".into()],
                    ..Default::default()
                }]}}
            })
        };
        let first = projected("before");
        state.update_projected_native_models(&first);
        state.client_views = first;
        assert_eq!(state.resolved_composer_model_selection().1, "before");
        let models = state.projected_native_catalogs["cursor"].models.as_ptr();
        assert!(!state.builtin_cli_available("antigravity"));

        let mut same_models = state.client_views.clone();
        same_models["nativeCatalogs"]["cursor"]["loading"] = json!(true);
        state.update_projected_native_models(&same_models);
        state.client_views = same_models;
        assert_eq!(
            state.projected_native_catalogs["cursor"].models.as_ptr(),
            models
        );

        let refreshed = projected("after");
        state.update_projected_native_models(&refreshed);
        state.client_views = refreshed;
        assert_eq!(state.resolved_composer_model_selection().1, "after");
        let mut removed = state.client_views.clone();
        removed["harnessIds"] = json!(["pi", "codex", "claude"]);
        state.client_views = removed;
        assert!(
            !state.builtin_cli_available("cursor"),
            "stale catalog is not proof of a current install"
        );
    }

    #[test]
    fn draft_pickers_and_capabilities_share_prompt_free_native_models() {
        let mut state = State::new(PathBuf::new(), false);
        state.current_mut().harness = "pi".into();
        for harness in ["codex", "claude"] {
            let model = AgentModel::from_harness_value(&json!({
                "provider":harness,"id":"fixture","name":"Fixture","thinkingLevels":["low","high"]
            }),harness).unwrap();
            if harness == "codex" {
                state.codex_catalog.models = vec![model];
            } else {
                state.claude_catalog.models = vec![model];
            }
            state.choose_harness(harness);
            assert!(state.current().pi.is_none());
            assert!(state.current().session.is_none());
            assert_eq!(state.current().models[0].thinking_levels, ["low", "high"]);
            let mut capabilities = json!({"models":[],"runtime":{}});
            state.add_harness_capabilities(None, &mut capabilities);
            assert!(
                capabilities["models"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|model| model["harness"] == harness && model["id"] == "fixture")
            );
        }
    }

    #[test]
    fn native_commands_wait_for_initialization_and_rewind_can_clear_server_history() {
        let mut state = State::new(PathBuf::new(), false);
        let (session, commands, _) = Pi::test_connection();
        let thread = state.current_mut();
        thread.harness = "codex".into();
        thread.harness_state["codexNativeExtensions"] = json!(true);
        thread.pi = Some(session);
        thread.initializing = true;
        thread.draft.replace_text("/goal Build fixture");
        state.send();
        assert!(commands.try_recv().is_err());
        let thread = state.current_mut();
        thread.initializing = false;
        thread.drive_session();
        let native = commands
            .try_iter()
            .find(|record| record["type"] == "native_command")
            .unwrap();
        assert_eq!(native["name"], "goal");
        assert_eq!(native["arguments"], "Build fixture");
        thread.server_owned = true;
        thread.apply(json!({"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"Old"}]}}));
        thread.apply(
            json!({"type":"response","command":"get_messages","id":"history","success":true,
            "data":{"messages":[],"replaceHistory":true}}),
        );
        assert!(thread.messages.is_empty());
    }

    #[test]
    fn claude_delegation_capabilities_and_harness_inheritance() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        state.test_connect_pi = Some(Box::new(|_, _| Some(Pi::test_connection().0)));
        state.threads[0].harness = "claude".into();
        state.threads[0].models = vec![
            AgentModel::from_harness_value(
                &json!({"provider":"claude","id":"default","thinkingLevels":["low","high"]}),
                "claude",
            )
            .unwrap(),
        ];
        state.threads[0].set_model("claude".into(), "default".into());
        let capabilities = state
            .mcp_call(1, "orchestrator_capabilities", &json!({}))
            .unwrap();
        assert!(
            capabilities["harnesses"]
                .as_array()
                .unwrap()
                .iter()
                .any(|h| h["id"] == "claude")
        );
        assert!(
            capabilities["models"].as_array().unwrap().iter().any(
                |m| m["harness"] == "claude" && m["reasoning_levels"] == json!(["low", "high"])
            )
        );
        for target in [
            None,
            Some(json!({"harness":"claude","model":"default","options":{"reasoning":"low"}})),
        ] {
            let mut args = json!({"prompt":"Fixture task"});
            if let Some(target) = target {
                args["target"] = target;
            }
            let result = state.mcp_call(1, "delegate_task", &args).unwrap();
            let child = state
                .threads
                .iter()
                .find(|t| Some(t.id as u64) == result["thread_id"].as_u64())
                .unwrap();
            assert_eq!(child.harness, "claude");
        }
    }

    #[test]
    fn codex_slash_compact_is_native_and_acp_keeps_its_own_commands() {
        let mut state = State::new(PathBuf::new(), false);
        let (session, commands, _) = Pi::test_connection();
        let thread = state.current_mut();
        thread.harness = "codex".into();
        thread.model_provider = "codex".into();
        thread.pi = Some(session);
        thread.session = Some("codex-thread".into());
        thread.draft.focused = true;
        thread.draft.replace_text("/comp");
        assert!(
            thread
                .slash_matches()
                .iter()
                .any(|c| c.name == "compact" && c.source == "Codex")
        );
        thread.draft.replace_text("/compact Keep the plan");
        state.send();
        let sent: Vec<_> = commands.try_iter().collect();
        assert!(
            sent.iter()
                .any(|r| r["type"] == "compact" && r["customInstructions"] == "Keep the plan")
        );
        assert!(!sent.iter().any(|r| r["type"] == "prompt"));
        let thread = state.current_mut();
        thread.harness = "fixture-acp".into();
        assert!(!thread.can_compact());
        thread.compact(None);
        assert!(thread.error.as_deref().unwrap().contains("ACP"));
    }

    #[test]
    fn codex_delegation_targets_inheritance_and_capabilities() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        state.test_connect_pi = Some(Box::new(|_, _| Some(Pi::test_connection().0)));
        state.threads[0].harness = "codex".into();
        state.threads[0].models = vec![
            AgentModel::from_harness_value(
                &json!({
                    "provider":"codex","id":"codex-fixture","thinkingLevels":["low","high"]
                }),
                "codex",
            )
            .unwrap(),
        ];
        state.threads[0].set_model("codex".into(), "codex-fixture".into());
        state.threads[0].set_thinking("high".into());
        let capabilities = state
            .mcp_call(1, "orchestrator_capabilities", &json!({}))
            .unwrap();
        assert_eq!(
            capabilities["models"][0]["reasoning_levels"],
            json!(["low", "high"])
        );
        for target in [
            None,
            Some(json!({"harness":"codex","model":"codex-fixture","options":{"reasoning":"low"}})),
        ] {
            let mut args = json!({"prompt":"Reply PONG"});
            if let Some(target) = target {
                args["target"] = target;
            }
            let result = state.mcp_call(1, "delegate_task", &args).unwrap();
            let child = state
                .threads
                .iter()
                .find(|t| Some(t.id as u64) == result["thread_id"].as_u64())
                .unwrap();
            assert_eq!(child.harness, "codex");
            assert_eq!(child.model_provider, "codex");
        }
        let count = state.threads.len();
        assert!(
            state
                .mcp_call(
                    1,
                    "delegate_task",
                    &json!({"prompt":"invalid",
            "target":{"harness":"codex","model":"codex-fixture","options":{"reasoning":"max"}}})
                )
                .is_err()
        );
        assert_eq!(state.threads.len(), count);
        assert!(
            state
                .mcp_call(
                    1,
                    "delegate_task",
                    &json!({"prompt":"invalid",
            "target":{"harness":"missing"}})
                )
                .is_err()
        );
    }

    #[test]
    fn default_harness_persists_and_applies_to_new_threads_and_homes() {
        let mut state = State::new(PathBuf::new(), false);
        assert_eq!(state.default_harness, "codex");
        state.cycle_default_harness();
        assert_eq!(state.default_harness, "claude");
        state
            .current_mut()
            .draft
            .replace_text("Preserve an existing draft");
        state.new_thread_in(None);
        assert_eq!(state.current().harness, "claude");
        state.captains = Captain::presets();
        state.open_captain(0);
        assert_eq!(state.current().harness, "claude");
        assert_eq!(state.current().native.access, AccessMode::ReadOnly);
        let snapshot = state.snapshot();
        state.restore_global(&snapshot).unwrap();
        assert_eq!(state.default_harness, "claude");
    }

    #[test]
    fn codex_captain_sessions_include_the_same_profile_memory_and_diffs_as_pi() {
        let mut state = State::new(PathBuf::new(), false);
        state.demo = true;
        state.captains = Captain::presets();
        state.open_captain(0);
        state
            .add_memory(0, "Essential fixture memory".into(), true)
            .unwrap();
        state
            .set_user_profile(UserProfile {
                name: "Fixture user".into(),
                time_zone: "America/New_York".into(),
                preferences: "Use Rust".into(),
            })
            .unwrap();
        let instructions = format!(
            "{}\n\n{}",
            state.captains[0].prompt(),
            state.current().memory_prompt()
        );
        for source in [
            crate::backend::SessionSource::New,
            crate::backend::SessionSource::Resume("saved".into()),
        ] {
            let start = crate::harness::Start {
                id: "codex".into(),
                workspace: state.data_dir.clone(),
                source,
                instructions: Some(instructions.clone()),
                shina: crate::mcp::INSTRUCTIONS,
                link: None,
                owner: Arc::default(),
                context_sent: true,
                prompt_suggestions: false,
                metadata_only: false,
            };
            let context = crate::codex::developer_instructions(&start);
            assert!(context.contains(crate::mcp::INSTRUCTIONS));
            assert!(context.contains(&state.captains[0].prompt()));
            assert!(context.contains("Essential fixture memory"));
            assert!(context.contains("Fixture user"));
        }
        state.current_mut().harness = "codex".into();
        let (pi, commands, _) = Pi::test_connection();
        state.current_mut().pi = Some(pi);
        // The common Thread::command path adds the memory diff, independent of harness.
        assert!(
            state
                .current_mut()
                .command("prompt", json!({"message":"Check memory"}))
        );
        let prompt = commands.try_recv().unwrap();
        let text = prompt["message"].as_str().unwrap();
        assert!(text.contains("<shina_memory_diff>"));
        assert!(text.contains("Essential fixture memory"));
        state
            .add_memory(0, "New fixture memory".into(), true)
            .unwrap();
        assert!(
            state
                .current_mut()
                .command("prompt", json!({"message":"Check again"}))
        );
        assert!(
            commands.try_recv().unwrap()["message"]
                .as_str()
                .unwrap()
                .contains("New fixture memory")
        );
    }

    #[test]
    fn workspace_fallback_does_not_seed_another_folder_and_empty_snapshot_is_honored() {
        let mut state = State::new(PathBuf::new(), false);
        let project_a = crate::projects::Project::new(1, PathBuf::from("folder-a"));
        let project_b = crate::projects::Project::new(2, PathBuf::from("folder-b"));
        let dir_a = project_a.path.clone();
        let dir_b = project_b.path.clone();
        let id_a = project_a.id;
        let id_b = project_b.id;
        state.projects.entries.push(project_a);
        state.projects.entries.push(project_b);
        state.threads[0].project = Some(id_b);
        state.threads[0].harness = "opencode".into();
        let mut catalog = crate::session::NativeCatalog::default();
        catalog.cwd = Some(dir_a.clone());
        catalog.models_cwd = Some(dir_a.clone());
        catalog.models = vec![AgentModel {
            provider: "opencode".into(),
            id: "model-a".into(),
            name: "Model A".into(),
            ..Default::default()
        }];
        catalog.revision = 1;
        catalog.set_workspace_models(&dir_a, &catalog.models.clone());
        state.acp_catalogs.insert("opencode".into(), catalog);

        state.apply_native_catalogs();
        assert!(
            state.threads[0].models.is_empty(),
            "another folder's fallback catalog must not seed a thread with no workspace snapshot"
        );
        // Switching discovery's active cwd to folder B before discovery replies keeps
        // models_cwd on folder A and must still not leak folder A's models into folder B.
        {
            let catalog = state.acp_catalogs.get_mut("opencode").unwrap();
            catalog.cwd = Some(dir_b.clone());
            catalog.revision += 1;
        }
        state.apply_native_catalogs();
        assert!(
            state.threads[0].models.is_empty(),
            "in-flight discovery for folder B must not leak folder A's cached models into folder B"
        );
        // Switching to folder A copies folder A's snapshot.
        state.threads[0].project = Some(id_a);
        state.threads[0].harness_state["catalogRevision"] = Value::Null;
        state.apply_native_catalogs();
        assert_eq!(state.threads[0].models.len(), 1);
        assert_eq!(state.threads[0].models[0].id, "model-a");

        // An explicitly empty snapshot for folder B clears thread.models and is honored by picker.
        state.threads[0].project = Some(id_b);
        state
            .acp_catalogs
            .get_mut("opencode")
            .unwrap()
            .set_workspace_models(&dir_b, &[]);
        state.apply_native_catalogs();
        assert!(state.threads[0].models.is_empty());
        assert_eq!(state.composer_model_refs("opencode").iter().count(), 0);
        assert_eq!(state.workspace_models(0, "opencode"), Some(vec![]));
    }

    #[test]
    fn cursor_picker_and_set_model_validate_against_the_fresh_workspace_catalog_after_live_acp_models()
     {
        let mut state = State::new(PathBuf::new(), false);
        let project = crate::projects::Project::new(1, PathBuf::from("cursor-workspace"));
        let dir = project.path.clone();
        let project_id = project.id;
        state.projects.entries.push(project);
        state.threads[0].project = Some(project_id);
        state.threads[0].harness = "cursor".into();
        let old_model = AgentModel {
            provider: "cursor".into(),
            id: "old-model".into(),
            name: "Old Model".into(),
            ..Default::default()
        };
        let new_model = AgentModel {
            provider: "cursor".into(),
            id: "new-model".into(),
            name: "New Model".into(),
            ..Default::default()
        };
        state
            .provider_availability
            .last
            .get_or_insert_with(Default::default)
            .cursor = true;
        state.cursor_catalog.installed = true;
        state.cursor_catalog.cwd = Some(dir.clone());
        state.cursor_catalog.models = vec![old_model.clone(), new_model.clone()];
        state
            .cursor_catalog
            .set_workspace_models(&dir, &[old_model.clone(), new_model.clone()]);
        state.apply_native_catalogs();

        // Live ACP session overwrites thread.models with the older session list.
        let (pi, commands, _) = Pi::test_connection();
        state.threads[0].pi = Some(pi);
        state.threads[0].apply(json!({
            "type": "response",
            "command": "get_available_models",
            "success": true,
            "data": {"models": [{"id": "old-model", "name": "Old Model", "provider": "cursor"}]}
        }));
        assert_eq!(state.threads[0].models.len(), 1);
        assert_eq!(state.threads[0].models[0].id, "old-model");

        // Picker still exposes new-model and picking it marks fromCursorCatalog: true.
        let picker = state.composer_model_refs("cursor");
        assert!(picker.iter().any(|model| model.id == "new-model"));
        state.select_model_from_picker("cursor", "new-model");
        let cmd = commands.try_recv().expect("set_model sent");
        assert_eq!(cmd["type"], "set_model");
        assert_eq!(cmd["modelId"], "new-model");
        assert_eq!(cmd["fromCursorCatalog"], true);
    }

    #[test]
    fn cursor_startup_prunes_stale_saved_options_and_drops_rejected_options() {
        let mut state = State::new(PathBuf::new(), false);
        let thread = &mut state.threads[0];
        thread.harness = "cursor".into();
        let (pi, commands, _) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.state_loaded = true;
        thread.history_loaded = true;
        thread.startup_model_applied = true;
        thread.startup_thinking_applied = true;
        thread.model_id = "cursor-pro".into();
        thread.models = vec![AgentModel {
            provider: "cursor".into(),
            id: "cursor-pro".into(),
            name: "Cursor Pro".into(),
            capabilities: crate::descriptors::ModelCapabilities {
                option_descriptors: vec![crate::descriptors::ProviderOptionDescriptor::Select {
                    id: "context".into(),
                    label: "Context".into(),
                    description: None,
                    options: vec![crate::descriptors::ProviderOptionChoice {
                        id: "200k".into(),
                        label: "200K".into(),
                        description: None,
                        is_default: false,
                    }],
                    current_value: Some("200k".into()),
                    prompt_injected_values: vec![],
                    role: None,
                }],
            },
            ..Default::default()
        }];
        // Stale unsupported option + valid advertised option.
        thread.model_options = vec![
            crate::model_selection::ProviderOptionSelection {
                id: "contextWindow".into(),
                value: crate::model_selection::OptionValue::Str("2m".into()),
            },
            crate::model_selection::ProviderOptionSelection {
                id: "contextWindow".into(),
                value: crate::model_selection::OptionValue::Str("200k".into()),
            },
        ];

        thread.drive_session();
        // Stale "2m" was pruned; only supported "200k" was sent.
        let sent = commands.try_recv().expect("supported set_option sent");
        assert_eq!(sent["type"], "set_option");
        assert_eq!(sent["optionId"], "contextWindow");
        assert_eq!(sent["value"], "200k");
        assert_eq!(thread.model_options.len(), 1);

        // If the CLI rejects the option during startup, it is also removed before failing.
        thread.apply(json!({
            "type": "response",
            "command": "set_option",
            "success": false,
            "error": "Rejected tier"
        }));
        assert!(
            thread.model_options.is_empty(),
            "rejected startup option must not remain saved to poison retries"
        );
    }

    #[test]
    fn registered_cursor_live_thread_resolves_fresh_catalog_for_thread_configure() {
        let mut state = State::new(PathBuf::new(), false);
        let project = crate::projects::Project::new(1, PathBuf::from("cursor-acp-workspace"));
        let dir = project.path.clone();
        let project_id = project.id;
        state.projects.entries.push(project);
        state.threads[0].project = Some(project_id);
        state.threads[0].harness = "cursor".into();
        let old_model = AgentModel {
            provider: "cursor".into(),
            id: "old-model".into(),
            name: "Old Model".into(),
            ..Default::default()
        };
        let new_model = AgentModel {
            provider: "cursor".into(),
            id: "new-model".into(),
            name: "New Model".into(),
            ..Default::default()
        };
        // Start a live session with only old-model before fresh discovery lands.
        let (pi, commands, _) = Pi::test_connection();
        state.threads[0].pi = Some(pi);
        state.threads[0].models = vec![old_model.clone()];

        // Fresh discovery on registered ACP cursor adds new-model.
        let mut catalog = crate::session::NativeCatalog::default();
        catalog.installed = true;
        catalog.cwd = Some(dir.clone());
        catalog.models_cwd = Some(dir.clone());
        catalog.models = vec![old_model.clone(), new_model.clone()];
        catalog.set_workspace_models(&dir, &[old_model.clone(), new_model.clone()]);
        state.acp_catalogs.insert("cursor".into(), catalog);

        // Even without calling apply_native_catalogs or select_model_from_picker,
        // the daemon's thread.configure path resolves host catalog membership.
        let cmd = crate::protocol::Command {
            command_id: uuid::Uuid::new_v4(),
            method: "thread.configure".into(),
            params: json!({"provider": "cursor", "model": "new-model"}),
        };
        state
            .execute_command(&cmd)
            .expect("thread.configure succeeds");
        let sent = commands.try_recv().expect("set_model sent");
        assert_eq!(sent["type"], "set_model");
        assert_eq!(sent["modelId"], "new-model");
        assert_eq!(sent["fromCursorCatalog"], true);

        // And apply_native_catalogs keeps live thread.models intact while updating cursor_catalog_models.
        state.threads[0].cursor_catalog_models = None;
        state.apply_native_catalogs();
        assert_eq!(state.threads[0].models.len(), 1);
        assert_eq!(state.threads[0].models[0].id, "old-model");
        assert!(
            state.threads[0]
                .cursor_catalog_models
                .as_ref()
                .is_some_and(|models| models.iter().any(|model| model.id == "new-model"))
        );
    }

    #[test]
    fn cursor_startup_prunes_saved_options_when_handshake_republishes_cached_rows_without_live_controls()
     {
        let mut state = State::new(PathBuf::new(), false);
        let thread = &mut state.threads[0];
        thread.harness = "cursor".into();
        let (pi, commands, _) = Pi::test_connection();
        thread.pi = Some(pi);
        thread.initializing = true;
        thread.state_loaded = true;
        thread.history_loaded = true;
        thread.startup_model_applied = true;
        thread.startup_thinking_applied = true;
        thread.model_id = "cursor-pro".into();
        thread.model_options = vec![crate::model_selection::ProviderOptionSelection {
            id: "contextWindow".into(),
            value: crate::model_selection::OptionValue::Str("1m".into()),
        }];

        // Handshake republishes cached row with stale cached optionDescriptors,
        // but reports empty liveOptionDescriptors (and advertised: false).
        thread.apply(json!({
            "type": "response",
            "command": "get_available_models",
            "success": true,
            "data": {
                "advertised": false,
                "liveOptionDescriptors": [],
                "models": [{
                    "id": "cursor-pro",
                    "name": "Cursor Pro",
                    "provider": "cursor",
                    "capabilities": {
                        "optionDescriptors": [{
                            "kind": "select",
                            "id": "context",
                            "label": "Context",
                            "options": [{"id": "1m", "label": "1M", "isDefault": true}],
                            "currentValue": "1m"
                        }]
                    }
                }]
            }
        }));

        thread.drive_session();
        let sent: Vec<_> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
        assert!(
            !sent.iter().any(|cmd| cmd["type"] == "set_option"),
            "stale cached descriptors must not trigger a failing set_option during startup: {sent:?}"
        );
        assert!(thread.model_options.is_empty());
        assert!(!thread.initializing);
    }

    #[test]
    fn pending_workspace_scan_does_not_leak_previous_folder_into_picker_or_draft_seeding() {
        let mut state = State::new(PathBuf::new(), false);
        let project_a = crate::projects::Project::new(1, PathBuf::from("workspace-a"));
        let project_b = crate::projects::Project::new(2, PathBuf::from("workspace-b"));
        let dir_a = project_a.path.clone();
        let dir_b = project_b.path.clone();
        let id_a = project_a.id;
        let id_b = project_b.id;
        state.projects.entries.push(project_a);
        state.projects.entries.push(project_b);
        state
            .provider_availability
            .last
            .get_or_insert_with(Default::default)
            .cursor = true;

        let cursor_model_a = AgentModel {
            provider: "cursor".into(),
            id: "cursor-model-a".into(),
            name: "Cursor Model A".into(),
            ..Default::default()
        };
        state.cursor_catalog.installed = true;
        state.cursor_catalog.cwd = Some(dir_b.clone());
        state.cursor_catalog.models_cwd = Some(dir_a.clone());
        state.cursor_catalog.models = vec![cursor_model_a.clone()];
        state
            .cursor_catalog
            .set_workspace_models(&dir_a, std::slice::from_ref(&cursor_model_a));

        let acp_model_a = AgentModel {
            provider: "opencode".into(),
            id: "opencode-model-a".into(),
            name: "OpenCode Model A".into(),
            ..Default::default()
        };
        let mut acp_catalog = crate::session::NativeCatalog::default();
        acp_catalog.installed = true;
        acp_catalog.cwd = Some(dir_b.clone());
        acp_catalog.models_cwd = Some(dir_a.clone());
        acp_catalog.models = vec![acp_model_a.clone()];
        acp_catalog.set_workspace_models(&dir_a, std::slice::from_ref(&acp_model_a));
        state.acp_catalogs.insert("opencode".into(), acp_catalog);

        // Thread 0 lives in folder A and holds folder A's discovered models.
        state.threads[0].project = Some(id_a);
        state.threads[0].harness = "cursor".into();
        state.threads[0].models = vec![cursor_model_a.clone()];

        // Thread 1 lives in folder B while folder B's discovery is still pending.
        let mut thread_b = Thread::new(2);
        thread_b.project = Some(id_b);
        thread_b.harness = "cursor".into();
        state.threads.push(thread_b);
        state.active = 1;

        assert_eq!(
            state.composer_model_refs("cursor").iter().count(),
            0,
            "picker fallback must not expose folder A's built-in Cursor models in folder B"
        );
        assert_eq!(
            state.composer_model_refs("opencode").iter().count(),
            0,
            "picker fallback must not expose folder A's ACP models in folder B"
        );

        // Draft seeding via inherit_composer must not copy folder A's models into folder B.
        let seed = json!({
            "harness": "cursor",
            "model": ["cursor", "cursor-model-a"],
            "access": "supervised"
        });
        state.inherit_composer(1, &seed);
        assert!(
            state.threads[1].models.is_empty(),
            "inherit_composer must not seed folder B's draft with folder A's Cursor models"
        );

        // Sticky composer selection from folder A must not leak into folder B's draft_model_state
        // or resolve as folder B's effective composer model while B's scan is pending.
        state.preferences.sticky_composer.sticky_active_provider = Some("cursor".into());
        state
            .preferences
            .sticky_composer
            .sticky_model_selection_by_provider
            .insert(
                "cursor".into(),
                crate::model_selection::ModelSelection {
                    instance_id: "cursor".into(),
                    model: "cursor-model-a".into(),
                    options: vec![],
                },
            );
        state.threads[1].harness = "cursor".into();
        state.threads[1].selected_model = None;
        state.threads[1].model_id.clear();
        state.threads[1].draft_model_state = Default::default();
        state.apply_sticky_composer_to_draft(1);
        assert!(
            !state.threads[1]
                .draft_model_state
                .model_selection_by_provider
                .contains_key("cursor"),
            "sticky selection from folder A must not be copied into folder B's draft_model_state"
        );
        assert_ne!(
            state.resolved_composer_model_selection().1,
            "cursor-model-a",
            "effective composer model in folder B must not resolve folder A's sticky model"
        );
    }

    #[test]
    fn client_mode_projected_catalogs_isolate_workspaces_while_discovery_is_pending() {
        let mut state = State::new(PathBuf::new(), false);
        state.client = Some(crate::client::Client::new(state.data_dir.clone()));
        let project_a = crate::projects::Project::new(1, PathBuf::from("remote-a"));
        let project_b = crate::projects::Project::new(2, PathBuf::from("remote-b"));
        let dir_a = project_a.path.clone();
        let dir_b = project_b.path.clone();
        let id_a = project_a.id;
        let id_b = project_b.id;
        state.projects.entries.push(project_a);
        state.projects.entries.push(project_b);

        let cursor_model_a = AgentModel {
            provider: "cursor".into(),
            id: "cursor-remote-a".into(),
            name: "Cursor Remote A".into(),
            ..Default::default()
        };
        let mut cursor_server = crate::session::NativeCatalog::default();
        cursor_server.installed = true;
        cursor_server.cwd = Some(dir_b.clone());
        cursor_server.models_cwd = Some(dir_a.clone());
        cursor_server.models = vec![cursor_model_a.clone()];
        cursor_server.set_workspace_models(&dir_a, std::slice::from_ref(&cursor_model_a));

        let acp_model_a = AgentModel {
            provider: "opencode".into(),
            id: "opencode-remote-a".into(),
            name: "OpenCode Remote A".into(),
            ..Default::default()
        };
        let mut acp_server = crate::session::NativeCatalog::from_acp_fixture(
            crate::harness::Agent {
                id: "opencode".into(),
                name: "OpenCode".into(),
                command: "opencode".into(),
                args: vec![],
                env: Default::default(),
            },
            vec![acp_model_a.clone()],
        );
        acp_server.cwd = Some(dir_b.clone());
        acp_server.models_cwd = Some(dir_a.clone());
        acp_server.set_workspace_models(&dir_a, std::slice::from_ref(&acp_model_a));

        let projected = json!({
            "harnessIds": ["pi", "codex", "claude", "cursor", "opencode"],
            "nativeCatalogs": {
                "cursor": cursor_server.snapshot(),
                "opencode": acp_server.snapshot(),
            }
        });
        state.update_projected_native_models(&projected);
        state.client_views = projected;
        state.provider_availability.last = Some(crate::harness::Availability {
            claude: true,
            antigravity: false,
            cursor: true,
        });

        // Client draft in folder B must not inherit folder A's projected models while B is scanning.
        state.threads[0].project = Some(id_b);
        state.threads[0].harness = "cursor".into();
        state.threads[0].client_draft = true;
        state.apply_native_catalogs();
        assert!(state.threads[0].models.is_empty());
        assert_eq!(state.composer_model_refs("cursor").iter().count(), 0);
        assert_eq!(state.composer_model_refs("opencode").iter().count(), 0);

        // Switching to folder A exposes folder A's projected workspace models.
        state.choose_draft_project(Some(id_a));
        assert_eq!(state.threads[0].models.len(), 1);
        assert_eq!(state.threads[0].models[0].id, "cursor-remote-a");
        assert!(
            state
                .composer_model_refs("opencode")
                .iter()
                .any(|model| model.id == "opencode-remote-a")
        );

        // Switching the client draft back from folder A to folder B clears cached models so
        // folder A's models never leak into folder B via the picker or cached thread state.
        state.choose_draft_project(Some(id_b));
        assert!(state.threads[0].models.is_empty());
        assert!(state.threads[0].cursor_catalog_models.is_none());
        assert_eq!(state.composer_model_refs("cursor").iter().count(), 0);
        assert_eq!(state.composer_model_refs("opencode").iter().count(), 0);

        // Even if a draft still held stale models from folder A before a projection refresh,
        // apply_native_catalogs clears them on a catalog miss for folder B.
        state.threads[0].models = vec![cursor_model_a.clone()];
        state.threads[0].cursor_catalog_models = Some(vec![cursor_model_a.clone()]);
        assert_eq!(state.composer_model_refs("cursor").iter().count(), 0);
        state.apply_native_catalogs();
        assert!(state.threads[0].models.is_empty());
        assert!(state.threads[0].cursor_catalog_models.is_none());

        // Linked-machine device-only drafts have no server-reported directory in client_views.
        state.machines.selected = "remote".into();
        assert!(state.thread_directory(0).is_err());

        let seed = json!({
            "harness": "cursor",
            "model": ["cursor", "cursor-remote-a"],
            "access": "supervised"
        });

        // Linked-machine draft in folder B must not inherit folder A's Cursor model.
        state.threads[0].project = Some(id_b);
        state.inherit_composer(0, &seed);
        assert!(state.threads[0].models.is_empty());
        assert_ne!(state.threads[0].model_id, "cursor-remote-a");
        assert_eq!(state.composer_model_refs("cursor").iter().count(), 0);
        assert_eq!(state.composer_model_refs("opencode").iter().count(), 0);

        // Linked-machine projectless draft (unknown workspace -> None) must not treat None as
        // unscoped and inherit folder A's models.
        state.choose_draft_project(None);
        state.inherit_composer(0, &seed);
        assert!(state.threads[0].models.is_empty());
        assert_ne!(state.threads[0].model_id, "cursor-remote-a");
        assert_eq!(state.composer_model_refs("cursor").iter().count(), 0);
        assert_eq!(state.composer_model_refs("opencode").iter().count(), 0);

        // Switching the linked-machine draft to folder A exposes folder A's models, and
        // switching back to folder B clears them again.
        state.threads[0].harness = "cursor".into();
        state.choose_draft_project(Some(id_a));
        assert_eq!(state.threads[0].models.len(), 1);
        assert_eq!(state.threads[0].models[0].id, "cursor-remote-a");
        state.choose_draft_project(Some(id_b));
        assert!(state.threads[0].models.is_empty());
        assert_eq!(state.composer_model_refs("cursor").iter().count(), 0);
    }

    fn leveled_model(harness: &str, id: &str, levels: &[&str]) -> AgentModel {
        AgentModel {
            provider: harness.into(),
            id: id.into(),
            name: id.into(),
            thinking_levels: levels.iter().map(|level| (*level).into()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_linked_draft_with_an_unknown_directory_borrows_no_scratch_thread_models() {
        for harness in ["codex", "claude"] {
            let mut state = State::new(PathBuf::new(), false);
            state.client = Some(crate::client::Client::new(state.data_dir.clone()));
            state.machines.selected = "remote".into();
            let scratch = PathBuf::from("/srv/shina/chats/7");
            let other_scratch = PathBuf::from("/srv/shina/chats/9");
            let model = leveled_model(harness, "scratch-only", &["low", "high"]);

            // The server discovered this model for one scratch chat's folder.
            let mut server = crate::session::NativeCatalog::default();
            server.installed = true;
            server.cwd = Some(scratch.clone());
            server.models_cwd = Some(scratch.clone());
            server.models = vec![model.clone()];
            server.set_workspace_models(&scratch, std::slice::from_ref(&model));
            let mut projected = json!({
                "harnessIds": ["pi", "codex", "claude"],
                "directories": {"7": scratch, "9": other_scratch},
                "nativeCatalogs": {},
            });
            projected["nativeCatalogs"][harness] = server.snapshot();
            state.update_projected_native_models(&projected);
            state.client_views = projected;

            // Thread 0 is the device's projectless draft. The server has not
            // reported a directory for it.
            state.threads[0].client_draft = true;
            state.threads[0].harness = harness.into();
            // Threads 7 and 9 are projected scratch chats with cached rows.
            for id in [7, 9] {
                let mut cached = Thread::new(id);
                cached.harness = harness.into();
                cached.models = vec![model.clone()];
                state.threads.push(cached);
            }
            state.active = 0;
            assert!(state.thread_directory(0).is_err());
            assert!(state.catalog_workspace_cwd(0).is_none());

            assert!(
                state.harness_models_for_cwd(harness, None).is_empty(),
                "{harness}: an unknown workspace must not borrow another scratch folder's rows"
            );
            assert_eq!(state.composer_model_refs(harness).iter().count(), 0);

            state.inherit_composer(
                0,
                &json!({"harness": harness, "model": [harness, "scratch-only"],
                    "thinking": "high", "access": "supervised"}),
            );
            assert!(state.threads[0].models.is_empty());
            assert!(state.threads[0].thinking_levels.is_empty());
            assert_ne!(state.threads[0].model_id, "scratch-only");

            state.threads[0].harness = harness.into();
            state.preferences.sticky_composer.sticky_active_provider = Some(harness.into());
            state
                .preferences
                .sticky_composer
                .sticky_model_selection_by_provider
                .insert(
                    harness.into(),
                    crate::model_selection::ModelSelection {
                        instance_id: harness.into(),
                        model: "scratch-only".into(),
                        options: vec![],
                    },
                );
            state.apply_sticky_composer_to_draft(0);
            assert!(
                !state.threads[0]
                    .draft_model_state
                    .model_selection_by_provider
                    .contains_key(harness)
            );
            assert_ne!(state.threads[0].model_id, "scratch-only");
            state.apply_native_catalogs();
            assert!(state.threads[0].models.is_empty());
            assert!(state.threads[0].thinking_levels.is_empty());

            // The folder that produced the rows still gets them. A linked machine's
            // other folders are separate workspaces, scratch or not.
            assert_eq!(
                state.harness_models_for_cwd(harness, Some(&scratch)),
                [model.clone()]
            );
            state.projected_native_catalogs.clear();
            assert_eq!(
                state.harness_models_for_cwd(harness, Some(&scratch)),
                [model.clone()]
            );
            assert_eq!(
                state.harness_models_for_cwd(harness, Some(&other_scratch)),
                [model.clone()],
                "{harness}: folder 9 reads the rows its own thread cached"
            );
            state.threads[2].models.clear();
            assert!(
                state
                    .harness_models_for_cwd(harness, Some(&other_scratch))
                    .is_empty(),
                "{harness}: folder 7's cached rows must not answer for folder 9"
            );
        }
    }

    #[test]
    fn local_scratch_chats_share_cached_models_and_projects_do_not() {
        for harness in ["codex", "claude"] {
            let mut state = State::new(PathBuf::new(), false);
            let project = crate::projects::Project::new(1, PathBuf::from("a-project"));
            let project_id = project.id;
            state.projects.entries.push(project);
            let model = leveled_model(harness, "cached", &["high"]);
            state.threads[0].harness = harness.into();
            state.threads[0].models = vec![model.clone()];
            state.threads.push(Thread::new(50));
            let mut in_project = Thread::new(51);
            in_project.project = Some(project_id);
            state.threads.push(in_project);

            let sibling = state.catalog_workspace_cwd(1);
            assert_ne!(sibling, state.catalog_workspace_cwd(0));
            assert_eq!(
                state.harness_models_for_cwd(harness, sibling.as_deref()),
                [model.clone()],
                "{harness}: a chat's scratch folder holds no provider config of its own"
            );
            let project_dir = state.catalog_workspace_cwd(2);
            assert!(
                state
                    .harness_models_for_cwd(harness, project_dir.as_deref())
                    .is_empty(),
                "{harness}: a scratch chat's rows must not seed a project"
            );
            assert!(state.harness_models_for_cwd(harness, None).is_empty());

            // The reverse holds too: a project's rows do not reach a scratch chat.
            state.threads[0].project = Some(project_id);
            assert!(
                state
                    .harness_models_for_cwd(harness, sibling.as_deref())
                    .is_empty()
            );
            assert_eq!(
                state.harness_models_for_cwd(harness, project_dir.as_deref()),
                [model.clone()]
            );

            // A linked machine with the same `<data_dir>/chats` profile path never
            // shares sibling scratch folders, nor does a project registered under `chats/`.
            state.threads[0].project = None;
            state.machines.selected = "remote".into();
            assert!(
                state
                    .harness_models_for_cwd(harness, sibling.as_deref())
                    .is_empty(),
                "{harness}: linked machine must not share sibling scratch folders"
            );
            state.machines.selected = "local".into();
            let chat_project = crate::projects::Project::new(2, sibling.clone().unwrap());
            let chat_project_id = chat_project.id;
            state.projects.entries.push(chat_project);
            state.threads[1].project = Some(chat_project_id);
            assert!(
                state.harness_models_for_thread(1, harness).is_empty(),
                "{harness}: project registered under chats/ must not borrow a scratch chat"
            );
        }
    }

    #[test]
    fn switching_a_draft_project_drops_the_old_folders_reasoning_levels_and_commands() {
        let mut state = State::new(PathBuf::new(), false);
        state.client = Some(crate::client::Client::new(state.data_dir.clone()));
        let project_a = crate::projects::Project::new(1, PathBuf::from("levels-a"));
        let project_b = crate::projects::Project::new(2, PathBuf::from("levels-b"));
        let dir_a = project_a.path.clone();
        let id_a = project_a.id;
        let id_b = project_b.id;
        state.projects.entries.push(project_a);
        state.projects.entries.push(project_b);

        let model_a = leveled_model("codex", "model-a", &["low", "high", "xhigh"]);
        let mut server = crate::session::NativeCatalog::default();
        server.installed = true;
        server.cwd = Some(dir_a.clone());
        server.models_cwd = Some(dir_a.clone());
        server.models = vec![model_a.clone()];
        server.set_workspace_models(&dir_a, std::slice::from_ref(&model_a));
        let projected = json!({
            "harnessIds": ["pi", "codex", "claude"],
            "nativeCatalogs": {"codex": server.snapshot()},
        });
        state.update_projected_native_models(&projected);
        state.client_views = projected;

        let draft = &mut state.threads[0];
        draft.client_draft = true;
        draft.harness = "codex".into();
        draft.project = Some(id_a);
        draft.model_provider = "codex".into();
        draft.model_id = "model-a".into();
        state.apply_native_catalogs();
        assert_eq!(state.threads[0].models, [model_a.clone()]);
        assert_eq!(state.threads[0].thinking_levels, ["low", "high", "xhigh"]);
        state.threads[0]
            .native
            .commands
            .push(pi_session::SlashCommand {
                name: "folder-a-command".into(),
                description: String::new(),
                source: "codex".into(),
            });
        state.threads[0].native.commands_loaded = true;

        // Folder B has no catalog yet. Nothing folder A reported stays visible.
        state.choose_draft_project(Some(id_b));
        let draft = &state.threads[0];
        assert!(draft.models.is_empty());
        assert!(
            draft.thinking_levels.is_empty(),
            "the reasoning menu must not offer folder A's levels while folder B scans"
        );
        assert!(draft.native.commands.is_empty() && !draft.native.commands_loaded);
        let descriptors = state.active_model_descriptors();
        assert!(
            crate::descriptors::split_column_and_footer_controls(&descriptors)
                .effort
                .is_none()
        );

        // Stale rows that reach a catalog miss are cleared with their levels.
        state.threads[0].models = vec![model_a.clone()];
        state.threads[0].thinking_levels = vec!["low".into(), "high".into(), "xhigh".into()];
        state.apply_native_catalogs();
        assert!(state.threads[0].models.is_empty());
        assert!(state.threads[0].thinking_levels.is_empty());

        // A thread already running in folder B is that folder's own evidence, so
        // its levels for the selected model show while the catalog is pending.
        let mut sibling = Thread::new(40);
        sibling.harness = "codex".into();
        sibling.project = Some(id_b);
        sibling.models = vec![leveled_model("codex", "model-b", &["medium"])];
        state.threads.push(sibling);
        state.threads[0].model_id = "model-b".into();
        state.apply_native_catalogs();
        assert!(state.threads[0].models.is_empty());
        assert_eq!(state.threads[0].thinking_levels, ["medium"]);

        // Back in folder A the selected model has no row, so folder B's levels go.
        state.choose_draft_project(Some(id_a));
        assert_eq!(state.threads[0].models, [model_a.clone()]);
        assert!(state.threads[0].thinking_levels.is_empty());

        // A refresh that drops the selected model or sends an explicit empty
        // snapshot clears stale reasoning levels on the draft.
        state.threads[0].model_id = "model-a".into();
        state.apply_native_catalogs();
        assert_eq!(state.threads[0].thinking_levels, ["low", "high", "xhigh"]);
        let other = leveled_model("codex", "other-model", &["low"]);
        server.models = vec![other.clone()];
        server.set_workspace_models(&dir_a, std::slice::from_ref(&other));
        let dropped = json!({
            "harnessIds": ["pi", "codex", "claude"],
            "nativeCatalogs": {"codex": server.snapshot()},
        });
        state.update_projected_native_models(&dropped);
        state.client_views = dropped;
        state.apply_native_catalogs();
        assert_eq!(state.threads[0].models, [other]);
        assert!(
            state.threads[0].thinking_levels.is_empty(),
            "dropping the selected model on refresh must clear stale reasoning levels"
        );

        state.threads[0].model_id = "other-model".into();
        state.apply_native_catalogs();
        assert_eq!(state.threads[0].thinking_levels, ["low"]);
        server.models.clear();
        server.set_workspace_models(&dir_a, &[]);
        let empty = json!({
            "harnessIds": ["pi", "codex", "claude"],
            "nativeCatalogs": {"codex": server.snapshot()},
        });
        state.update_projected_native_models(&empty);
        state.client_views = empty;
        state.apply_native_catalogs();
        assert!(state.threads[0].models.is_empty());
        assert!(
            state.threads[0].thinking_levels.is_empty(),
            "an explicit empty snapshot must clear stale reasoning levels"
        );
    }

    #[test]
    fn delegation_does_not_fall_back_to_models_from_other_projects() {
        let mut state = State::new(PathBuf::new(), false);
        let project_a = crate::projects::Project::new(1, PathBuf::from("delegate-a"));
        let project_b = crate::projects::Project::new(2, PathBuf::from("delegate-b"));
        let id_a = project_a.id;
        let id_b = project_b.id;
        let dir_b = project_b.path.clone();
        state.projects.entries.push(project_a);
        state.projects.entries.push(project_b);

        let codex_b = leveled_model("codex", "proj-b-codex", &["high"]);
        let claude_b = leveled_model("claude", "proj-b-claude", &["high"]);
        state
            .codex_catalog
            .set_workspace_models(&dir_b, std::slice::from_ref(&codex_b));
        state
            .claude_catalog
            .set_workspace_models(&dir_b, std::slice::from_ref(&claude_b));

        // Thread 0 in project A has no snapshot; thread 1 in project B has cached rows.
        state.threads[0].harness = "codex".into();
        state.threads[0].project = Some(id_a);
        let mut other_codex = Thread::new(10);
        other_codex.harness = "codex".into();
        other_codex.project = Some(id_b);
        other_codex.models = vec![codex_b];
        state.threads.push(other_codex);
        let mut other_claude = Thread::new(11);
        other_claude.harness = "claude".into();
        other_claude.project = Some(id_b);
        other_claude.models = vec![claude_b];
        state.threads.push(other_claude);

        assert!(
            state.delegation_models(&state.threads[0]).0.is_empty(),
            "same-harness delegation must not fall back to another project's models"
        );
        assert!(
            state.harness_models_for_thread(0, "claude").is_empty(),
            "cross-harness delegation must not borrow another project's models"
        );
        let capabilities = state.orchestrator_capabilities(0);
        assert_eq!(
            capabilities["ready"], false,
            "orchestrator_capabilities must not report ready using another project's models"
        );
        assert!(
            capabilities["models"]
                .as_array()
                .is_some_and(|models| models.is_empty()),
            "orchestrator_capabilities must not advertise another project's models: {capabilities}"
        );
        for harness in ["codex", "claude"] {
            let entry = capabilities["harnesses"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["id"] == harness)
                .unwrap();
            assert!(
                entry["models"]
                    .as_array()
                    .is_some_and(|models| models.is_empty()),
                "{harness} harness capability must not advertise another project's models: {entry}"
            );
        }
    }
}
