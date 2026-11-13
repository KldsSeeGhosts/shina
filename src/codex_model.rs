//! Harness settings and delegation metadata, kept outside the shared model reducer.
use super::*;

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
        if self.status.busy() || self.configuring {
            return;
        }
        self.model_options.retain(|o| o.id != id);
        self.model_options
            .push(crate::model_selection::ProviderOptionSelection {
                id: id.clone(),
                value: value.clone(),
            });
        if matches!(
            id.as_str(),
            "thinking" | "effort" | "reasoningEffort" | "reasoning" | "thought_level"
        ) && let Some(level) = value.as_str()
            && matches!(self.harness.as_str(), "pi" | "codex" | "claude")
        {
            self.selected_thinking = Some(level.to_owned());
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
            _ => return (false, None),
        };
        (catalog.loading(), catalog.error.as_deref())
    }
    pub(super) fn refresh_native_catalogs(&mut self, force: bool) {
        if self.demo || !self.runtime_enabled || self.client.is_some() {
            return;
        }
        let cwd = self.working_directory();
        self.codex_catalog
            .refresh("codex", cwd.clone(), self.wake.clone(), force);
        self.claude_catalog
            .refresh("claude", cwd, self.wake.clone(), force);
        self.apply_native_catalogs();
    }

    pub(super) fn apply_native_catalogs(&mut self) {
        if self.client.is_some() {
            for harness in ["codex", "claude"] {
                let catalog = &self.client_views["nativeCatalogs"][harness];
                let models = self.harness_models(harness);
                let projects = &self.projects.entries;
                for thread in self
                    .threads
                    .iter_mut()
                    .filter(|thread| thread.client_draft && thread.harness == harness)
                {
                    if !models.is_empty() {
                        thread.models = models.clone();
                    }
                    thread.refresh_thinking_levels();
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
                                    description: command["description"]
                                        .as_str()
                                        .unwrap_or("")
                                        .into(),
                                    source: command["source"].as_str().unwrap_or(harness).into(),
                                })
                            })
                            .collect();
                        thread.native.commands_loaded = true;
                    }
                }
            }
            return;
        }
        let directories: Vec<_> = (0..self.threads.len())
            .map(|index| self.thread_directory(index).ok())
            .collect();
        for (index, thread) in self.threads.iter_mut().enumerate() {
            let catalog = match thread.harness.as_str() {
                "codex" => &self.codex_catalog,
                "claude" => &self.claude_catalog,
                _ => continue,
            };
            if thread.pi.is_some() {
                continue;
            }
            if thread.harness_state["catalogRevision"].as_u64() == Some(catalog.revision) {
                continue;
            }
            thread.harness_state["catalogRevision"] = json!(catalog.revision);
            if let Some(error) = &catalog.error
                && crate::usage_limits::authentication_error(error)
            {
                thread.error = Some(error.clone());
                thread.status = Status::Failed;
            }
            if !catalog.commands_ready() {
                thread.native.commands_loaded = false;
                thread.native.commands.clear();
                thread.native.skills.clear();
            }
            if !catalog.models.is_empty() {
                thread.models = catalog.models.clone();
                thread.refresh_thinking_levels();
            }
            if catalog.commands_ready() && directories[index] == catalog.cwd {
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

    pub(super) fn validate_harness(&self, id: &str) -> Result<(), String> {
        if matches!(id, "pi" | "codex" | "claude") {
            Ok(())
        } else {
            self.harnesses.find(id).map(drop)
        }
    }

    pub(super) fn harness_models(&self, harness: &str) -> Vec<AgentModel> {
        let mut models = vec![];
        let provider = if harness == "pi" { "cpa" } else { harness };
        if self.client.is_some() && matches!(harness, "codex" | "claude") {
            models.extend(
                serde_json::from_value::<Vec<AgentModel>>(
                    self.client_views["nativeCatalogs"][harness]["models"].clone(),
                )
                .unwrap_or_default(),
            );
        }
        if harness == "pi" {
            models.extend(self.catalog.models.iter().cloned());
            if let Some(default) = &self.catalog.default_model
                && !models.iter().any(|m| m.id == default.id)
            {
                models.push(default.clone());
            }
        }
        if harness == "codex" {
            models.extend(self.codex_catalog.models.iter().cloned());
        } else if harness == "claude" {
            models.extend(self.claude_catalog.models.iter().cloned());
        }
        for thread in self.threads.iter().filter(|t| t.harness == harness) {
            for model in &thread.models {
                if model.provider == provider
                    && !models.iter().any(|m: &AgentModel| m.id == model.id)
                {
                    models.push(model.clone());
                }
            }
        }
        models
    }

    pub(super) fn add_harness_capabilities(&self, capabilities: &mut Value) {
        let mut harnesses = vec![json!({"id":"pi","name":"Pi","provider":"cpa"})];
        let codex_models = self.harness_models("codex");
        harnesses.push(json!({"id":"codex","name":"Codex","provider":"codex",
            "ready":!codex_models.is_empty(),"protocol":"codex.app-server"}));
        let claude_models = self.harness_models("claude");
        harnesses.push(json!({"id":"claude","name":"Claude","provider":"claude",
            "ready":!claude_models.is_empty(),"available":crate::claude::process().is_ok(),
            "protocol":"claude.stream-json"}));
        harnesses.extend(self.harnesses.agents.iter().map(
            |agent| json!({"id":agent.id,"name":agent.name,"provider":agent.id,"protocol":"acp"}),
        ));
        capabilities["harnesses"] = json!(harnesses);
        let values = capabilities["models"].as_array_mut().expect("models array");
        for model in codex_models.into_iter().chain(claude_models) {
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
        capabilities["runtime"]["providers"] = json!(["cpa", "codex", "claude"]);
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
        ids.extend(self.harnesses.agents.iter().map(|agent| agent.id.clone()));
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

    pub fn locked_provider_for_current(&self) -> Option<String> {
        let t = self.current();
        if t.has_started_session() {
            Some(t.harness.clone())
        } else {
            None
        }
    }

    pub fn model_disabled_reason_for(&self, instance_id: &str, _model: &str) -> Option<String> {
        let t = self.current();
        crate::model_picker::get_model_disabled_reason(
            t.has_started_session(),
            false,
            &t.harness,
            instance_id,
        )
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
            let (installed, error, auth_status, update_required_models, has_models) =
                if self.client.is_some() {
                    let snap = &self.client_views["nativeCatalogs"][id];
                    let models_empty = snap["models"].as_array().is_none_or(Vec::is_empty);
                    let err = snap["error"].as_str().map(str::to_owned);
                    let installed = snap["installed"]
                        .as_bool()
                        .unwrap_or_else(|| id != "claude" || crate::claude::process().is_ok());
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
                    let updates: Vec<crate::model_picker::UpdateRequiredModel> =
                        serde_json::from_value(snap["updateRequiredModels"].clone())
                            .unwrap_or_default();
                    (installed, err, auth, updates, !models_empty)
                } else {
                    let catalog = if id == "codex" {
                        &self.codex_catalog
                    } else {
                        &self.claude_catalog
                    };
                    let installed = if id == "claude" {
                        catalog.installed || crate::claude::process().is_ok()
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
                message: error,
                compatibility_broken_message: None,
                update_required_models,
            });
        }

        // ACP agents
        for agent in &self.harnesses.agents {
            if seen.iter().any(|(earlier_id, earlier_name)| {
                earlier_id == &agent.id && earlier_name == &agent.name
            }) {
                continue;
            }
            seen.push((agent.id.clone(), agent.name.clone()));
            entries.push(ProviderInstanceEntry {
                instance_id: agent.id.clone(),
                driver_kind: agent.id.clone(),
                display_name: agent.name.clone(),
                continuation_group_key: None,
                enabled: true,
                installed: true,
                status: ProviderStatusKind::Ready,
                auth_status: ProviderAuthStatus::Unknown,
                can_setup: false,
                is_available: true,
                is_acp: true,
                message: None,
                compatibility_broken_message: None,
                update_required_models: vec![],
            });
        }

        entries
    }

    pub fn picker_models_by_instance(
        &self,
    ) -> std::collections::BTreeMap<String, Vec<crate::model_picker::PickerModelOption>> {
        use crate::model_picker::PickerModelOption;
        let t = self.current();
        let mut map = std::collections::BTreeMap::new();
        for entry in self.provider_instance_entries() {
            let mut models = if entry.instance_id == t.harness && !t.models.is_empty() {
                t.models.clone()
            } else {
                self.harness_models(&entry.instance_id)
            };
            if entry.is_acp && models.is_empty() {
                models.push(AgentModel {
                    provider: entry.instance_id.clone(),
                    id: "default".into(),
                    name: "Default".into(),
                    is_default: true,
                    ..Default::default()
                });
            }
            // Retain active thread's model if unavailable / uncatalogued
            if entry.instance_id == t.harness
                && !t.model_id.is_empty()
                && !models.iter().any(|m| m.id == t.model_id)
            {
                models.push(AgentModel {
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
            }
            let options = models
                .into_iter()
                .map(|m| PickerModelOption {
                    slug: m.id,
                    name: m.name,
                    short_name: m.short_name,
                    sub_provider: m.sub_provider,
                    aliases: m.aliases,
                    badge: m.badge,
                    is_default: m.is_default,
                    is_custom: m.is_custom,
                    is_legacy: m.is_legacy,
                    is_unavailable: m.is_unavailable,
                })
                .collect();
            map.insert(entry.instance_id, options);
        }
        map
    }

    pub fn open_model_picker(&mut self, picker: SettingsPicker) {
        self.settings_picker = Some(picker);
        self.open_option_menu = None;
        self.thinking_drag = None;
        if picker == SettingsPicker::Model {
            self.model_search.replace_text("");
            let entries = self.provider_instance_entries();
            let options_by_instance = self.picker_models_by_instance();
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
        let t = self.current();
        let entries = self.provider_instance_entries();
        let models_by_instance = self.picker_models_by_instance();
        let locked_provider = self.locked_provider_for_current();
        let thread_sel = t.thread_model_selection();
        let project_sel = self.project_default_model_selection(&t.harness);
        let draft_active = t.draft_model_state.active_provider.as_deref();
        let runtime_id = t.has_started_session().then_some(t.harness.as_str());
        let thread_id = thread_sel.as_ref().map(|s| s.instance_id.as_str());
        let project_id = project_sel.as_ref().map(|s| s.instance_id.as_str());
        let resolved_prov = crate::model_selection::resolve_composer_provider_selection(
            &entries,
            &[draft_active, runtime_id, thread_id, project_id],
            locked_provider.as_deref(),
            runtime_id.or(thread_id),
        );
        let selected_instance_id = resolved_prov
            .selected_instance_id
            .clone()
            .unwrap_or_else(|| t.harness.clone());
        let eff = crate::model_selection::derive_effective_composer_model_state(
            Some(&t.draft_model_state),
            &entries,
            &models_by_instance,
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
        let models_by_instance = self.picker_models_by_instance();
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
        self.current_mut().set_model(provider, next.model.clone());
        for opt in &next.options {
            self.current_mut()
                .set_option(opt.id.clone(), opt.value.clone());
        }
        self.current_mut().draft_model_state = draft_state;
        self.current_mut().fallback_active = None;
        self.current_mut().fallback_explicit_override = true;
        self.composer_notices.sync("fallback-model", None);
        self.preferences.sticky_composer = sticky;
        self.reconcile_current_access_mode();
        self.checkpoint();
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
        let t = self.current();
        let (resolved_instance, resolved_model_id, resolved_opts) =
            self.resolved_composer_model_selection();
        let provider = if resolved_instance == "pi" {
            "cpa"
        } else {
            resolved_instance.as_str()
        };
        let mut models = if resolved_instance == t.harness && !t.models.is_empty() {
            t.models.clone()
        } else {
            self.harness_models(&resolved_instance)
        };
        let mut caps = models
            .iter_mut()
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
            crate::descriptors::with_implicit_fast_mode_default(&caps, Some(&resolved_opts));
        for opt in &t.model_options {
            if !selections.iter().any(|s| s.id == opt.id) {
                selections.push(opt.clone());
            }
        }
        crate::descriptors::get_provider_option_descriptors(&caps, &selections)
    }

    pub(super) fn apply_sticky_composer_to_draft(&mut self, index: usize) {
        let sticky = self.preferences.sticky_composer.clone();
        if sticky.sticky_active_provider.is_none()
            && sticky.sticky_model_selection_by_provider.is_empty()
        {
            return;
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
            let models = self.harness_models(active);
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
    fn draft_pickers_and_capabilities_share_prompt_free_native_models() {
        let mut state = State::new(PathBuf::new(), false);
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
            state.add_harness_capabilities(&mut capabilities);
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
        assert_eq!(state.default_harness, "pi");
        state.cycle_default_harness();
        assert_eq!(state.default_harness, "codex");
        state
            .current_mut()
            .draft
            .replace_text("Preserve an existing draft");
        state.new_thread_in(None);
        assert_eq!(state.current().harness, "codex");
        state.captains = Captain::presets();
        state.open_captain(0);
        assert_eq!(state.current().harness, "codex");
        assert_eq!(state.current().native.access, AccessMode::ReadOnly);
        let snapshot = state.snapshot();
        state.restore_global(&snapshot).unwrap();
        assert_eq!(state.default_harness, "codex");
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
                link: None,
                owner: Arc::default(),
                context_sent: true,
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
}
