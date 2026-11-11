//! Harness settings and delegation metadata, kept outside the shared model reducer.
use super::*;

impl Thread {
    /// Native app-server usage is push-based, including refreshes after compaction.
    pub(super) fn apply_codex(&mut self, record: &Value) -> bool {
        if self.harness != "codex" || record["type"] != "context_usage" {
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
        } else {
            text.into()
        }
    }
}

impl State {
    pub(super) fn validate_harness(&self, id: &str) -> Result<(), String> {
        if matches!(id, "pi" | "codex") {
            Ok(())
        } else {
            self.harnesses.find(id).map(drop)
        }
    }

    pub(super) fn harness_models(&self, harness: &str) -> Vec<AgentModel> {
        let mut models = vec![];
        let provider = if harness == "pi" { "cpa" } else { harness };
        if harness == "pi" {
            models.extend(self.catalog.models.iter().cloned());
            if let Some(default) = &self.catalog.default_model
                && !models.iter().any(|m| m.id == default.id)
            {
                models.push(default.clone());
            }
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
        harnesses.extend(self.harnesses.agents.iter().map(
            |agent| json!({"id":agent.id,"name":agent.name,"provider":agent.id,"protocol":"acp"}),
        ));
        capabilities["harnesses"] = json!(harnesses);
        let values = capabilities["models"].as_array_mut().expect("models array");
        for model in codex_models {
            if !values
                .iter()
                .any(|m| m["provider"] == model.provider && m["id"] == model.id)
            {
                values.push(json!({"harness":"codex","provider":"codex","id":model.id,
                    "name":model.name,"reasoning_levels":model.thinking_levels}));
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
        capabilities["runtime"]["providers"] = json!(["cpa", "codex"]);
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

    /// Harness ids a thread can use: Pi, Codex, then registered ACP agents.
    pub(crate) fn harness_ids(&self) -> Vec<String> {
        let mut ids = vec!["pi".to_string(), "codex".to_string()];
        ids.extend(self.harnesses.agents.iter().map(|agent| agent.id.clone()));
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
