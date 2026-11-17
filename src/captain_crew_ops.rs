//! Independent crew roots use ordinary inputs, queues and run controls.
//! User commands use device receipts; MCP intents use the same SQLite receipt
//! table under a non-authenticating, stable Captain audit namespace.
use super::*;
use crate::protocol::Command;

pub(super) fn keys(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or("Crew arguments must be an object.")?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unsupported crew setting {key}."));
    }
    Ok(())
}

impl State {
    fn crew_database(&self) -> Result<crate::database::Database, String> {
        if self.client.is_some() {
            return Err("Send this crew command to the Captain's owning server.".into());
        }
        let database = crate::database::for_path(&self.data_dir.join("index.json"))
            .ok_or("Crew control requires the durable server.")?;
        database.require_batch()?;
        Ok(database)
    }

    fn crew_home_index(&self, captain: usize) -> Result<usize, String> {
        self.require_active_captain(captain)?;
        let home = self
            .captain_home(captain)
            .ok_or("Open the Captain's home before creating crew.")?;
        self.threads
            .iter()
            .position(|thread| {
                thread.id == home
                    && thread.captain_home
                    && thread.captain == Some(captain)
                    && !thread.delegated
                    && thread.machine == "local"
            })
            .ok_or("The Captain's current primary home is unavailable.".into())
    }

    fn independent_crew_index(&self, captain: usize, id: usize) -> Result<usize, String> {
        self.require_active_captain(captain)?;
        let index = self
            .threads
            .iter()
            .position(|thread| thread.id == id)
            .ok_or("Unknown crew thread.")?;
        let thread = &self.threads[index];
        if thread.crew_captain != Some(captain)
            || thread.captain_home
            || thread.delegated
            || thread.parent.is_some()
            || thread.peer_origin.is_some()
            || thread.machine != "local"
        {
            return Err(
                "This control requires an independent root in this Captain's current crew.".into(),
            );
        }
        if thread.archived() {
            return Err("Restore the crew thread before sending or stopping work.".into());
        }
        Ok(index)
    }

    fn crew_prompt(&self, input: &Value, thread: &Thread) -> Result<Prompt, String> {
        keys(input, &["text", "attachments", "draft", "context"])?;
        let text = input["text"].as_str().ok_or("input.text is required.")?;
        let images = match input.get("attachments") {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or("input.attachments must be an array.")?
                .iter()
                .map(|value| Attachment::from_value(value).ok_or("Invalid crew attachment."))
                .collect::<Result<Vec<_>, _>>()?,
        };
        attachments::validate_prompt_images(&images)?;
        let context = input
            .get("context")
            .filter(|value| !value.is_null())
            .map(crate::composer_context::decode_message_context)
            .transpose()?;
        let received = crate::composer_tokens::received_prompt_with_context(
            text.into(),
            images,
            input.get("draft").cloned(),
            context,
        )?;
        let editor = received.draft.as_ref().map_or_else(
            || crate::composer_rich::draft_from_canonical(&received.text),
            crate::composer_rich::Snapshot::editor,
        );
        let draft_context = received
            .draft
            .as_ref()
            .filter(|draft| !draft.context.is_empty())
            .map(|draft| draft.context.clone())
            .or_else(|| {
                received
                    .context
                    .as_ref()
                    .map(|context| context.records.clone())
            })
            .unwrap_or_default();
        let reply = crate::composer_tokens::split_leading_reply_reference(
            &received.text,
            Some(&editor),
            received.context.as_ref(),
        )
        .map(|(record, _)| record);
        let server = self.machine_identity("local");
        let prompt = crate::composer_tokens::prompt_for_target_with_reply(
            &editor,
            &received.images,
            &draft_context,
            reply.as_ref(),
            Some("local"),
            Some(&server),
            &self.peers,
            &|machine| self.machine_label(machine),
        )?;
        if prompt.text.trim().is_empty() && prompt.images.is_empty() {
            return Err("Crew work needs a message or attachment.".into());
        }
        if let Some(error) = thread.validate_prompt_length(&prompt) {
            return Err(error);
        }
        if prompt.images.iter().any(Attachment::is_image) && thread.supports_images != Some(true) {
            return Err(thread.image_model_error());
        }
        Ok(prompt)
    }

    fn crew_thread(&self, captain: usize, args: &Value, agent: bool) -> Result<Thread, String> {
        let home_index = self.crew_home_index(captain)?;
        let home = &self.threads[home_index];
        let scope = self.captain_scope(captain);
        let project = match args.get("project") {
            None if scope.len() > 1 => {
                return Err("project is required for a Captain with several workspaces.".into());
            }
            None => scope.first().copied(),
            Some(Value::Null) if scope.is_empty() => None,
            Some(value) => {
                let project = value
                    .as_u64()
                    .and_then(|id| usize::try_from(id).ok())
                    .filter(|id| scope.contains(id))
                    .ok_or("The crew workspace must belong to the Captain's scope.")?;
                Some(project)
            }
        };
        if project.is_some_and(|id| {
            self.project(Some(id))
                .is_none_or(|project| project.machine != "local")
        }) {
            return Err("Crew creation stays on its Captain's owning server.".into());
        }
        if let Some(target) = args.get("target").and_then(Value::as_object)
            && target
                .keys()
                .any(|key| matches!(key.as_str(), "machine" | "environmentId" | "projectId"))
        {
            return Err(
                "Independent crew creation is local. Use project from the Captain's local scope."
                    .into(),
            );
        }
        let target = parse_delegation_target(args)?;
        let harness = target
            .as_ref()
            .and_then(|target| target.harness.as_deref())
            .unwrap_or(&home.harness);
        self.validate_harness(harness)?;
        let provider = if harness == "pi" { "cpa" } else { harness };
        if target
            .as_ref()
            .and_then(|target| target.provider.as_deref())
            .is_some_and(|name| name != provider)
        {
            return Err(format!(
                "The selected harness requires provider {provider}."
            ));
        }
        let models = if harness == home.harness {
            self.delegation_models(home).0
        } else {
            self.harness_models(harness)
        };
        let inherited = if harness == home.harness {
            self.inherited_selection(home_index)
        } else if harness == "pi" {
            self.default_selection()
        } else {
            None
        };
        let mut selection = target
            .as_ref()
            .and_then(|target| target.model.as_ref())
            .map(|model| Selection {
                provider: provider.into(),
                model: model.clone(),
                reasoning: target.as_ref().and_then(|target| target.reasoning.clone()),
            })
            .or(inherited);
        if let Some(reasoning) = target.as_ref().and_then(|target| target.reasoning.as_ref()) {
            selection
                .as_mut()
                .ok_or("Choose a model before setting crew reasoning.")?
                .reasoning = Some(reasoning.clone());
        }
        if let Some(selection) = &selection {
            self.validate_delegation_selection(
                &models,
                selection,
                target.as_ref().is_some_and(|target| target.model.is_some()),
            )?;
        }
        let mut thread = Thread::new(self.next_id);
        thread.harness = harness.into();
        thread.project = project;
        thread.env_mode = Some(ThreadEnvMode::Local);
        thread.captain = Some(captain);
        thread.crew_captain = Some(captain);
        thread.models = models;
        thread.native.access = match args.get("access") {
            Some(_) if agent => {
                return Err("Agents cannot request a new crew permission mode.".into());
            }
            Some(value) => value
                .as_str()
                .and_then(AccessMode::parse)
                .ok_or("Invalid crew access mode.")?,
            None if agent => {
                if home.native.access == AccessMode::ReadOnly {
                    AccessMode::default()
                } else {
                    home.native.access
                }
            }
            None => AccessMode::parse(&self.thread_defaults.access).unwrap_or_default(),
        };
        thread.native.access = self.effective_access_mode(&thread);
        if let Some(selection) = selection {
            thread.set_model(selection.provider, selection.model);
            if let Some(reasoning) = selection.reasoning {
                thread.set_thinking(reasoning);
            }
        }
        if let Some(title) = args.get("title") {
            let title = title
                .as_str()
                .filter(|title| {
                    !title.trim().is_empty()
                        && title.chars().count() <= 200
                        && !title.chars().any(char::is_control)
                })
                .ok_or("title must contain 1 to 200 characters on one line.")?;
            thread.title = title.into();
            thread.renamed = true;
        }
        Ok(thread)
    }

    pub fn captain_crew_create(
        &mut self,
        captain: usize,
        args: &Value,
        agent: bool,
        intent_id: uuid::Uuid,
    ) -> Result<Value, String> {
        keys(
            args,
            &[
                "captain",
                "project",
                "title",
                "input",
                "target",
                "access",
                "placement",
            ],
        )?;
        if args
            .get("captain")
            .is_some_and(|value| *value != json!(captain))
        {
            return Err("The crew command names a different Captain.".into());
        }
        let mut thread = self.crew_thread(captain, args, agent)?;
        let mut prompt = self.crew_prompt(&args["input"], &thread)?;
        let next = self
            .next_id
            .checked_add(1)
            .filter(|id| *id < usize::MAX)
            .ok_or("Thread IDs are exhausted.")?;
        self.crew_database()?;
        #[cfg(test)]
        let git_ctx = self.test_git_context.clone().unwrap_or_default();
        #[cfg(not(test))]
        let git_ctx = crate::vcs::GitContext::default();
        let placement = self.plan_thread_placement(
            &thread,
            placement::spec_from_args(args)?.as_ref(),
            &format!("crew:{intent_id}"),
            "local",
            &git_ctx,
        )?;
        if placement.read_only() {
            thread.native.access = AccessMode::ReadOnly;
        }
        let pending = placement.phase == placement::PlacementPhase::Pending;
        if placement.uses_worktree() {
            thread.worktree_path = placement.checkout.clone();
            thread.branch = placement.branch.clone();
            thread.env_mode = Some(ThreadEnvMode::Worktree);
        }
        thread.placement = Some(placement);
        attachments::stage_prompt_files(&mut prompt)?;
        let id = thread.id;
        thread.server_owned = true;
        thread.begin_prompt(prompt);
        self.threads.push(thread);
        self.next_id = next;
        let index = self.threads.len() - 1;
        self.sync_memory_links();
        self.refresh_captain_contexts();
        if !pending {
            self.connect_thread(index);
        }
        // A synchronous startup refusal restores the prompt to a draft. Save
        // that exact input in this creation transaction, not on a later tick.
        if self.threads[index].has_draft_content() {
            self.threads[index].recovery_command = Some(intent_id);
        }
        self.collect_recovered_input(index);
        self.threads[index].recovery_command = None;
        self.persistence_scope = None;
        Ok(
            json!({"saved":true,"captain":captain,"threadId":id,"runId":self.threads[index].run_id,
            "runtimeGeneration":self.threads[index].request_generation,
            "placement":self.threads[index].placement.as_ref().map(placement::ThreadPlacement::value)}),
        )
    }

    pub fn captain_crew_send(
        &mut self,
        captain: usize,
        id: usize,
        input: &Value,
    ) -> Result<Value, String> {
        let index = self.independent_crew_index(captain, id)?;
        let thread = &self.threads[index];
        // A failed placement starts nothing, so its held input is recoverable
        // by new Captain work. Other interrupted input still needs review.
        let placement_failed = thread
            .placement
            .as_ref()
            .is_some_and(|placement| placement.phase == placement::PlacementPhase::Failed);
        if thread.status == Status::Paused
            || thread.phase == "Stopping"
            || (thread.native.queue_held && !placement_failed)
            || thread.has_recoverable_native_inputs()
            || self.recovered_inputs.contains_key(&id)
            || (thread.status == Status::Failed && thread.has_draft_content() && !placement_failed)
        {
            return Err(
                "Review or resume the crew thread's interrupted input before adding work.".into(),
            );
        }
        let mut prompt = self.crew_prompt(input, thread)?;
        let serial = thread
            .serial
            .checked_add(1)
            .ok_or("Queued input IDs are exhausted.")?;
        self.crew_database()?;
        attachments::stage_prompt_files(&mut prompt)?;
        let thread = &mut self.threads[index];
        if placement_failed {
            thread.native.queue_held = false;
        }
        thread.serial = serial;
        thread
            .native
            .queue
            .push_back(pi_session::QueuedMessage { id: serial, prompt });
        // Existing queues serialize as held input at restart. Never rebuild a
        // follow-up by replaying its tool receipt.
        thread.drive_session();
        if self.threads[index]
            .placement
            .as_ref()
            .is_some_and(|placement| placement.phase == placement::PlacementPhase::Failed)
        {
            self.request_reprovision(index);
        }
        if self.threads[index].pi.is_none()
            && self.threads[index]
                .placement
                .as_ref()
                .is_none_or(|placement| placement.phase == placement::PlacementPhase::Ready)
        {
            self.connect_thread(index);
        }
        self.persistence_scope = None;
        Ok(json!({"saved":true,"captain":captain,"threadId":id,"inputId":serial}))
    }

    pub fn captain_crew_stop(
        &mut self,
        captain: usize,
        id: usize,
        run: &Value,
        generation: &str,
    ) -> Result<Value, String> {
        let index = self.independent_crew_index(captain, id)?;
        let thread = &self.threads[index];
        if !run.is_null() && !run.is_string() {
            return Err("runId must be text or null.".into());
        }
        if json!(thread.run_id) != *run || thread.request_generation != generation {
            return Err("This stop belongs to an earlier crew run or runtime generation.".into());
        }
        self.crew_database()?;
        let idle = !thread.status.busy() && !thread.queued();
        if !idle {
            self.threads[index].stop();
        }
        self.persistence_scope = None;
        Ok(json!({"saved":true,"threadId":id,"runId":run,"alreadyIdle":idle}))
    }

    pub(super) fn captain_crew_tool(
        &mut self,
        caller: usize,
        tool: &str,
        args: &Value,
    ) -> Result<Value, String> {
        if self.client.is_some() {
            return Err("Crew tools run only on the Captain's owning server.".into());
        }
        let thread = self
            .threads
            .iter()
            .find(|thread| {
                thread.id == caller
                    && thread.captain_home
                    && !thread.delegated
                    && thread.peer_origin.is_none()
            })
            .ok_or("Only a Captain home may control independent crew.")?;
        let captain = thread.captain.ok_or("The caller has no Captain profile.")?;
        let method = match tool {
            "captain_crew_create" => "captain.crew.create",
            "captain_crew_send" => "captain.crew.send",
            "captain_crew_stop" => "captain.crew.stop",
            _ => return Err("Unknown crew control.".into()),
        };
        let request = args["requestId"]
            .as_str()
            .and_then(|id| uuid::Uuid::parse_str(id).ok())
            .ok_or("requestId must be a stable UUID.")?;
        let mut intent = args.clone();
        intent
            .as_object_mut()
            .ok_or("Crew arguments must be an object.")?
            .remove("requestId");
        let command = Command {
            command_id: request,
            method: method.into(),
            params: intent.clone(),
        };
        let database = crate::database::for_path(&self.data_dir.join("index.json"))
            .ok_or("Crew control requires the durable server.")?;
        if let Some(receipt) = database.captain_tool_receipt(captain, request)? {
            if receipt.hash != command.hash() {
                return Err("This requestId already names a different crew operation.".into());
            }
            let mut result = receipt.result;
            result["duplicate"] = json!(true);
            let current = result["threadId"]
                .as_u64()
                .and_then(|id| self.threads.iter().find(|thread| thread.id as u64 == id));
            result["exists"] = json!(current.is_some());
            result["currentCrewCaptain"] = json!(current.and_then(|thread| thread.crew_captain));
            return Ok(result);
        }
        self.require_active_captain(captain)?;
        if self.captain_home(captain) != Some(caller) {
            return Err(
                "Only the Captain's current primary home may start a crew operation.".into(),
            );
        }
        let result = match tool {
            "captain_crew_create" => {
                keys(
                    &intent,
                    &["prompt", "project", "title", "target", "placement"],
                )?;
                let prompt = intent["prompt"]
                    .as_str()
                    .filter(|text| !text.trim().is_empty() && text.len() <= 64000)
                    .ok_or("prompt must contain 1 to 64000 UTF-8 bytes.")?
                    .to_owned();
                intent.as_object_mut().unwrap().remove("prompt");
                intent["input"] = json!({"text":prompt});
                self.captain_crew_create(captain, &intent, true, request)?
            }
            "captain_crew_send" => {
                keys(&intent, &["threadId", "message"])?;
                let id = intent["threadId"]
                    .as_u64()
                    .and_then(|id| usize::try_from(id).ok())
                    .ok_or("threadId is required.")?;
                let message = intent["message"]
                    .as_str()
                    .filter(|text| !text.trim().is_empty() && text.len() <= 64000)
                    .ok_or("message must contain 1 to 64000 UTF-8 bytes.")?;
                let text = format!(
                    "{INJECTION_PREFIX}[delivery crew-input-{request}]\nShina: work from Captain #{captain}\n{message}"
                );
                self.captain_crew_send(captain, id, &json!({"text":text}))?
            }
            _ => {
                keys(&intent, &["threadId", "runId", "runtimeGeneration"])?;
                let id = intent["threadId"]
                    .as_u64()
                    .and_then(|id| usize::try_from(id).ok())
                    .ok_or("threadId is required.")?;
                let run = intent
                    .get("runId")
                    .ok_or("runId is required, including null for an idle thread.")?;
                let generation = intent["runtimeGeneration"]
                    .as_str()
                    .ok_or("runtimeGeneration is required.")?;
                self.captain_crew_stop(captain, id, run, generation)?
            }
        };
        if let Err(error) = database.stage_captain_tool_receipt(captain, &command, result.clone()) {
            database.fail(error.clone());
            self.storage_error = Some(error.clone());
            return Err(error);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeState;
    use std::sync::Mutex;

    type Channels = Arc<Mutex<Vec<(usize, mpsc::Receiver<Value>, mpsc::SyncSender<Event>)>>>;

    fn fixture() -> (RuntimeState, Channels) {
        let directory = std::env::temp_dir().join(format!("shina-crew-{}", uuid::Uuid::new_v4()));
        let mut runtime = RuntimeState::open(directory, Arc::new(|| {})).unwrap();
        runtime
            .database
            .as_ref()
            .unwrap()
            .issue_device("maintainer", "fixture-token")
            .unwrap();
        runtime.state.captains = Captain::presets();
        runtime.state.open_captain(0);
        let model = AgentModel::from_value(&json!({
            "provider":"cpa","id":"crew-model","name":"Crew model","input":["text","image"]
        }))
        .unwrap();
        runtime.state.catalog.models = vec![model.clone()];
        runtime.state.catalog.default_model = Some(model.clone());
        let home = runtime.state.captain_home(0).unwrap();
        let index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == home)
            .unwrap();
        runtime.state.threads[index].models = vec![model];
        runtime.state.threads[index].set_model("cpa".into(), "crew-model".into());
        let channels: Channels = Arc::default();
        let hooked = channels.clone();
        runtime.state.test_connect_pi = Some(Box::new(move |id, _| {
            let (session, commands, events) = Pi::test_connection();
            hooked.lock().unwrap().push((id, commands, events));
            Some(session)
        }));
        (runtime, channels)
    }

    fn create(runtime: &mut RuntimeState, text: &str) -> usize {
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "captain.crew.create",
                    json!({
                        "captain":0,"input":{"text":text}
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        receipt["result"]["threadId"].as_u64().unwrap() as usize
    }

    #[test]
    fn independent_crew_creation_rejects_before_allocation_and_never_reuses_a_draft() {
        let (mut runtime, _) = fixture();
        let original = runtime.state.next_id;
        let count = runtime.state.threads.len();
        for args in [
            json!({"captain":0,"input":{"text":""}}),
            json!({"captain":0,"project":99,"input":{"text":"Work"}}),
            json!({"captain":0,"input":{"text":"Work"},"target":{"model":"unknown"}}),
            json!({"captain":0,"input":{"text":"Work","attachments":null}}),
            json!({"captain":0,"input":{"text":"Work","context":{"version":999}}}),
            json!({"captain":0,"input":{"text":"Work"},"access":"invented"}),
            json!({"captain":0,"input":{"text":"Work"},"envMode":"worktree"}),
        ] {
            let receipt = runtime
                .command("maintainer", Command::new("captain.crew.create", args))
                .unwrap();
            assert_eq!(receipt["status"], "rejected", "{receipt}");
            assert_eq!(runtime.state.next_id, original);
            assert_eq!(runtime.state.threads.len(), count);
        }
        let focus = runtime.state.current().id;
        let tabs = runtime.state.tabs.clone();
        runtime
            .state
            .current_mut()
            .draft
            .replace_text("Unsent coordinator draft");
        let id = create(&mut runtime, "First crew input");
        assert_eq!(id, original);
        assert_eq!(runtime.state.current().id, focus);
        assert_eq!(runtime.state.tabs, tabs);
        assert_eq!(
            runtime.state.current().draft.text,
            "Unsent coordinator draft"
        );
        assert!(runtime.state.tasks.is_empty());
        let crew = runtime
            .state
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .unwrap();
        assert!(!crew.delegated);
        assert_eq!(crew.parent, None);
        assert_eq!(crew.crew_captain, Some(0));
        assert_eq!(crew.messages[0].text, "First crew input");
    }

    #[test]
    fn independent_crew_mcp_receipts_and_first_prompts_commit_before_reply_and_dedupe_in_one_batch()
    {
        let (mut runtime, channels) = fixture();
        let database = runtime.database.clone().unwrap();
        let home = runtime.state.captain_home(0).unwrap();
        runtime.state.captains[0].instructions = "PRIVATE-PROFILE".into();
        for (name, text) in [
            ("AGENTS.md", "PRIVATE-AGENTS"),
            ("NOTES.md", "PRIVATE-PLAN"),
            ("CREW.md", "PUBLIC-CREW"),
        ] {
            runtime.state.seed_captain_document(
                0,
                name,
                CaptainDocument {
                    text: text.into(),
                    revision: 1,
                    updated_at: 1,
                },
            );
        }
        let request = uuid::Uuid::new_v4();
        let args = json!({"requestId":request,"prompt":"Do durable crew work"});
        let (reply, received) = mpsc::channel();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.mcp_request(crate::mcp::Request {
            thread: home,
            tool: "captain_crew_create".into(),
            args: args.clone(),
            reply,
        });
        assert!(
            received.try_recv().is_err(),
            "saved reply is deferred until commit"
        );
        let duplicate = runtime
            .state
            .mcp_call(home, "captain_crew_create", &args)
            .unwrap();
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(
            runtime
                .state
                .threads
                .iter()
                .filter(|thread| thread.crew_captain == Some(0))
                .count(),
            1
        );
        assert!(
            channels
                .lock()
                .unwrap()
                .iter()
                .all(|(_, commands, _)| commands.try_recv().is_err())
        );
        runtime.persist_for_test(capture.finish()).unwrap();
        let saved = received.recv().unwrap().unwrap();
        let id = saved["threadId"].as_u64().unwrap() as usize;
        assert!(
            database
                .captain_tool_receipt(0, request)
                .unwrap()
                .unwrap()
                .seq
                > 0
        );
        let wire = channels
            .lock()
            .unwrap()
            .iter()
            .find(|(thread, _, _)| *thread == id)
            .unwrap()
            .1
            .try_recv()
            .unwrap();
        let message = wire["message"].as_str().unwrap();
        assert!(message.contains("PUBLIC-CREW"), "{wire}");
        for private in ["PRIVATE-PROFILE", "PRIVATE-AGENTS", "PRIVATE-PLAN"] {
            assert!(!message.contains(private), "{wire}");
        }
        assert!(runtime.state.tasks.is_empty());
        let mut changed = args.clone();
        changed["prompt"] = json!("Different work");
        assert!(
            runtime
                .state
                .mcp_call(home, "captain_crew_create", &changed)
                .is_err()
        );
        let bytes = database.read("index.json").unwrap().unwrap().to_string();
        assert!(
            bytes.contains("Do durable crew work"),
            "the first input is durable"
        );
        let registered: i64 = database.test_connection(|connection| {
            connection
                .query_row(
                    "SELECT count(*) FROM devices WHERE id LIKE 'mcp/%'",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        });
        assert_eq!(
            registered, 0,
            "receipt audit actors are never authentication devices"
        );
    }

    #[test]
    fn independent_crew_tool_retries_after_restart_release_and_delete_never_recreate_work() {
        let (mut runtime, _) = fixture();
        let database = runtime.database.clone().unwrap();
        let home = runtime.state.captain_home(0).unwrap();
        let args = json!({"requestId":uuid::Uuid::new_v4(),"prompt":"Saved crew input"});
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        let saved = runtime
            .state
            .mcp_call(home, "captain_crew_create", &args)
            .unwrap();
        let id = saved["threadId"].as_u64().unwrap() as usize;
        runtime.persist_for_test(capture.finish()).unwrap();
        let directory = runtime.state.data_dir.clone();
        drop(runtime);
        drop(database);
        let mut runtime = RuntimeState::open(directory, Arc::new(|| {})).unwrap();
        let count = runtime.state.threads.len();
        let repeated = runtime
            .state
            .mcp_call(home, "captain_crew_create", &args)
            .unwrap();
        assert_eq!(repeated["threadId"], id);
        assert_eq!(runtime.state.threads.len(), count);
        let index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == id)
            .unwrap();
        // A read-only retry must reflect an existing root's current membership.
        runtime.state.threads[index].crew_captain = None;
        runtime.state.threads[index].captain = None;
        let repeated = runtime
            .state
            .mcp_call(home, "captain_crew_create", &args)
            .unwrap();
        assert!(repeated["currentCrewCaptain"].is_null());
        runtime.state.threads.remove(index);
        let repeated = runtime
            .state
            .mcp_call(home, "captain_crew_create", &args)
            .unwrap();
        assert_eq!(repeated["exists"], false);
        assert_eq!(runtime.state.threads.len(), count - 1);
    }

    #[test]
    fn independent_crew_send_survives_restart_as_held_input_and_stop_fences_new_runs() {
        let (mut runtime, _) = fixture();
        let id = create(&mut runtime, "First input");
        let index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == id)
            .unwrap();
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "captain.crew.send",
                    json!({
                        "captain":0,"threadId":id,"input":{"text":"Next input"}
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert_eq!(runtime.state.threads[index].native.queue.len(), 1);
        let generation = runtime.state.threads[index].request_generation.clone();
        let run = json!(runtime.state.threads[index].run_id);
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "captain.crew.stop",
                    json!({
                        "captain":0,"threadId":id,"runId":"old-run","runtimeGeneration":generation
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "rejected");
        assert_eq!(json!(runtime.state.threads[index].run_id), run);
        let directory = runtime.state.data_dir.clone();
        drop(runtime);
        let mut reopened = RuntimeState::open(directory, Arc::new(|| {})).unwrap();
        let crew = reopened
            .state
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .unwrap();
        assert!(crew.native.queue_held);
        assert_eq!(crew.native.queue[0].prompt.text, "Next input");
        assert!(crew.pi.is_none());
        let receipt = reopened.command("maintainer", Command::new("captain.crew.send", json!({
            "captain":0,"threadId":id,"input":{"text":"Must not release recovered input"}
        }))).unwrap();
        assert_eq!(receipt["status"], "rejected");
    }

    #[test]
    fn independent_crew_creation_preserves_files_and_refuses_undeclared_image_support() {
        let (mut runtime, _) = fixture();
        let file = attachments::store_uploaded_file(
            &runtime.state.data_dir.join("attachments"),
            "literal.txt",
            "text/plain",
            b"Literal attachment bytes",
        )
        .unwrap();
        let id = runtime.state.next_id;
        let receipt = runtime.command("maintainer", Command::new("captain.crew.create", json!({
            "captain":0,"input":{"text":"Read this file","attachments":[file.to_value()]}
        }))).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let thread = runtime
            .state
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .unwrap();
        assert_eq!(thread.messages[0].text, "Read this file");
        assert_eq!(thread.messages[0].images[0].name, "literal.txt");
        assert_eq!(
            attachments::read(&thread.messages[0].images[0]).unwrap(),
            b"Literal attachment bytes"
        );
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let image = attachments::store_uploaded(
            &attachments::directory().unwrap(),
            "tiny.png",
            bytes.get_ref(),
        )
        .unwrap();
        let before = runtime.state.next_id;
        for model in [
            json!({"provider":"cpa","id":"crew-model","input":["text"]}),
            json!({"provider":"cpa","id":"crew-model"}),
        ] {
            let model = AgentModel::from_value(&model).unwrap();
            runtime.state.catalog.models = vec![model.clone()];
            runtime.state.catalog.default_model = Some(model);
            let receipt = runtime.command("maintainer", Command::new("captain.crew.create", json!({
                "captain":0,"input":{"text":"Inspect this","attachments":[image.to_value()]}
            }))).unwrap();
            assert_eq!(receipt["status"], "rejected", "{receipt}");
            assert_eq!(runtime.state.next_id, before);
        }
    }

    #[test]
    fn independent_crew_startup_failure_keeps_the_saved_first_input_and_receipt() {
        let (mut runtime, channels) = fixture();
        runtime.state.test_connect_error = Some("Injected startup failure".into());
        let command = Command::new(
            "captain.crew.create",
            json!({
                "captain":0,"input":{"text":"Keep this first input"}
            }),
        );
        let receipt = runtime.command("maintainer", command.clone()).unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let id = receipt["result"]["threadId"].as_u64().unwrap() as usize;
        let thread = runtime
            .state
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .unwrap();
        assert_eq!(thread.status, Status::Failed);
        assert!(
            runtime.state.recovered_inputs[&id]
                .to_string()
                .contains("Keep this first input")
        );
        assert_eq!(
            runtime.state.recovered_inputs[&id]["commandId"],
            json!(command.command_id)
        );
        let persisted = runtime
            .database
            .as_ref()
            .unwrap()
            .read("index.json")
            .unwrap()
            .unwrap();
        assert!(
            persisted["recoveredInputs"][id.to_string()]
                .to_string()
                .contains("Keep this first input")
        );
        assert!(channels.lock().unwrap().is_empty());
        assert_eq!(runtime.command("maintainer", command).unwrap(), receipt);
    }

    #[test]
    fn independent_crew_control_refuses_workers_foreign_captains_and_new_earlier_home_intents() {
        let (mut runtime, _) = fixture();
        let home = runtime.state.captain_home(0).unwrap();
        let id = create(&mut runtime, "First input");
        let args = json!({"requestId":uuid::Uuid::new_v4(),"prompt":"Unauthorized root"});
        assert!(
            runtime
                .state
                .mcp_call(id, "captain_crew_create", &args)
                .is_err()
        );
        let database = runtime.database.clone().unwrap();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.reset_captain_conversation(0).unwrap();
        runtime.persist_for_test(capture.finish()).unwrap();
        assert!(
            runtime
                .state
                .mcp_call(home, "captain_crew_create", &args)
                .is_err()
        );
        runtime.state.open_captain(1);
        let other = runtime.state.captain_home(1).unwrap();
        assert!(
            runtime
                .state
                .mcp_call(
                    other,
                    "captain_crew_send",
                    &json!({
                        "requestId":uuid::Uuid::new_v4(),"threadId":id,"message":"Wrong Captain"
                    })
                )
                .is_err()
        );
        assert!(
            runtime
                .state
                .mcp_call(
                    other,
                    "captain_crew_create",
                    &json!({
                        "requestId":uuid::Uuid::new_v4(),"captain":0,"prompt":"Forged Captain"
                    })
                )
                .is_err()
        );
        assert_eq!(
            runtime
                .state
                .threads
                .iter()
                .filter(|thread| thread.crew_captain == Some(0))
                .count(),
            1
        );
    }

    #[test]
    fn independent_crew_mcp_send_dedupes_and_never_approves_a_provider_request() {
        let (mut runtime, channels) = fixture();
        let home = runtime.state.captain_home(0).unwrap();
        let id = create(&mut runtime, "First input");
        let index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == id)
            .unwrap();
        runtime.state.threads[index].request = Some(json!({"id":"confirm-1","method":"confirm"}));
        let args =
            json!({"requestId":uuid::Uuid::new_v4(),"threadId":id,"message":"Follow-up work"});
        let database = runtime.database.clone().unwrap();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime
            .state
            .mcp_call(home, "captain_crew_send", &args)
            .unwrap();
        runtime
            .state
            .mcp_call(home, "captain_crew_send", &args)
            .unwrap();
        assert_eq!(runtime.state.threads[index].native.queue.len(), 1);
        assert_eq!(
            runtime.state.threads[index].request.as_ref().unwrap()["id"],
            "confirm-1"
        );
        runtime.persist_for_test(capture.finish()).unwrap();
        assert_eq!(runtime.state.current().id, home);
        for (_, commands, _) in channels.lock().unwrap().iter() {
            assert!(
                commands
                    .try_iter()
                    .all(|command| command["type"] != "extension_ui_response")
            );
        }
        let repeated = runtime
            .state
            .mcp_call(home, "captain_crew_send", &args)
            .unwrap();
        assert_eq!(repeated["duplicate"], true);
        assert_eq!(runtime.state.threads[index].native.queue.len(), 1);
        let mut different = args;
        different["message"] = json!("Changed intent");
        assert!(
            runtime
                .state
                .mcp_call(home, "captain_crew_send", &different)
                .is_err()
        );
    }

    #[test]
    fn independent_crew_mcp_stop_retry_never_stops_a_newer_run() {
        let (mut runtime, channels) = fixture();
        let home = runtime.state.captain_home(0).unwrap();
        let id = create(&mut runtime, "First input");
        let index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == id)
            .unwrap();
        let args = json!({"requestId":uuid::Uuid::new_v4(),"threadId":id,
            "runId":runtime.state.threads[index].run_id,"runtimeGeneration":runtime.state.threads[index].request_generation});
        let database = runtime.database.clone().unwrap();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime
            .state
            .mcp_call(home, "captain_crew_stop", &args)
            .unwrap();
        runtime.persist_for_test(capture.finish()).unwrap();
        let serial = runtime.state.threads[index].stop_serial;
        for (_, commands, _) in channels.lock().unwrap().iter() {
            commands.try_iter().for_each(drop);
        }
        runtime.state.threads[index].run_id = Some("newer-run".into());
        runtime.state.threads[index].status = Status::Running;
        runtime.state.threads[index].phase.clear();
        let repeated = runtime
            .state
            .mcp_call(home, "captain_crew_stop", &args)
            .unwrap();
        assert_eq!(repeated["duplicate"], true);
        assert_eq!(runtime.state.threads[index].stop_serial, serial);
        assert_eq!(runtime.state.threads[index].status, Status::Running);
        assert!(
            channels
                .lock()
                .unwrap()
                .iter()
                .all(|(_, commands, _)| commands.try_recv().is_err())
        );
    }

    #[test]
    fn independent_crew_sql_failure_saves_neither_root_nor_tool_receipt_and_dispatches_nothing() {
        let (mut runtime, channels) = fixture();
        let home = runtime.state.captain_home(0).unwrap();
        let database = runtime.database.clone().unwrap();
        let before = database.read("index.json").unwrap().unwrap();
        database.test_connection(|connection| connection.execute_batch(
            "CREATE TRIGGER fail_crew_receipt BEFORE INSERT ON command_receipts
             WHEN NEW.device LIKE 'mcp/captain/%' BEGIN SELECT RAISE(ABORT,'injected crew receipt failure'); END;"
        ).unwrap());
        let request = uuid::Uuid::new_v4();
        let (reply, received) = mpsc::channel();
        database.begin().unwrap();
        let capture = crate::outbox::Capture::begin();
        runtime.state.mcp_request(crate::mcp::Request {
            thread: home,
            tool: "captain_crew_create".into(),
            args: json!({"requestId":request,"prompt":"Must not dispatch before commit"}),
            reply,
        });
        assert!(runtime.persist_for_test(capture.finish()).is_err());
        assert!(received.try_recv().is_err());
        assert!(
            channels
                .lock()
                .unwrap()
                .iter()
                .all(|(_, commands, _)| commands.try_recv().is_err())
        );
        assert!(database.read("index.json").is_err());
        database.test_connection(|connection| {
            assert_eq!(
                crate::projection_store::read(connection, "index.json")
                    .unwrap()
                    .unwrap(),
                before
            );
            let count: i64 = connection
                .query_row(
                    "SELECT count(*) FROM command_receipts WHERE device LIKE 'mcp/captain/%'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn independent_crew_work_counts_follow_canonical_roots_and_current_task_routes_after_reset() {
        let (mut runtime, _) = fixture();
        let home = runtime.state.captain_home(0).unwrap();
        create(&mut runtime, "Independent crew work");
        runtime.state.open_captain(1);
        let other_home = runtime.state.captain_home(1).unwrap();
        for moved in [false, true] {
            let id = runtime.state.next_id;
            runtime.state.next_id += 1;
            let mut child = Thread::new(id);
            child.parent = Some(home);
            child.delegated = true;
            if moved {
                child.crew_captain = Some(0);
                child.captain = Some(0);
            }
            let mut task = tasks::Task::new(&child, "Existing durable task", None);
            task.state = TaskState::Running;
            task.started_at = Some(lifecycle::now());
            if moved {
                child.crew_captain = Some(1);
                child.captain = Some(1);
            }
            child.status = Status::Running;
            runtime.state.tasks.push(task);
            runtime.state.threads.push(child);
        }
        assert_eq!(runtime.state.crew_working_now(home), 2);
        assert_eq!(runtime.state.crew_working_now(other_home), 1);
        let receipt = runtime
            .command(
                "maintainer",
                Command::new("captain.reset", json!({"captain":0})),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let current = runtime.state.captain_home(0).unwrap();
        assert_ne!(current, home);
        assert_eq!(runtime.state.crew_working_now(current), 2);
        assert_eq!(runtime.state.crew_working_now(home), 0);
        assert_eq!(runtime.state.home_status(current), Status::Running);
    }

    #[test]
    fn independent_crew_multiple_workspaces_require_an_explicit_scoped_project() {
        let (mut runtime, _) = fixture();
        runtime.state.projects.entries = vec![
            Project::new(1, runtime.state.data_dir.join("first")),
            Project::new(2, runtime.state.data_dir.join("second")),
        ];
        runtime.state.projects.next_id = 3;
        runtime.state.set_captain_scope(0, vec![1, 2]).unwrap();
        let before = runtime.state.next_id;
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "captain.crew.create",
                    json!({
                        "captain":0,"input":{"text":"Ambiguous workspace"}
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "rejected");
        assert_eq!(runtime.state.next_id, before);
        let receipt = runtime
            .command(
                "maintainer",
                Command::new(
                    "captain.crew.create",
                    json!({
                        "captain":0,"project":2,"input":{"text":"Exact workspace"}
                    }),
                ),
            )
            .unwrap();
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        let id = receipt["result"]["threadId"].as_u64().unwrap() as usize;
        let index = runtime
            .state
            .threads
            .iter()
            .position(|thread| thread.id == id)
            .unwrap();
        assert_eq!(runtime.state.threads[index].project, Some(2));
        assert_eq!(
            runtime.state.thread_directory(index).unwrap(),
            runtime.state.data_dir.join("second")
        );
    }

    /// The server resolves coordination authority from the calling thread's role and the
    /// Captain's current ownership, never from model-supplied identifiers. Read-only
    /// harness policy can now admit crew tools; these rejections are what keeps them safe.
    #[test]
    fn crew_tools_reject_every_caller_that_is_not_the_current_primary_home() {
        let (mut runtime, _) = fixture();
        let home = runtime.state.captain_home(0).unwrap();
        let ordinary = runtime
            .state
            .threads
            .iter()
            .find(|thread| thread.id != home && !thread.captain_home)
            .map(|thread| thread.id)
            .unwrap();
        let push = |runtime: &mut RuntimeState, fill: &dyn Fn(&mut Thread)| -> usize {
            let id = runtime.state.next_id;
            runtime.state.next_id += 1;
            let mut thread = Thread::new(id);
            fill(&mut thread);
            runtime.state.threads.push(thread);
            id
        };
        let earlier = push(&mut runtime, &|thread| {
            thread.captain_home = true;
            thread.captain = Some(0);
        });
        let crew = push(&mut runtime, &|thread| {
            thread.crew_captain = Some(0);
        });
        let worker = push(&mut runtime, &|thread| {
            thread.delegated = true;
            thread.parent = Some(home);
        });
        let remote = push(&mut runtime, &|thread| {
            thread.delegated = true;
            thread.peer_origin = Some(crate::model::remote_tasks::PeerOrigin {
                device_id: "fixture-peer".into(),
                server_id: "fixture-server".into(),
                server_label: "Fixture".into(),
                parent_thread_id: 1,
                task_id: "task".into(),
                captain: "Ember".into(),
                memories: vec![],
                result: None,
                acknowledged: false,
                attempt_message: String::new(),
                attempt_id: None,
                cancelled_commands: vec![],
                reply_after_position: 0,
                document_reads: false,
            });
        });
        let create = |runtime: &mut RuntimeState, caller: usize| {
            runtime.state.mcp_call(
                caller,
                "captain_crew_create",
                &json!({"requestId":uuid::Uuid::new_v4(),"prompt":"Unauthorized crew work"}),
            )
        };
        for caller in [crew, worker, remote, ordinary] {
            assert_eq!(
                create(&mut runtime, caller).unwrap_err(),
                "Only a Captain home may control independent crew."
            );
        }
        // An earlier home of the same Captain is a record, not a coordinator.
        assert_eq!(
            create(&mut runtime, earlier).unwrap_err(),
            "Only the Captain's current primary home may start a crew operation."
        );
        // A demoted Captain's revoked scope blocks even its surviving home.
        runtime.state.captain_lifecycles.insert(
            0,
            captain_lifecycle::CaptainLifecycle {
                phase: CaptainPhase::Retired,
                ..Default::default()
            },
        );
        let error = create(&mut runtime, home).unwrap_err();
        assert!(error.contains("demoted"), "{error}");
    }
}
