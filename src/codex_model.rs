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
}

impl State {
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
