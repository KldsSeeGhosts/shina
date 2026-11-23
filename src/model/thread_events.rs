use super::*;

impl Thread {
    pub fn apply(&mut self, record: Value) {
        let queued_prompt = if record["type"] == "response" && record["command"] == "prompt" {
            self.peer_prompt
                .as_ref()
                .filter(|(prompt, _)| record["id"] == *prompt)
                .and_then(|(_, input)| self.peer_provider_inputs.get(&input.to_string()))
                .cloned()
        } else {
            None
        };
        if let Some(binding) = &queued_prompt {
            if record["success"] == false
                && !string(&record, "error")
                    .to_ascii_lowercase()
                    .contains("compaction")
            {
                self.execution_settlements
                    .push((binding.clone(), "rejected".into()));
                self.peer_provider_inputs
                    .retain(|_, queued| queued != binding);
            } else if record["success"] != false {
                self.execution_acceptances.push(binding.clone());
            }
        }
        if record["type"] == "message_start" && record["message"]["role"] == "user" {
            let text = content_text(&record["message"]["content"]);
            if let Some(input) = tasks::delivery_id(&text)
                && let Some(binding) = self.peer_provider_inputs.get(input).cloned()
                && (!self.status.busy() || self.provider_terminal_run == self.run_id)
            {
                self.peer_provider_inputs.remove(input);
                if self.next_execution.as_ref() == Some(&binding) {
                    self.next_execution = None;
                }
                self.run_id = Some(binding.run_id.clone());
                self.execution = Some(binding.clone());
                if let Some(origin) = &mut self.peer_origin {
                    origin.attempt_id = binding.attempt_id;
                    origin.result = None;
                    origin.acknowledged = false;
                    origin.attempt_message = text;
                    origin.reply_after_position = self.messages.len();
                }
                self.arm_task();
                self.note_provider_running();
            }
        }
        self.observe_task_history(&record);
        self.last_activity = Instant::now();
        if self.apply_native(&record) {
            self.drive_session();
            return;
        }
        match string(&record, "type") {
            "steering_restart" => {
                let held = self.native.queue_held;
                self.mark_stopped();
                self.native.queue_held = held;
                self.resumable_run_id = None;
            }
            "steer_delivery_unknown" => {
                let mut images = content_images(&record["message"]["content"]);
                images.extend(
                    record["attachments"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Attachment::from_value),
                );
                self.messages.push(Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    role: "You",
                    text: string(&record, "text").into(),
                    images,
                    context: None,
                });
                self.error = Some("ACP steering delivery could not be confirmed. The provider may have accepted this message, so Shina did not resend it.".into());
            }
            "harness_update" => {
                if record["field"] == "awareness" {
                    self.harness_context_sent = record["value"] == true;
                } else if let Some(field) = record["field"].as_str() {
                    self.harness_state[field] = record["value"].clone();
                    if matches!(field, "plan" | "plan_update") {
                        self.record_plan(&record["value"]);
                    }
                }
            }
            "response" => {
                let command = string(&record, "command");
                if self
                    .operation_deadline
                    .is_some_and(|(kind, _)| kind == command && kind != "abort")
                {
                    self.operation_deadline = None;
                }
                let injection = command == "prompt"
                    && self
                        .injection_id
                        .as_ref()
                        .is_some_and(|id| record["id"] == *id);
                if injection {
                    self.injection_id = None;
                    if record["success"] != false {
                        self.pending_injections.pop_front();
                        self.injection_retry_at = None;
                    } else if string(&record, "error")
                        .to_ascii_lowercase()
                        .contains("compaction")
                    {
                        self.injection_retry_at = Some(Instant::now() + Duration::from_secs(1));
                        self.diagnostic(
                            "Completion delivery deferred until Pi finishes compaction.",
                        );
                        return;
                    }
                }
                // Context stats are optional telemetry. A rejected or stale request must never
                // surface as a conversation error, and it must not disturb configuring.
                if command == "get_session_stats" {
                    if self.context.accept_response(&record) {
                        self.refresh_estimated_breakdown();
                    }
                    self.drive_session();
                    return;
                }
                if record["success"] == false {
                    if queued_prompt.is_some() && injection {
                        // Refusal of a distinct queued input does not fail or
                        // settle the provider turn that is already running.
                        self.pending_injections.pop_front();
                        self.injection_retry_at = None;
                        self.diagnostic(string(&record, "error"));
                        return;
                    }
                    if command == "prompt" && queued_prompt.is_none() {
                        self.memory_staged.clear();
                        self.memory_context.clear();
                        self.scope_context_pending = true;
                        self.note_provider_terminal("rejected");
                    }
                    self.error = Some(string(&record, "error").to_owned());
                    if self.initializing {
                        if command == "set_option"
                            && let Some(rejected) = self.pending_cursor_option.take()
                        {
                            self.model_options.retain(|option| option.id != rejected);
                        }
                        self.fail_session(self.error.clone().unwrap());
                        return;
                    }
                    if matches!(command, "set_model" | "set_thinking_level") {
                        if command == "set_thinking_level"
                            && self.pending_thinking.is_some()
                            && self.selected_thinking == self.pending_thinking
                        {
                            // The rejected level must not stay selected: drop the pin and
                            // every recorded choice of it back to what the provider runs.
                            self.selected_thinking = None;
                            self.revert_rejected_thinking();
                        }
                        self.configuring = false;
                        self.pending_thinking = None;
                        if command == "set_thinking_level" {
                            self.reconcile_thinking();
                        }
                    }
                    if command == "prompt"
                        && self
                            .pending_prompt
                            .as_ref()
                            .is_some_and(|(id, _, _)| record["id"] == *id)
                        && let Some((_, prompt, index)) = self.pending_prompt.take()
                    {
                        if self.revert_handoff() {
                            self.submission_error = self.error.clone();
                        }
                        self.restore_prompt(&prompt);
                        self.remove_optimistic_prompt(&prompt, index);
                    }
                    if matches!(command, "prompt" | "get_state" | "get_messages") {
                        self.native.queue_held = true;
                        self.status = Status::Failed;
                        self.started = None;
                        self.complete_task("failed");
                    }
                    return;
                }
                match command {
                    "get_state" => {
                        let data = &record["data"];
                        if data["turnCapabilities"].is_object() {
                            self.harness_state["turnCapabilities"] =
                                data["turnCapabilities"].clone();
                        }
                        if data["liveOptionDescriptors"].is_array() {
                            self.live_option_descriptors =
                                serde_json::from_value(data["liveOptionDescriptors"].clone()).ok();
                        }
                        if self
                            .startup_selection_check
                            .as_ref()
                            .is_some_and(|id| record["id"] == *id)
                        {
                            self.startup_selection_check = None;
                            let model_matches =
                                self.selected_model
                                    .as_ref()
                                    .is_none_or(|(provider, model)| {
                                        data["model"]["provider"] == *provider
                                            && data["model"]["id"] == *model
                                    });
                            let reasoning_matches = self
                                .selected_thinking
                                .as_ref()
                                .is_none_or(|level| data["thinkingLevel"] == *level);
                            if !model_matches || !reasoning_matches {
                                self.fail_session(format!(
                                    "Pi applied model {}/{} with reasoning {} instead of the delegated selection {}/{} with reasoning {}. Shina did not send the task.",
                                    string(&data["model"], "provider"),
                                    string(&data["model"], "id"),
                                    string(data, "thinkingLevel"),
                                    self.selected_model.as_ref().map_or("", |v| v.0.as_str()),
                                    self.selected_model.as_ref().map_or("", |v| v.1.as_str()),
                                    self.selected_thinking.as_deref().unwrap_or("default"),
                                ));
                                return;
                            }
                            self.startup_selection_verified = true;
                        }
                        if !self.initializing || self.selected_model.is_none() {
                            self.update_model(&data["model"]);
                        }
                        if !self.initializing || self.selected_thinking.is_none() {
                            self.thinking = string(data, "thinkingLevel").to_owned();
                        }
                        if self.harness == "pi" {
                            self.note_pi_thinking_default();
                        }
                        self.session = data["sessionFile"].as_str().map(str::to_owned);
                        if record["id"] == "state" {
                            self.state_loaded = true;
                            if data["isStreaming"] == true {
                                self.status = Status::Running;
                                self.started.get_or_insert_with(Instant::now);
                            }
                        }
                    }
                    "get_available_models" => {
                        if record["data"]["liveOptionDescriptors"].is_array() {
                            self.live_option_descriptors = serde_json::from_value(
                                record["data"]["liveOptionDescriptors"].clone(),
                            )
                            .ok();
                        } else if record["data"]["advertised"] == false {
                            self.live_option_descriptors = Some(Vec::new());
                        }
                        self.models = record["data"]["models"]
                            .as_array()
                            .map(|models| {
                                if self.harness == "pi" {
                                    AgentModel::pi_models(models)
                                } else {
                                    models
                                        .iter()
                                        .filter_map(|model| {
                                            AgentModel::from_harness_value(model, &self.harness)
                                        })
                                        .collect()
                                }
                            })
                            .unwrap_or_default();
                        if self.harness == "pi" {
                            self.note_pi_thinking_default();
                        }
                        self.refresh_thinking_levels();
                    }
                    "get_available_thinking_levels" => {
                        self.thinking_levels = record["data"]["levels"]
                            .as_array()
                            .map(|levels| {
                                levels
                                    .iter()
                                    .filter_map(|level| level.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default();
                    }
                    "set_model" => {
                        self.update_model(&record["data"]);
                        self.refresh_thinking_levels();
                        // Refresh the effective level too. Pi may clamp it for the new model.
                        self.command("get_state", json!({}));
                        self.command("get_available_thinking_levels", json!({}));
                        self.configuring = false;
                        // Capacity and the previous percentage belonged to the old model.
                        self.refresh_context_usage();
                        if let Some(confirmed) = self.thread_model_selection() {
                            crate::model_selection::release_model_selection(
                                &mut self.draft_model_state,
                                &confirmed,
                            );
                        }
                    }
                    "set_thinking_level" => {
                        if let Some(level) = self.pending_thinking.take() {
                            self.thinking = level;
                        }
                        self.command("get_state", json!({}));
                        self.configuring = false;
                        if let Some(confirmed) = self.thread_model_selection() {
                            crate::model_selection::release_model_selection(
                                &mut self.draft_model_state,
                                &confirmed,
                            );
                        }
                        self.reconcile_thinking();
                    }
                    "set_option" if self.initializing && self.harness == "cursor" => {
                        self.pending_cursor_option = None;
                        self.configuring = false;
                    }
                    "get_messages" if record["id"] == "history" => {
                        // `history_loaded` also covers the optimistic server
                        // transcript restored from disk. Only this response
                        // can provide a provider-side sample for replay.
                        self.peer_input_history = record["data"]["messages"]
                            .as_array()
                            .filter(|_| {
                                record["success"] == true
                                    && record["data"]["historyReplayed"] != false
                            })
                            .map(|messages| {
                                messages
                                    .iter()
                                    .filter(|message| message["role"] == "user")
                                    .map(|message| content_text(&message["content"]))
                                    .filter(|text| tasks::delivery_id(text).is_some())
                                    .map(|text| remote_tasks::input_history_key(&text))
                                    .collect()
                            });
                        if self.server_owned
                            && self.harness != "pi"
                            && record["data"]["replaceHistory"] != true
                            && record["data"]["messages"]
                                .as_array()
                                .is_some_and(|messages| {
                                    messages.is_empty()
                                        || record["data"]["historyReplayed"] == false
                                })
                            && !self.messages.is_empty()
                        {
                            // ACP resume need not replay history. Keep the server transcript
                            // and the existing optimistic prompt index in that case.
                            self.recover_native_inputs();
                            self.history_loaded = true;
                            self.drive_session();
                            return;
                        }
                        if let Some(messages) = record["data"]["messages"].as_array() {
                            if self.captain_return_context
                                && messages.is_empty()
                                && self.queued_prompt.is_none()
                                && self.pending_prompt.is_none()
                            {
                                // Warming a fresh provider must not replace the controller transcript.
                                self.history_loaded = true;
                                self.drive_session();
                                return;
                            }
                            let fresh_boundary = self
                                .messages
                                .iter()
                                .rposition(|m| self.is_fresh_session_message(&m.id));
                            if let Some(b) = fresh_boundary
                                && messages.is_empty()
                                && record["data"]["replaceHistory"] != true
                                && self.queued_prompt.is_none()
                                && self.pending_prompt.is_none()
                                && self.messages.len() > b
                            {
                                self.recover_native_inputs();
                                self.history_loaded = true;
                                self.drive_session();
                                return;
                            }
                            let saved_queued_id = self
                                .queued_prompt
                                .as_ref()
                                .and_then(|(_, idx)| self.messages.get(*idx).map(|m| m.id.clone()));
                            let _ = &saved_queued_id;
                            let saved_work = self.work.clone();
                            let saved_orders: std::collections::HashMap<_, _> = self
                                .tools
                                .iter()
                                .filter(|t| t.order > 0)
                                .map(|t| (t.id.clone(), t.order))
                                .collect();
                            let saved_thoughts: Vec<_> = self
                                .thoughts
                                .iter()
                                .filter(|t| t.order > 0)
                                .map(|t| (t.anchor, t.block, t.order))
                                .collect();
                            let mut saved_messages = std::mem::take(&mut self.messages);
                            let mut prefix_messages = Vec::new();
                            let mut prefix_tools = Vec::new();
                            let mut prefix_thoughts = Vec::new();
                            let mut prefix_work = Vec::new();
                            let mut prefix_compactions = Vec::new();
                            let message_offset = fresh_boundary.unwrap_or(0);
                            if let Some(b) = fresh_boundary {
                                let keep = b.min(saved_messages.len());
                                prefix_messages = saved_messages.drain(..keep).collect();
                                for tool in std::mem::take(&mut self.tools) {
                                    if tool.anchor.message < keep {
                                        prefix_tools.push(tool);
                                    }
                                }
                                for thought in std::mem::take(&mut self.thoughts) {
                                    if thought.anchor.message < keep {
                                        prefix_thoughts.push(thought);
                                    }
                                }
                                for work in &saved_work {
                                    if work.start < keep {
                                        prefix_work.push(work.clone());
                                    }
                                }
                                for compaction in std::mem::take(&mut self.native.compactions) {
                                    if compaction["anchor"]["message"]
                                        .as_u64()
                                        .is_some_and(|m| (m as usize) < keep)
                                    {
                                        prefix_compactions.push(compaction);
                                    }
                                }
                            } else {
                                self.tools.clear();
                                self.thoughts.clear();
                                self.native.compactions.clear();
                            }
                            self.work.clear();
                            self.assistant = None;
                            if let Some(points) = record["data"]["forkMessages"].as_array() {
                                self.native.fork_points = points.clone();
                            }
                            if let Some(compactions) = record["data"]["compactions"].as_array() {
                                self.native.compactions = compactions
                                    .iter()
                                    .rev()
                                    .take(32)
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .into_iter()
                                    .rev()
                                    .collect();
                            }
                            let messages = approval_send::history_with_approval_feedback(
                                messages,
                                &saved_messages,
                            );
                            let mut native_history_index = 0;
                            for message in &messages {
                                if message["shinaFeedbackId"].is_null() {
                                    self.anchor_compactions(native_history_index);
                                    native_history_index += 1;
                                }
                                let role = string(message, "role");
                                let index = self.messages.len();
                                if matches!(role, "user" | "assistant") {
                                    let text = content_text(&message["content"]);
                                    let images = if role == "user" {
                                        content_images(&message["content"])
                                    } else {
                                        Vec::new()
                                    };
                                    if role == "assistant" || !text.is_empty() || !images.is_empty()
                                    {
                                        if role == "assistant" {
                                            self.ensure_work();
                                        }
                                        self.messages.push(Message {
                                            id: message["shinaFeedbackId"]
                                                .as_str()
                                                .map(str::to_owned)
                                                .unwrap_or_else(|| {
                                                    uuid::Uuid::new_v4().to_string()
                                                }),
                                            role: if role != "user" {
                                                "Pi"
                                            } else if text.starts_with(INJECTION_PREFIX) {
                                                "Shina"
                                            } else {
                                                "You"
                                            },
                                            text,
                                            images,
                                            context: None,
                                        });
                                    }
                                }
                                if role == "custom" && message["display"] != false {
                                    self.messages.push(Message {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        role: "Shina",
                                        text: content_text(&message["content"]),
                                        images: content_images(&message["content"]),
                                        context: None,
                                    });
                                }
                                if role == "assistant" {
                                    self.anchor_content_work(&message["content"], index, true);
                                    if message["stopReason"] == "aborted" {
                                        let order = self.next_work_order();
                                        self.ensure_work().stopped = true;
                                        self.ensure_work().stopped_order = order;
                                    } else if matches!(
                                        message["stopReason"].as_str(),
                                        Some("stop" | "length")
                                    ) {
                                        self.ensure_work().outcome = Some(Status::Completed);
                                    } else if message["stopReason"] == "error" {
                                        self.ensure_work().outcome = Some(Status::Failed);
                                    }
                                }
                                if role == "toolResult"
                                    && let Some(tool) = self
                                        .tools
                                        .iter_mut()
                                        .rev()
                                        .find(|t| t.id == string(message, "toolCallId"))
                                {
                                    tool.failed = message["isError"] == true;
                                    tool.accept_result(message);
                                    tool.duration_ms = message["durationMs"].as_u64();
                                }
                            }
                            // Provider history replaces arrays during recovery. Preserve
                            // stored identities before publishing the reconstructed rows.
                            let mut used = std::collections::HashSet::new();
                            let mut identities = self
                                .messages
                                .iter()
                                .map(|message| {
                                    let found =
                                        saved_messages.iter().enumerate().find(|(i, old)| {
                                            !used.contains(i)
                                                && if message.id.starts_with("approval-feedback:") {
                                                    old.id == message.id
                                                } else {
                                                    !old.id.starts_with("approval-feedback:")
                                                }
                                                && old.role == message.role
                                                && old.text == message.text
                                        });
                                    found.map(|(i, old)| {
                                        used.insert(i);
                                        old.id.clone()
                                    })
                                })
                                .collect::<Vec<_>>();
                            for (i, message) in self.messages.iter_mut().enumerate() {
                                if let Some(id) = identities[i].take() {
                                    message.id = id;
                                } else if let Some(old) = saved_messages.get(i).filter(|old| {
                                    old.role == message.role
                                        && !used.contains(&i)
                                        && !old.id.starts_with("approval-feedback:")
                                }) {
                                    used.insert(i);
                                    message.id = old.id.clone();
                                }
                            }
                            self.anchor_compactions(native_history_index);
                            for tool in &mut self.tools {
                                if let Some(order) = saved_orders.get(&tool.id) {
                                    tool.order = *order;
                                }
                            }
                            for thought in &mut self.thoughts {
                                if let Some((_, _, order)) =
                                    saved_thoughts.iter().find(|(anchor, block, _)| {
                                        *anchor == thought.anchor && *block == thought.block
                                    })
                                {
                                    thought.order = *order;
                                }
                            }
                            for work in &mut self.work {
                                if work.outcome.is_none() {
                                    work.outcome = saved_work
                                        .iter()
                                        .find(|saved| saved.start == work.start + message_offset)
                                        .and_then(|saved| saved.outcome);
                                }
                                if let Some(saved) = saved_work.iter().find(|saved| {
                                    saved.start == work.start + message_offset && saved.stopped
                                }) {
                                    work.stopped = true;
                                    work.stopped_order = saved.stopped_order;
                                    work.elapsed = saved.elapsed;
                                }
                            }
                            for saved in saved_work
                                .into_iter()
                                .filter(|w| w.stopped && w.start >= message_offset)
                            {
                                let local_start = saved.start - message_offset;
                                if !self.work.iter().any(|w| w.start == local_start) {
                                    let mut shifted = saved;
                                    shifted.start = local_start;
                                    self.work.push(shifted);
                                }
                            }
                            self.work.sort_by_key(|w| w.start);
                            if message_offset > 0 {
                                for tool in &mut self.tools {
                                    tool.anchor.message += message_offset;
                                }
                                for thought in &mut self.thoughts {
                                    thought.anchor.message += message_offset;
                                }
                                for work in &mut self.work {
                                    work.start += message_offset;
                                }
                                for compaction in &mut self.native.compactions {
                                    if let Some(m) = compaction["anchor"]["message"].as_u64() {
                                        compaction["anchor"]["message"] =
                                            json!(m + message_offset as u64);
                                    }
                                }
                                if let Some(assistant) = &mut self.assistant {
                                    *assistant += message_offset;
                                }
                                prefix_messages.append(&mut self.messages);
                                self.messages = prefix_messages;
                                prefix_tools.append(&mut self.tools);
                                self.tools = prefix_tools;
                                prefix_thoughts.append(&mut self.thoughts);
                                self.thoughts = prefix_thoughts;
                                prefix_work.append(&mut self.work);
                                self.work = prefix_work;
                                prefix_compactions.append(&mut self.native.compactions);
                                self.native.compactions = prefix_compactions;
                            }
                            self.agent_index = crate::agents::AgentIndex::rebuild(&self.tools);
                            // A process can accept a prompt before its response reaches us.
                            // Do not replay it if its unique delivery marker is already saved.
                            self.pending_injections.retain_mut(|injection| {
                                injection.shown = false;
                                !self
                                    .messages
                                    .iter()
                                    .any(|m| m.role == "Shina" && m.text == injection.text)
                            });
                            self.injection_id = None;
                        }
                        if let Some((prompt, index)) = &mut self.queued_prompt {
                            *index = self.messages.len();
                            let mut id = uuid::Uuid::new_v4().to_string();
                            if record["data"]["messages"]
                                .as_array()
                                .is_some_and(|messages| messages.is_empty())
                                && let Some(prev_id) =
                                    self.fresh_session_messages.last().filter(|prev_id| {
                                        self.messages.iter().all(|m| m.id != **prev_id)
                                    })
                            {
                                id = prev_id.clone();
                            }
                            let _ = &id;
                            self.messages.push(Message {
                                id,
                                role: "You",
                                text: prompt.text.clone(),
                                images: prompt.images.to_vec(),
                                context: prompt.context.clone(),
                            });
                        }
                        self.recover_native_inputs();
                        self.history_loaded = true;
                    }
                    "prompt" => {
                        if !injection
                            && self
                                .pending_prompt
                                .as_ref()
                                .is_some_and(|(id, _, _)| record["id"] == *id)
                        {
                            self.pending_prompt = None;
                            self.captain_return_context = false;
                            self.mark_handoff_delivered();
                            self.note_provider_prompt_accepted();
                        }
                        if injection && queued_prompt.is_none() {
                            self.note_provider_prompt_accepted();
                        }
                        if record["data"]["disposition"] == "handled"
                            && self.status == Status::Running
                        {
                            // A command can launch work independently. Check Pi before settling it.
                            self.probe_command_idle();
                        }
                        // Capture the newly created session path before a long first run finishes.
                        self.command("get_state", json!({}));
                    }
                    _ => {}
                }
            }
            "agent_start" => {
                self.note_provider_running();
                if !self.completion_notified {
                    self.arm_task();
                }
                self.mark_handoff_delivered();
                if self.run_id.is_none() {
                    self.run_id = Some(uuid::Uuid::new_v4().to_string());
                }
                self.arm_crew_report();
                self.prompt_suggestion = None;
                self.life.last_started_at = Some(lifecycle::now());
                self.status = Status::Running;
                self.live_work_collapsed = false;
                self.assistant = None;
                self.phase = "Waiting for the model".into();
                self.started.get_or_insert_with(Instant::now);
                self.ensure_work().outcome = Some(Status::Running);
            }
            // agent_end is deliberately NOT settlement. Pi can still retry or compact.
            "agent_settled" => {
                if self
                    .operation_deadline
                    .is_some_and(|(kind, _)| kind == "abort")
                {
                    self.operation_deadline = None;
                }
                self.life.last_completed_at = Some(lifecycle::now());
                self.expire_live_question("The provider turn ended.");
                if self.native.questionnaire.is_none() {
                    self.request = None;
                }
                let aborted = record["aborted"] == true;
                if aborted {
                    self.mark_stopped();
                } else {
                    self.resumable_run_id = None;
                }
                self.finish_run();
                self.live_work_collapsed = false;
                if self
                    .native
                    .questionnaire
                    .as_ref()
                    .is_some_and(|form| form.capability == "message")
                {
                    self.native.dialog_status = Some(self.status);
                }
                // A stopped run is idle, not a finished task.
                if aborted && self.status == Status::Completed {
                    self.status = Status::Ready;
                }
                if aborted || self.status != Status::Completed {
                    self.prompt_suggestion = None;
                }
                self.complete_task(if aborted {
                    "cancelled"
                } else {
                    status_word(self.status)
                });
                self.note_provider_terminal(if aborted {
                    "cancelled"
                } else {
                    status_word(self.status)
                });
                self.settle_memory(!aborted && self.status == Status::Completed);
                // The turn that just finished is what changed the live context window.
                self.context.cancel_request();
                self.refresh_context_usage();
            }
            "message_start" if record["message"]["role"] == "assistant" => {
                self.note_provider_running();
                self.assistant = None;
                if self.native.questionnaire.is_none() {
                    self.request = None;
                }
                self.status = Status::Running;
                self.assistant();
            }
            "message_update" => {
                let update = &record["assistantMessageEvent"];
                if update["type"] == "text_delta" {
                    self.assistant().text.push_str(string(update, "delta"));
                    self.phase = "Writing".into();
                } else if matches!(
                    string(update, "type"),
                    "thinking_start" | "thinking_delta" | "thinking_end"
                ) {
                    let thought =
                        self.thought(update["contentIndex"].as_u64().unwrap_or(0) as usize);
                    thought.started.get_or_insert_with(Instant::now);
                    match string(update, "type") {
                        "thinking_delta" => thought.text.push_str(string(update, "delta")),
                        "thinking_end" => {
                            thought.text = string(update, "content").into();
                            thought.done = true;
                            thought.duration_ms = thought
                                .started
                                .map(|start| start.elapsed().as_millis() as u64);
                        }
                        _ => {}
                    }
                    self.phase = "Thinking".into();
                }
            }
            "message_end" if record["message"]["role"] == "assistant" => {
                let message = &record["message"];
                let text = content_text(&message["content"]);
                self.assistant();
                let index = self.assistant.unwrap();
                if !text.is_empty() {
                    self.assistant().text = text;
                    self.error = None;
                }
                self.anchor_content_work(&message["content"], index, false);
                if message["stopReason"] != "aborted"
                    && let Some(error) = message["errorMessage"].as_str().filter(|s| !s.is_empty())
                {
                    self.error = Some(error.to_owned());
                }
                self.assistant = None;
            }
            "tool_execution_start" => {
                self.phase = format!("Running {}", string(&record, "toolName"));
                let id = string(&record, "toolCallId");
                if let Some(index) = self.tools.iter().rposition(|tool| tool.id == id) {
                    let tool = &mut self.tools[index];
                    tool.started.get_or_insert_with(Instant::now);
                    if let Some(parent) = record["parentToolCallId"].as_str() {
                        tool.parent_call = Some(parent.into());
                    }
                    self.agent_index.observe(index, tool);
                } else {
                    self.assistant();
                    let anchor = ToolAnchor {
                        message: self.assistant.unwrap_or(self.messages.len()),
                        offset: self.assistant.map_or(0, |i| self.messages[i].text.len()),
                    };
                    let mut tool =
                        Tool::new(id, string(&record, "toolName"), &record["args"], anchor);
                    tool.accept_metadata(&record);
                    tool.order = self.next_work_order();
                    tool.started = Some(Instant::now());
                    tool.parent_call = record["parentToolCallId"].as_str().map(str::to_owned);
                    tool.seed_subagents();
                    self.agent_index.observe(self.tools.len(), &tool);
                    self.tools.push(tool);
                }
            }
            "auto_retry_start" => {
                self.record_retry(&record, false);
                self.phase = format!("Retrying, attempt {}", record["attempt"]);
                self.diagnostic(string(&record, "errorMessage"));
            }
            "auto_retry_end" => {
                self.record_retry(&record, true);
                if record["success"] == true {
                    self.error = None;
                    self.phase = "Working".into();
                } else {
                    self.error = Some(
                        record["finalError"]
                            .as_str()
                            .filter(|error| !error.is_empty())
                            .unwrap_or("Provider error")
                            .into(),
                    );
                }
            }
            "compaction_start" => {
                self.phase = "Compacting context".into();
                // The old percentage describes the context that is being replaced.
                self.context.begin_compaction();
            }
            "compaction_end" => {
                if let Some(error) = record["errorMessage"].as_str() {
                    self.error = Some(error.into());
                    if !self.native.queue.is_empty() {
                        self.native.queue_held = true;
                    }
                }
                self.context.finish_compaction();
                // Pi reports null usage until its next assistant response, so this request
                // legitimately leaves the ring unknown rather than reusing the old figure.
                self.refresh_context_usage();
                self.flush_injection();
            }
            "thinking_level_changed" => self.thinking = string(&record, "level").into(),
            "tool_execution_update" | "tool_execution_end" => {
                let ending = record["type"] == "tool_execution_end";
                if let Some(index) = self
                    .tools
                    .iter()
                    .rposition(|tool| tool.id == string(&record, "toolCallId"))
                {
                    let tool = &mut self.tools[index];
                    tool.done = ending;
                    tool.failed = record["isError"] == true;
                    if ending {
                        tool.duration_ms = record["durationMs"].as_u64().or_else(|| {
                            tool.started.map(|start| start.elapsed().as_millis() as u64)
                        });
                    }
                    tool.accept_result(if ending {
                        &record["result"]
                    } else {
                        &record["partialResult"]
                    });
                    self.agent_index.observe(index, tool);
                }
            }
            "subagent_activity_update" => {
                if let Some(index) = self
                    .tools
                    .iter()
                    .rposition(|tool| tool.id == string(&record, "toolCallId"))
                {
                    let tool = &mut self.tools[index];
                    tool.accept_subagents(&record["results"]);
                    self.agent_index.observe(index, tool);
                }
            }
            "extension_ui_request" => {
                if matches!(
                    string(&record, "method"),
                    "confirm" | "select" | "input" | "editor"
                ) {
                    self.reply.replace_text(
                        self.native
                            .questionnaire
                            .as_ref()
                            .map(|form| form.answers[form.index].other.as_str())
                            .unwrap_or_else(|| string(&record, "prefill")),
                    );
                    self.request = Some(record);
                    self.refresh_typing_guard();
                    if self.native.questionnaire.is_some() && !self.request_held_by_typing_guard() {
                        self.draft.focused = false;
                        self.reply.focused = false;
                    }
                    self.status = Status::Waiting;
                } else if record["method"] == "set_editor_text" {
                    self.draft.replace_text(string(&record, "text"));
                }
            }
            "prompt_suggestion" => {
                let event_run_id = record["runId"].as_str();
                if !self.status.busy()
                    && self.status == Status::Completed
                    && self.run_id.is_some()
                    && event_run_id == self.run_id.as_deref()
                    && crate::harness::supports_prompt_suggestions(&self.harness)
                {
                    self.prompt_suggestion = record["text"]
                        .as_str()
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(|text| PromptSuggestion {
                            run_id: event_run_id.unwrap().to_owned(),
                            text: text.to_owned(),
                        });
                }
            }
            _ => {}
        }
        self.drive_session();
    }

    pub(super) fn next_work_order(&self) -> u64 {
        self.tools
            .iter()
            .map(|tool| tool.order)
            .chain(self.thoughts.iter().map(|thought| thought.order))
            .chain(self.work.iter().map(|work| work.stopped_order))
            .chain(
                self.native
                    .compactions
                    .iter()
                    .filter_map(|c| c["order"].as_u64()),
            )
            .chain(
                crate::work::retry_events(self)
                    .iter()
                    .filter_map(|e| e["order"].as_u64()),
            )
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub(super) fn mark_stopped(&mut self) {
        self.resumable_run_id = self.run_id.clone();
        self.native.queue_held = true;
        let elapsed = self
            .started
            .map(|at| at.elapsed().as_secs())
            .or(self.elapsed);
        self.finish_retry(Some("Provider retry stopped"));
        self.assistant();
        let order = self.next_work_order();
        let work = self.ensure_work();
        work.stopped = true;
        if work.stopped_order == 0 {
            work.stopped_order = order;
        }
        work.elapsed = elapsed.or(work.elapsed);
    }

    pub(super) fn record_retry(&mut self, record: &Value, ending: bool) {
        let turn = self.ensure_work().start;
        let previous = crate::work::retry_events(self)
            .iter()
            .position(|e| e["turn"].as_u64() == Some(turn as u64));
        if ending && previous.is_none() {
            return;
        }
        if previous.is_none() {
            self.assistant();
            let message = self.assistant.unwrap();
            let event = json!({"turn":turn,"anchor":ToolAnchor {message,offset:self.messages[message].text.len()},
                "order":self.next_work_order()});
            if !self.harness_state["workRetries"].is_array() {
                self.harness_state["workRetries"] = json!([]);
            }
            self.harness_state["workRetries"]
                .as_array_mut()
                .unwrap()
                .push(event);
        }
        let events = self.harness_state["workRetries"].as_array_mut().unwrap();
        let index = previous.unwrap_or(events.len() - 1);
        let event = &mut events[index];
        event["running"] = json!(!ending);
        if ending {
            event["success"] = record["success"].clone();
            event["error"] = record["finalError"].clone();
        } else {
            event["attempt"] = record["attempt"].clone();
            event["maxAttempts"] = record["maxAttempts"].clone();
            event["error"] = record["errorMessage"].clone();
        }
    }

    pub(super) fn finish_retry(&mut self, error: Option<&str>) {
        let turn = self.work.last().map(|w| w.start);
        if crate::work::retry_events(self)
            .iter()
            .any(|e| e["turn"].as_u64() == turn.map(|start| start as u64) && e["running"] == true)
        {
            self.record_retry(&json!({"success":error.is_none(),"finalError":error}), true);
        }
    }

    pub(super) fn record_plan(&mut self, source: &Value) {
        let turn = self.ensure_work().start;
        let id = source["_shinaPlanId"]
            .as_str()
            .or_else(|| source["planId"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("plan-{turn}"));
        let index = if let Some(index) = self.tools.iter().position(|t| t.id == id) {
            index
        } else {
            self.assistant();
            let message = self.assistant.unwrap();
            let mut tool = Tool::new(
                &id,
                "plan",
                source,
                ToolAnchor {
                    message,
                    offset: self.messages[message].text.len(),
                },
            );
            tool.kind = Some(crate::work::ToolKind::Plan);
            tool.order = self.next_work_order();
            self.agent_index.observe(self.tools.len(), &tool);
            self.tools.push(tool);
            self.tools.len() - 1
        };
        let tool = &mut self.tools[index];
        tool.done = true;
        tool.input = source.clone();
        tool.structured =
            json!({"input":source,"structuredContent":{"plan":crate::work::plan_data(source)}});
    }

    pub(super) fn update_model(&mut self, model: &Value) {
        if let Some(id) = model["id"].as_str() {
            let provider = string(model, "provider").to_owned();
            let known = self
                .models
                .iter()
                .find(|m| m.id == id)
                .map(|m| (m.name.clone(), m.context_window, m.supports_images));
            if self.model_id != id || self.model_provider != provider {
                // Pi changed models: the old context percentage is not a measurement of the
                // new model's window, so it stays unknown until the next stats response.
                self.context.begin_model_change();
                self.context_window = None;
                self.supports_images = None;
            }
            self.model_id = id.into();
            self.model_provider = provider;
            self.model = model["name"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| known.as_ref().map(|(name, _, _)| name.clone()))
                .unwrap_or_else(|| id.into());
            let window = model["contextWindow"]
                .as_u64()
                .filter(|window| *window > 0)
                .or_else(|| known.as_ref().and_then(|(_, window, _)| *window));
            if window.is_some() {
                self.context_window = window;
            }
            if let Some(input) = model["input"].as_array() {
                self.supports_images = Some(input.iter().any(|kind| kind == "image"));
            } else if let Some((_, _, Some(images))) = known {
                self.supports_images = Some(images);
            }
        }
    }

    pub(super) fn note_pi_thinking_default(&mut self) {
        if let Some(model) = self.models.iter_mut().find(|m| m.id == self.model_id) {
            model.apply_pi_thinking_default(&self.thinking);
        }
    }

    pub(super) fn restore_prompt(&mut self, prompt: &Prompt) {
        let split_reply = prompt.draft.as_ref().and_then(|draft| {
            crate::composer_tokens::split_leading_reply_reference(
                &prompt.text,
                Some(&draft.editor()),
                prompt.context.as_ref(),
            )
        });
        let submitted = prompt.draft.as_ref().map_or_else(
            || crate::composer_rich::draft(&prompt.text),
            crate::composer_rich::Snapshot::editor,
        );
        self.draft = crate::composer_rich::prepend(&submitted, &self.draft);
        // Keep submitted images first. A newer draft may have added images while the process was
        // starting, so merge those after the snapshot without duplicating either set.
        let mut images = prompt
            .draft
            .as_ref()
            .map_or_else(|| prompt.images.to_vec(), |draft| draft.attachments.clone());
        for image in &self.draft_images {
            if !images.iter().any(|current| current.id == image.id) {
                images.push(image.clone());
            }
        }
        // Preserve both drafts even if their combined images exceed a message limit.
        // The user can remove images before sending. Never discard a newer attachment.
        self.draft_images = images;
        let mut context = prompt
            .draft
            .as_ref()
            .filter(|draft| !draft.context.is_empty())
            .map(|draft| draft.context.clone())
            .or_else(|| prompt.context.as_ref().map(|c| c.records.clone()))
            .unwrap_or_default();
        for record in &self.draft_context {
            if !context.iter().any(|c| c.context_id == record.context_id) {
                context.push(record.clone());
            }
        }
        self.draft_context = context;
        if self.draft_reply_to.is_none()
            && let Some((reply_record, _)) = split_reply
        {
            self.draft_reply_to = Some(reply_record);
        }
        self.attachment_error = attachments::validate_prompt_images(&self.draft_images).err();
    }

    pub(crate) fn restore_snapshot(&mut self, snapshot: &crate::composer_rich::Snapshot) {
        if let Some(edit) = &mut self.queued_edit {
            edit.ordinary = crate::composer_rich::prepend(&snapshot.editor(), &edit.ordinary);
            let mut images = snapshot.attachments.clone();
            for image in &edit.ordinary_images {
                if !images.iter().any(|existing| existing.id == image.id) {
                    images.push(image.clone());
                }
            }
            edit.ordinary_images = images;
            let mut context = snapshot.context.clone();
            for record in &edit.ordinary_context {
                if !context.iter().any(|c| c.context_id == record.context_id) {
                    context.push(record.clone());
                }
            }
            edit.ordinary_context = context;
        } else {
            let mut prompt = Prompt::new(String::new(), snapshot.attachments.clone());
            prompt.draft = Some(snapshot.clone());
            self.restore_prompt(&prompt);
        }
    }

    pub(crate) fn finish_remote_send(&mut self, command: uuid::Uuid, accepted: bool) -> bool {
        if !self
            .remote_send
            .as_ref()
            .is_some_and(|(id, _)| *id == command)
        {
            return false;
        }
        let (_, snapshot) = self.remote_send.take().unwrap();
        let sent_arm = self.remote_send_fresh_arm.take();
        let sent_reply_to = self.remote_send_reply_to.take();
        if accepted {
            if let Some(arm) = sent_arm
                && self.fresh_session_arm == Some(arm)
            {
                self.fresh_session_arm = None;
            }
        } else {
            self.restore_snapshot(&snapshot);
            if self.draft_reply_to.is_none() {
                self.draft_reply_to = sent_reply_to;
            }
        }
        true
    }

    pub(super) fn finish_run(&mut self) {
        self.finish_retry(self.error.clone().as_deref());
        self.status = if self.error.is_some() {
            Status::Failed
        } else {
            Status::Completed
        };
        if self.status == Status::Failed {
            self.native.queue_held = true;
            self.life.failed_at = Some(lifecycle::now());
        }
        self.elapsed = self
            .started
            .take()
            .map(|start| start.elapsed().as_secs())
            .or(self.elapsed);
        self.ensure_work().elapsed = self.elapsed;
        let start = self.ensure_work().start;
        let outcome = if self.ensure_work().stopped {
            None
        } else {
            Some(self.status)
        };
        for work in &mut self.work {
            if work.start == start || work.outcome == Some(Status::Running) {
                work.outcome = outcome;
            }
        }
        self.finish_thoughts();
        self.assistant = None;
        self.phase.clear();
        if self.error.is_none() {
            self.retry_prompt = None;
        }
    }

    pub(super) fn finish_thoughts(&mut self) {
        for thought in self.thoughts.iter_mut().filter(|thought| !thought.done) {
            thought.done = true;
            thought.duration_ms = thought
                .started
                .map(|start| start.elapsed().as_millis() as u64);
        }
    }
}
