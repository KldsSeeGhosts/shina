//! Client-side independent-crew draft. An ordinary client draft carries a crew
//! binding (`Thread::crew_draft`) naming a Captain on its owning machine. The
//! first Send issues one atomic `captain.crew.create` command, so membership
//! and the first input commit together and a failure cannot leave a blank or
//! half-joined thread. Design: `docs/captains.md` "Independent crew commands"
//! and the follow-on UI design's "New crew thread" flow. There is no KSG Code
//! equivalent for the binding; the send mirrors `promote_client_draft`'s
//! snapshot/clear/restore lifecycle on the existing `client_creating` and
//! `remote_send` fields.
use super::*;
use crate::protocol::Command;

/// The crew a client draft's first send joins. `server` is the machine key the
/// binding was taken on (`machines.selected` at that moment), `captain` the
/// Captain index on that machine, and `name` the Captain's display name at
/// bind time, kept so the chip stays readable if the profile later renames.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CrewDraftTarget {
    pub server: String,
    pub captain: usize,
    pub name: String,
}

impl State {
    /// Availability for the Brief's create action, before a draft has a crew binding.
    pub fn crew_creation_unavailable(
        &self,
        expected_server: &str,
        captain: usize,
    ) -> Option<String> {
        if expected_server != self.machines.selected {
            return Some("That action belonged to another machine.".into());
        }
        if let Err(error) = self.require_active_captain(captain) {
            return Some(error);
        }
        if self.client.is_some() && !self.machine_connected(expected_server) {
            return Some(format!(
                "{} is unreachable.",
                self.machine_label(expected_server)
            ));
        }
        match self.captain_access(captain) {
            CaptainAccess::Full => {}
            CaptainAccess::Earlier => {
                return Some("Read only: an earlier conversation is open.".into());
            }
            CaptainAccess::UpdateRequired => {
                return Some("Update the linked Shina to create crew.".into());
            }
            CaptainAccess::Unreachable => return Some("The machine is unreachable.".into()),
            CaptainAccess::NoRecords => return Some("No captain records.".into()),
        }
        self.crew_control_capability(expected_server).err()
    }

    /// `captain-crew-control-v1` on the draft's owning machine. A missing
    /// capability means the peer predates crew creation; an unlinked server is
    /// reported separately. Without a client this desktop is the server.
    fn crew_control_capability(&self, server: &str) -> Result<(), String> {
        let Some(client) = &self.client else {
            return Ok(());
        };
        if client.supports_capability(server, crate::protocol::CAPTAIN_CREW_CONTROL) {
            return Ok(());
        }
        if client.commands_for(server).is_none() && server != "local" {
            return Err("This machine is not linked.".into());
        }
        Err(crate::protocol::CAPTAIN_CREW_CONTROL_UPDATE_REQUIRED.into())
    }

    /// The notice a draft keeps after its crew binding is removed because the
    /// draft left the Captain's machine. `select_machine`, draft retargeting
    /// and any other path that moves a draft across machines use this text.
    pub fn crew_draft_removed_notice(&self, target: &CrewDraftTarget) -> String {
        format!(
            "Removed from {}'s crew: {} lives on {}.",
            target.name,
            target.name,
            self.machine_label(&target.server)
        )
    }

    /// Drop every crew binding in the current thread list that names a machine
    /// other than the selected one, replacing it with the explanatory notice.
    /// `select_machine` calls this after `self.threads` holds the new machine's
    /// threads; `apply_draft_retarget`/`retarget_draft_choose` call it after
    /// the moved draft lands. Switching back does not restore the binding.
    pub fn crew_draft_strip_foreign(&mut self) {
        let notices: Vec<String> = self
            .threads
            .iter()
            .filter_map(|thread| {
                thread
                    .crew_draft
                    .as_ref()
                    .filter(|target| target.server != self.machines.selected)
                    .map(|target| self.crew_draft_removed_notice(target))
            })
            .collect();
        let mut notices = notices.into_iter();
        for thread in &mut self.threads {
            if thread
                .crew_draft
                .as_ref()
                .is_some_and(|target| target.server != self.machines.selected)
            {
                thread.crew_draft = None;
                thread.crew_draft_notice = Some(notices.next().expect("notice per binding"));
            }
        }
    }

    /// Why the open draft's crew binding blocks its send, in the same order the
    /// send path checks: binding/machine staleness, Captain state, capability,
    /// then workspace scope and composer settings the backend refuses. Returns
    /// `None` when the chip is absent or the binding can send.
    pub fn crew_draft_unavailable(&self) -> Option<String> {
        let target = self.current().crew_draft.as_ref()?;
        self.crew_draft_blocker(target).err()
    }

    fn crew_draft_blocker(&self, target: &CrewDraftTarget) -> Result<(), String> {
        if target.server != self.machines.selected {
            return Err(format!(
                "This draft belongs to {}.",
                self.machine_label(&target.server)
            ));
        }
        if self.client.is_some() && !self.machine_connected(&target.server) {
            return Err(format!(
                "{} is unreachable.",
                self.machine_label(&target.server)
            ));
        }
        match self.captain_access(target.captain) {
            CaptainAccess::Full => {}
            CaptainAccess::Earlier => {
                return Err("Read only: an earlier conversation is open.".into());
            }
            CaptainAccess::UpdateRequired => {
                return Err("Update the linked Shina to create crew.".into());
            }
            CaptainAccess::Unreachable => return Err("The machine is unreachable.".into()),
            CaptainAccess::NoRecords => return Err("No captain records.".into()),
        }
        self.require_active_captain(target.captain)?;
        self.crew_control_capability(&target.server)?;
        let draft = self.current();
        let scope = self.captain_scope(target.captain);
        if scope.is_empty() {
            if draft.project.is_some() {
                return Err(format!(
                    "{} has no workspaces, so its crew stays project-less. Clear the workspace or remove the chip.",
                    target.name
                ));
            }
        } else if !draft
            .project
            .is_some_and(|project| scope.contains(&project))
        {
            let workspace = draft
                .project
                .and_then(|id| self.project(Some(id)).map(|p| p.name.clone()))
                .unwrap_or_else(|| "This workspace".into());
            return Err(format!(
                "{workspace} is outside {}'s workspaces. Pick one of its workspaces, or remove the chip.",
                target.name
            ));
        }
        if !self.crew_draft_placement()
            && (draft.worktree_path.is_some()
                || draft.branch.is_some()
                || self.effective_env_mode(draft) == ThreadEnvMode::Worktree)
        {
            return Err(
                "Crew creation cannot start a new worktree. Switch back to the local checkout or remove the chip."
                    .into(),
            );
        }
        if !draft.draft_model_state.additional_models.is_empty() {
            return Err(
                "Crew creation sends one model. Remove the extra models or the crew chip.".into(),
            );
        }
        if draft.draft_images.iter().any(Attachment::is_image)
            && draft.supports_images != Some(true)
        {
            let model = if draft.model.is_empty() {
                "The selected model"
            } else {
                &draft.model
            };
            return Err(if draft.supports_images == Some(false) {
                format!(
                    "{model} does not accept image input. Remove the images or choose a model that supports images."
                )
            } else {
                format!(
                    "{model} does not declare image input. Remove the images or choose a model that supports images."
                )
            });
        }
        Ok(())
    }

    /// Open an ordinary client draft on the Captain's machine with the crew
    /// binding and the Captain's first scoped workspace preset. `expected_server`
    /// is the machine the control rendered for; a stale click from another
    /// selection creates nothing.
    pub fn begin_crew_draft(
        &mut self,
        expected_server: &str,
        captain: usize,
    ) -> Result<(), String> {
        if let Some(reason) = self.crew_creation_unavailable(expected_server, captain) {
            return Err(reason);
        }
        let scope = self.captain_scope(captain);
        let project = scope
            .first()
            .copied()
            .filter(|id| self.project(Some(*id)).is_some());
        let name = self.captains[captain].name.clone();
        self.new_client_draft(project);
        if !self.current().client_draft {
            return Err("Could not open a draft on this machine.".into());
        }
        let thread = self.current_mut();
        // A backend without placement-v1 never creates a crew thread inside
        // a new worktree; a worktree choice made later blocks send there.
        thread.env_mode = Some(ThreadEnvMode::Local);
        thread.base_branch = None;
        thread.start_from_origin = None;
        thread.worktree_path = None;
        thread.branch = None;
        thread.crew_draft = Some(CrewDraftTarget {
            server: expected_server.to_owned(),
            captain,
            name,
        });
        thread.crew_draft_notice = None;
        // With placement-v1 a crew worker writes in its own worktree unless
        // the draft picks the read-only checkout.
        if self.crew_draft_placement() {
            self.set_draft_env_mode(ThreadEnvMode::Worktree);
        }
        Ok(())
    }

    /// Whether the open crew draft's machine takes an explicit placement
    /// (`placement-v1`). Without a client this desktop is the server.
    pub fn crew_draft_placement(&self) -> bool {
        self.current().crew_draft.as_ref().is_some_and(|target| {
            self.client.as_ref().is_none_or(|client| {
                client.supports_capability(&target.server, crate::protocol::PLACEMENT)
            })
        })
    }

    /// The `placement` argument for the draft's `captain.crew.create`, read
    /// from the composer's workspace controls: a new worktree on the chosen
    /// base, the selected existing worktree, or the project's checkout read
    /// only. Without workspace controls the backend's default applies.
    fn crew_draft_placement_args(&self) -> Option<Value> {
        if !self.crew_draft_placement() {
            return None;
        }
        match self.composer_vcs_controls() {
            ComposerVcsControls::Hidden => None,
            ComposerVcsControls::LockedWorktree { worktree_path, .. } => {
                Some(json!({"mode":"existing","worktree":worktree_path}))
            }
            ComposerVcsControls::Editable {
                mode: ThreadEnvMode::Local,
                ..
            } => Some(json!({"mode":"shared"})),
            ComposerVcsControls::Editable {
                base_branch,
                start_from_origin,
                ..
            } => Some(match base_branch {
                Some(base) => {
                    json!({"mode":"isolated","baseBranch":base,"startFromOrigin":start_from_origin})
                }
                None => json!({"mode":"isolated"}),
            }),
        }
    }

    /// Send the bound draft through `captain.crew.create`. Called from
    /// `dispatch_send_full` once it knows the current draft carries a binding,
    /// after the shared send guards. Returns `true` only when the command was
    /// handed to the owning machine; every refusal or failure keeps the
    /// composer content and returns `false`.
    pub fn send_crew_draft(&mut self) -> bool {
        let Some(target) = self.current().crew_draft.clone() else {
            return false;
        };
        if !self.current().has_draft_content()
            || self.current().remote_send.is_some()
            || self.current().client_creating.is_some()
        {
            return false;
        }
        if let Some(error) = self.crew_draft_unavailable() {
            self.current_mut().submission_error = Some(error);
            return false;
        }
        let target_server_id = self.machine_identity(&target.server);
        let prompt = match crate::composer_tokens::prompt_for_target_with_reply(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
            self.current().draft_reply_to.as_ref(),
            Some(&target.server),
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
        self.current_mut().submission_error = None;
        // The backend's target contract accepts harness, model and
        // options.reasoning only. Placement keys (machine, environmentId,
        // projectId) are refused there, so they are never written here; the
        // crew thread always lands in the Captain's local scope workspace.
        let mut target_params = json!({"harness":self.current().harness});
        if let Some((_, model)) = &self.current().selected_model {
            target_params["model"] = json!(model);
        }
        if let Some(reasoning) = &self.current().selected_thinking {
            target_params["options"] = json!({"reasoning":reasoning});
        }
        let mut params = json!({
            "captain":target.captain,
            "project":self.current().project,
            "access":self.current().native.access.id(),
            "target":target_params,
            "input":{
                "text":prompt.text,
                "attachments":prompt.images.as_ref(),
                "context":prompt.context,
                "draft":prompt.draft,
            },
        });
        if let Some(placement) = self.crew_draft_placement_args() {
            params["placement"] = placement;
        }
        let result = match self.remote("captain.crew.create", params.clone()) {
            Some(result) => result,
            None => return self.apply_local_crew_create(&target, &params),
        };
        match result {
            Ok(command) => {
                // Same retain/clear lifecycle as promote_client_draft: the
                // snapshot owns the submitted input until its receipt.
                let snapshot = crate::composer_rich::Snapshot::capture_with_context(
                    &self.current().draft,
                    &self.current().draft_images,
                    &self.current().draft_context,
                );
                let thread = self.current_mut();
                thread.client_creating = Some(command);
                thread.remote_send = Some((command, snapshot));
                thread.remote_send_reply_to = thread.draft_reply_to.take();
                thread.life.last_requested_at = Some(lifecycle::now());
                thread.draft.replace_text("");
                thread.draft_images.clear();
                thread.draft_context.clear();
                thread.retained_context = Default::default();
                thread.typing_guard.on_blur_or_send();
                self.usage_limits_notice_key = None;
                self.composer_notices.remove("usage-limits");
                true
            }
            Err(error) => {
                self.storage_error = Some(error);
                false
            }
        }
    }

    /// No client link: run the same atomic create through the local backend.
    /// The accepted member replaces the device draft, mirroring promotion.
    fn apply_local_crew_create(&mut self, target: &CrewDraftTarget, args: &Value) -> bool {
        match self.captain_crew_create(target.captain, args, false, uuid::Uuid::new_v4()) {
            Ok(value) => {
                let Some(id) = value["threadId"]
                    .as_u64()
                    .and_then(|id| usize::try_from(id).ok())
                else {
                    return false;
                };
                let draft_id = self.current().id;
                if self
                    .threads
                    .iter()
                    .position(|thread| thread.id == draft_id && thread.client_draft)
                    .is_some_and(|index| {
                        self.threads.remove(index);
                        true
                    })
                {
                    for tab in &mut self.tabs {
                        if *tab == draft_id {
                            *tab = id;
                        }
                    }
                }
                if let Some(index) = self.threads.iter().position(|thread| thread.id == id) {
                    self.select_thread(index);
                }
                self.checkpoint();
                true
            }
            Err(error) => {
                self.current_mut().error = Some(error);
                false
            }
        }
    }

    /// Match a `captain.crew.create` receipt to the draft that sent it, by the
    /// receipt's server, the exact command UUID and the bound Captain. Only an
    /// accepted receipt carrying the new `threadId` promotes the draft to the
    /// canonical thread; a matching projection alone never settles the send.
    /// A rejection restores the full original rich input and keeps the chip.
    pub(crate) fn crew_draft_receipt(
        &mut self,
        server: &str,
        command: &Command,
        value: &Value,
    ) -> bool {
        if command.method != "captain.crew.create" {
            return false;
        }
        enum Outcome {
            Handled,
            Failed(Option<String>, usize, String),
        }
        let outcome = {
            let threads = if server == self.machines.selected {
                Some(&mut self.threads)
            } else {
                self.machines.threads.get_mut(server)
            };
            let Some(threads) = threads else {
                return false;
            };
            let Some(draft) = threads.iter_mut().find(|thread| {
                thread.client_draft
                    && thread.client_creating == Some(command.command_id)
                    && thread.crew_draft.as_ref().is_some_and(|target| {
                        target.server == server
                            && command.params["captain"].as_u64() == Some(target.captain as u64)
                    })
            }) else {
                return false;
            };
            let accepted = value["result"]["status"] == "accepted";
            let created = value["result"]["result"]["threadId"]
                .as_u64()
                .and_then(|id| usize::try_from(id).ok());
            if accepted && created.is_none() {
                // Acceptance may already have created a root and started its input.
                // Keep the original command held instead of offering a fresh create.
                draft.error = Some("The server accepted crew creation without a thread identity. Check the Captain's crew before sending more work. This draft is held to prevent a duplicate.".into());
                return true;
            }
            if accepted && let Some(id) = created {
                // Promotion follows the server projection; no second send is
                // issued, because membership and the first input already
                // committed together on the owning machine.
                draft.client_creating = None;
                draft.remote_send = None;
                draft.remote_send_reply_to = None;
                draft.crew_draft = None;
                draft.client_promoted = Some(id);
                Outcome::Handled
            } else {
                draft.client_creating = None;
                if let Some((_, snapshot)) = draft.remote_send.take() {
                    draft.restore_snapshot(&snapshot);
                }
                let reply_to = draft.remote_send_reply_to.take();
                if draft.draft_reply_to.is_none() {
                    draft.draft_reply_to = reply_to;
                }
                let error = value["error"]["message"]
                    .as_str()
                    .or(value["result"]["result"]["error"].as_str())
                    .unwrap_or(if accepted {
                        "The server accepted the crew command but named no thread."
                    } else {
                        "Crew creation was not confirmed. Your draft was kept."
                    })
                    .to_string();
                draft.error = Some(error.clone());
                Outcome::Failed(draft.background_send.take(), draft.id, error)
            }
        };
        if let Outcome::Failed(background, draft_id, error) = outcome {
            if let Some(server) = background {
                self.notify_background_failure(&server, draft_id, error);
            } else {
                self.machines.notice = Some(error);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A desktop client linked to its local server, with one Captain projected.
    fn crew_client() -> (State, tokio::sync::mpsc::Receiver<Command>) {
        let mut state = State::new(PathBuf::new(), false);
        let (client, commands) = crate::client::Client::test_connected(state.data_dir.clone());
        *client.info.lock().expect("info") = json!({"capabilities":[
            crate::protocol::CAPTAIN_STATE,
            crate::protocol::CAPTAIN_CREW_CONTROL,
        ]});
        state.client = Some(client);
        state.captains = crate::captains::Captain::presets();
        (state, commands)
    }

    fn receipt(accepted: bool, thread: usize) -> Value {
        if accepted {
            json!({"result":{"status":"accepted","result":{"saved":true,"threadId":thread}}})
        } else {
            json!({"result":{"status":"rejected","result":{"error":"No crew for you."}}})
        }
    }

    #[test]
    fn begin_crew_draft_binds_the_open_draft_to_the_captains_machine() {
        let (mut state, _commands) = crew_client();
        assert!(state.begin_crew_draft("studio", 0).is_err());
        state.begin_crew_draft("local", 0).unwrap();
        let draft = state.current();
        assert!(draft.client_draft);
        assert_eq!(draft.env_mode, Some(ThreadEnvMode::Local));
        let target = draft.crew_draft.clone().unwrap();
        assert_eq!(target.server, "local");
        assert_eq!(target.captain, 0);
        assert!(!target.name.is_empty());
        assert!(state.crew_draft_unavailable().is_none());
    }

    #[test]
    fn begin_crew_draft_refuses_a_machine_without_crew_control() {
        let mut state = State::new(PathBuf::new(), false);
        let (client, _commands) = crate::client::Client::test_connected(state.data_dir.clone());
        *client.info.lock().expect("info") =
            json!({"capabilities":[crate::protocol::CAPTAIN_STATE]});
        state.client = Some(client);
        state.captains = crate::captains::Captain::presets();
        let error = state.begin_crew_draft("local", 0).unwrap_err();
        assert!(error.contains("Update required"), "{error}");
        assert!(state.current().crew_draft.is_none());
    }

    #[test]
    fn changing_workspace_keeps_the_binding_and_never_reuses_it_for_an_ordinary_draft() {
        let (mut state, _commands) = crew_client();
        state.begin_crew_draft("local", 0).unwrap();
        let draft_id = state.current().id;
        state
            .current_mut()
            .draft
            .replace_text("Keep my first input");
        state.choose_draft_project(Some(1));
        assert_eq!(state.current().id, draft_id);
        assert!(state.current().crew_draft.is_some());
        assert_eq!(state.current().draft.text, "Keep my first input");
        assert!(state.crew_draft_unavailable().is_some());
        state.current_mut().draft.replace_text("");
        state.choose_draft_project(None);
        state.new_thread_in(None);
        assert_ne!(state.current().id, draft_id);
        assert!(state.current().crew_draft.is_none());
    }

    #[test]
    fn a_draft_outside_scope_worktree_or_multi_model_cannot_send() {
        let (mut state, _commands) = crew_client();
        state.captain_scopes.insert(0, vec![1]);
        state.begin_crew_draft("local", 0).unwrap();
        state.current_mut().project = Some(7);
        let reason = state.crew_draft_unavailable().unwrap();
        assert!(reason.contains("outside"), "{reason}");
        // Scope blocks without clearing text or the binding.
        state.current_mut().draft.replace_text("still here");
        assert!(!state.send_crew_draft());
        assert_eq!(state.current().draft.text, "still here");
        assert!(state.current().crew_draft.is_some());
        state.current_mut().project = Some(1);
        assert!(state.crew_draft_unavailable().is_none());
        state.current_mut().env_mode = Some(ThreadEnvMode::Worktree);
        assert!(state.crew_draft_unavailable().is_some());
        state.current_mut().env_mode = Some(ThreadEnvMode::Local);
        state
            .current_mut()
            .draft_model_state
            .additional_models
            .push(crate::model_selection::ModelSelection {
                instance_id: "pi".into(),
                model: "fixture".into(),
                options: vec![],
            });
        assert!(state.crew_draft_unavailable().is_some());
    }

    #[test]
    fn a_placement_machine_sends_the_drafts_workspace_choice_as_its_placement() {
        // A real project, so the draft has workspace controls.
        let mut state = State::new(".".into(), false);
        let project = state.current().project.unwrap();
        let (client, mut commands) = crate::client::Client::test_connected(state.data_dir.clone());
        state.client = Some(client);
        state.captains = crate::captains::Captain::presets();
        *state.client.as_ref().unwrap().info.lock().expect("info") = json!({"capabilities":[
            crate::protocol::CAPTAIN_STATE,
            crate::protocol::CAPTAIN_CREW_CONTROL,
            crate::protocol::VCS_WORKTREES,
            crate::protocol::PLACEMENT,
        ]});
        state.captain_scopes.insert(0, vec![project]);
        let path = state
            .project(Some(project))
            .unwrap()
            .path
            .to_string_lossy()
            .into_owned();
        state.vcs_branches.insert(
            ("local".into(), path),
            VcsBranchCacheEntry {
                is_repo: true,
                loaded: true,
                loading: false,
                error: None,
                refs: vec![crate::vcs::VcsRef {
                    name: "main".into(),
                    is_remote: Some(false),
                    remote_name: None,
                    upstream_ref: None,
                    current: true,
                    is_default: true,
                    worktree_path: None,
                }],
            },
        );
        state.begin_crew_draft("local", 0).unwrap();
        // A writer defaults to its own worktree on the default branch, and that sends.
        assert_eq!(state.current().env_mode, Some(ThreadEnvMode::Worktree));
        assert_eq!(state.current().base_branch.as_deref(), Some("main"));
        assert!(state.crew_draft_unavailable().is_none());
        state.current_mut().draft.replace_text("Fix the parser");
        assert!(state.send_crew_draft());
        let command = commands.try_recv().unwrap();
        assert_eq!(command.params["placement"]["mode"], "isolated");
        assert_eq!(command.params["placement"]["baseBranch"], "main");
        assert!(state.crew_draft_receipt("local", &command, &receipt(false, 0)));
        // The checkout choice is the read-only shared placement, never a writable one.
        state.set_draft_env_mode(ThreadEnvMode::Local);
        assert!(state.send_crew_draft());
        let command = commands.try_recv().unwrap();
        assert_eq!(command.params["placement"], json!({"mode":"shared"}));
    }

    #[test]
    fn a_crew_draft_sends_one_atomic_create_and_promotes_on_its_receipt() {
        let (mut state, mut commands) = crew_client();
        state.begin_crew_draft("local", 0).unwrap();
        state
            .current_mut()
            .draft
            .replace_text("Investigate the parser");
        assert!(state.send_crew_draft());
        let command = commands.try_recv().unwrap();
        assert_eq!(command.method, "captain.crew.create");
        assert_eq!(command.params["captain"], json!(0));
        assert_eq!(command.params["input"]["text"], "Investigate the parser");
        assert!(command.params.get("threadId").is_none());
        assert_eq!(state.current().client_creating, Some(command.command_id));
        // No double-send while the create is in flight.
        assert!(!state.send_crew_draft());
        assert!(commands.try_recv().is_err());
        // A receipt for another command or another captain cannot settle it.
        let other = Command::new("captain.crew.create", json!({"captain":9}));
        assert!(!state.crew_draft_receipt("local", &other, &receipt(true, 40)));
        let wrong_server = command.clone();
        assert!(!state.crew_draft_receipt("studio", &wrong_server, &receipt(true, 40)));
        assert_eq!(state.current().client_creating, Some(command.command_id));
        assert!(state.crew_draft_receipt("local", &command, &receipt(true, 40)));
        let draft = state.current();
        assert_eq!(draft.client_promoted, Some(40));
        assert!(draft.crew_draft.is_none());
        assert!(draft.client_creating.is_none());
        assert!(draft.remote_send.is_none());
    }

    #[test]
    fn a_rejected_create_restores_the_rich_input_and_keeps_the_chip() {
        let (mut state, mut commands) = crew_client();
        state.begin_crew_draft("local", 0).unwrap();
        state.current_mut().draft.replace_text("First work item");
        let image = Attachment {
            asset_id: None,
            uploaded: None,
            source: None,
            id: "draft.png".into(),
            name: "draft.png".into(),
            mime: "application/octet-stream".into(),
            width: 0,
            height: 0,
            bytes: 4,
            path: None,
        };
        state.current_mut().draft_images.push(image);
        state.current_mut().supports_images = Some(true);
        assert!(state.send_crew_draft());
        let command = commands.try_recv().unwrap();
        assert!(state.crew_draft_receipt("local", &command, &receipt(false, 0)));
        let draft = state.current();
        assert!(draft.client_creating.is_none());
        assert!(draft.crew_draft.is_some(), "the chip survives a rejection");
        assert_eq!(draft.draft.text, "First work item");
        assert_eq!(draft.draft_images.len(), 1);
        assert_eq!(draft.error.as_deref(), Some("No crew for you."));
        // The same draft can try again; it issues a fresh command id.
        assert!(state.send_crew_draft());
        let retry = commands.try_recv().unwrap();
        assert_ne!(retry.command_id, command.command_id);
    }

    #[test]
    fn accepted_creation_without_an_identity_never_enables_a_second_create() {
        let (mut state, mut commands) = crew_client();
        state.begin_crew_draft("local", 0).unwrap();
        state.current_mut().draft.replace_text("Do this once");
        assert!(state.send_crew_draft());
        let command = commands.try_recv().unwrap();
        assert!(state.crew_draft_receipt(
            "local",
            &command,
            &json!({
                "result":{"status":"accepted","result":{"saved":true}}
            })
        ));
        assert_eq!(state.current().client_creating, Some(command.command_id));
        assert!(state.current().remote_send.is_some());
        state.current_mut().draft.replace_text("Do this once");
        assert!(!state.send_crew_draft());
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn switching_machine_unbinds_the_chip_with_an_explanatory_notice() {
        let (mut state, _commands) = crew_client();
        state.begin_crew_draft("local", 0).unwrap();
        state.current_mut().draft.replace_text("keep me");
        // The draft landed on another machine's list, as retargeting does.
        state.current_mut().crew_draft.as_mut().unwrap().server = "studio".into();
        state.crew_draft_strip_foreign();
        let draft = state.current();
        assert!(draft.crew_draft.is_none());
        let notice = draft.crew_draft_notice.clone().unwrap();
        assert!(notice.contains("Removed from"), "{notice}");
        assert!(notice.contains("crew"), "{notice}");
        assert_eq!(draft.draft.text, "keep me");
    }
}
