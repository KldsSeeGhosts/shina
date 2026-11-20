//! Client-side Captain lifecycle state: the shared confirmation dialog and the
//! command seam that sends archive, unarchive, demote and delete through the
//! durable runtime's command queue. Mutations never run outside a runtime
//! transaction, so a clientless State refuses them instead of applying.
use super::*;

pub(crate) const LIFECYCLE_PREVIEW: &str = "captain.lifecycle.preview";
pub(crate) const LIFECYCLE_CONFLICT_PREFIX: &str = "Captain changed since version";

/// The message every rejected lifecycle receipt surfaces, wherever it landed.
fn lifecycle_receipt_error(value: &Value) -> String {
    value["error"]["message"]
        .as_str()
        .or_else(|| value["result"]["result"]["error"].as_str())
        .unwrap_or("The captain change was rejected.")
        .to_string()
}

impl State {
    /// Queue a command on `server`'s channel. Local goes through the same queue
    /// as linked machines, so every mutation lands inside a runtime batch.
    fn lifecycle_send(
        &self,
        server: &str,
        method: &str,
        params: Value,
    ) -> Result<uuid::Uuid, String> {
        let Some(client) = &self.client else {
            return Err("This Captain change requires the durable server.".into());
        };
        client
            .commands_for(server)
            .ok_or_else(|| "This machine is not linked.".to_string())?
            .send(method, params)
    }

    /// The negotiated-capability check for one action on one machine. A
    /// clientless local State has no peer to negotiate with and passes; the
    /// send still refuses below.
    fn lifecycle_capability(&self, server: &str, capability: &str) -> bool {
        let Some(client) = &self.client else {
            return true;
        };
        if server == "local" {
            client.has_capability(capability)
        } else {
            client
                .linked
                .get(server)
                .is_some_and(|link| link.has_capability(capability))
        }
    }

    /// Why a lifecycle control is unavailable right now, or `None` when it may
    /// send. Checked both to disable the entry points and again at press time.
    /// The phase check decides which actions a Captain's record allows; the
    /// earlier-home check then blocks every Active-captain mutation while an
    /// earlier conversation is open. Archived and retired records keep their
    /// appropriate actions (unarchive, delete) even though `captain_access`
    /// reports them as `Earlier`.
    pub fn captain_lifecycle_unavailable(
        &self,
        expected_server: &str,
        captain: usize,
        action: CaptainLifecycleAction,
    ) -> Option<String> {
        if expected_server != self.machines.selected {
            return Some("That action belonged to another machine.".into());
        }
        let phase = self.captain_phase(captain);
        if captain >= self.captains.len() || phase == CaptainPhase::Deleted {
            return Some("No captain records.".into());
        }
        let server = self.machines.selected.clone();
        let label = self.machine_label(&server);
        if self.client.is_some() && !self.machine_connected(&server) {
            return Some(format!("{label} is unreachable."));
        }
        if !self.lifecycle_capability(&server, action.capability()) {
            return Some(format!(
                "Update Shina on {label} to {} captains.",
                action.action()
            ));
        }
        if self
            .lifecycle_pending
            .iter()
            .any(|((host, _), change)| *host == server && change.captain == captain)
        {
            return Some(format!("Waiting for {label} to confirm."));
        }
        let allowed = match action {
            CaptainLifecycleAction::Archive | CaptainLifecycleAction::Demote => {
                phase == CaptainPhase::Active
            }
            CaptainLifecycleAction::Unarchive => phase == CaptainPhase::Archived,
            CaptainLifecycleAction::Delete => matches!(
                phase,
                CaptainPhase::Active | CaptainPhase::Archived | CaptainPhase::Retired
            ),
        };
        if !allowed {
            return Some(
                match phase {
                    CaptainPhase::Archived => "This captain is archived.",
                    CaptainPhase::Retired => "This captain is already an ordinary thread.",
                    _ => "The Captain is not in the required lifecycle state.",
                }
                .into(),
            );
        }
        if phase == CaptainPhase::Active && self.captain_access(captain) == CaptainAccess::Earlier {
            return Some("Read only: an earlier conversation is open.".into());
        }
        None
    }

    /// Open the shared confirmation for `action` on `captain`, then immediately
    /// ask the owning machine for its authoritative preview. The dialog is
    /// bound to `expected_server` for its whole life; callbacks rendered for
    /// another machine open nothing.
    pub fn open_captain_lifecycle(
        &mut self,
        expected_server: &str,
        captain: usize,
        action: CaptainLifecycleAction,
    ) -> Result<(), String> {
        if let Some(reason) = self.captain_lifecycle_unavailable(expected_server, captain, action) {
            return Err(reason);
        }
        self.brief.lifecycle = Some(CaptainLifecycleDialog {
            server: self.machines.selected.clone(),
            captain,
            action,
            preview: None,
            pending: None,
            error: None,
            confirm_name: TextEdit::default(),
            keep_conversations: true,
        });
        self.refresh_captain_lifecycle_preview()
    }

    /// Re-read the preview for the open dialog. Remote machines answer through
    /// a `captain.lifecycle.preview` read command matched by exact receipt;
    /// a clientless local State reads its own records, which is safe because a
    /// preview is a `&self` query, not a mutation. Typed fields are kept.
    pub fn refresh_captain_lifecycle_preview(&mut self) -> Result<(), String> {
        let Some(dialog) = self.brief.lifecycle.as_mut() else {
            return Err("No captain action is open.".into());
        };
        if dialog.pending.is_some() {
            return Err("Waiting for the machine to answer.".into());
        }
        let server = dialog.server.clone();
        let captain = dialog.captain;
        let action = dialog.action;
        // A conflict notice survives the refresh it triggers; only a fresh
        // explicit confirmation clears it. The stale preview is dropped so a
        // confirm can never send its old version.
        if !dialog.conflict() {
            dialog.error = None;
        }
        dialog.preview = None;
        if self.client.is_none() && server == "local" {
            let preview = self.captain_lifecycle_preview(captain, action.action())?;
            if let Some(dialog) = self.brief.lifecycle.as_mut() {
                dialog.preview = Some(preview);
            }
            return Ok(());
        }
        let id = self.lifecycle_send(
            &server,
            LIFECYCLE_PREVIEW,
            json!({"captain":captain,"action":action.action()}),
        )?;
        if let Some(dialog) = self.brief.lifecycle.as_mut() {
            dialog.pending = Some(id);
        }
        Ok(())
    }

    /// Send the mutation with the preview's exact `captainVersion` and
    /// `crewSeen`. Every confirm mints a fresh UUID; a dialog with a command in
    /// flight refuses, so a double press never submits twice. A clientless
    /// State refuses rather than mutating outside a runtime transaction.
    pub fn confirm_captain_lifecycle(&mut self) -> Result<CrewMutation, String> {
        let Some(dialog) = self.brief.lifecycle.as_ref() else {
            return Err("No captain action is open.".into());
        };
        if dialog.pending.is_some() {
            return Err("This change is already waiting for a reply.".into());
        }
        let server = dialog.server.clone();
        let captain = dialog.captain;
        let action = dialog.action;
        if let Some(reason) = self.captain_lifecycle_unavailable(&server, captain, action) {
            return Err(reason);
        }
        let Some(preview) = dialog.preview.clone() else {
            return Err("Wait for the preview before confirming.".into());
        };
        if let Some(blocker) = preview["blockers"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(Value::as_str)
        {
            return Err(blocker.into());
        }
        let mut params = json!({
            "captain":captain,
            "expectedVersion":preview["captainVersion"],
            "crewSeen":preview["crewSeen"],
        });
        if action == CaptainLifecycleAction::Delete {
            let name = self
                .captains
                .get(captain)
                .map(|captain| captain.name.clone())
                .unwrap_or_default();
            if dialog.confirm_name.text.trim() != name {
                return Err(format!("Type {name} to confirm."));
            }
            params["confirmName"] = json!(name);
            params["keepConversations"] = json!(dialog.keep_conversations);
        }
        let id = self.lifecycle_send(&server, action.method(), params)?;
        self.lifecycle_pending
            .insert((server, id), LifecycleChange { captain, action });
        if let Some(dialog) = self.brief.lifecycle.as_mut() {
            dialog.pending = Some(id);
            dialog.error = None;
        }
        Ok(CrewMutation::Pending(id))
    }

    /// Match a receipt to the open dialog's in-flight command or a pending
    /// mutation by exact server, UUID and method. Returns `true` only when all
    /// three match, so receipts from other machines, other commands or other
    /// methods fall through to the generic hooks untouched.
    pub(crate) fn captain_lifecycle_receipt(
        &mut self,
        server: &str,
        command: &crate::protocol::Command,
        value: &Value,
    ) -> bool {
        let key = (server.to_string(), command.command_id);
        if let Some(dialog) =
            self.brief.lifecycle.as_ref().filter(|dialog| {
                dialog.server == server && dialog.pending == Some(command.command_id)
            })
        {
            let action = dialog.action;
            let mutation = self.lifecycle_pending.contains_key(&key);
            if command.method == LIFECYCLE_PREVIEW && !mutation {
                self.settle_lifecycle_preview(value);
                return true;
            }
            // A mutation receipt settles only a mutation that was actually sent:
            // the pending record is written at confirm time.
            if command.method == action.method() && mutation {
                self.settle_lifecycle_mutation(server, command, value);
                return true;
            }
            return false;
        }
        if let Some(change) = self.lifecycle_pending.get(&key).copied()
            && command.method == change.action.method()
        {
            self.settle_lifecycle_mutation(server, command, value);
            return true;
        }
        false
    }

    fn settle_lifecycle_preview(&mut self, value: &Value) {
        let Some(dialog) = self.brief.lifecycle.as_mut() else {
            return;
        };
        dialog.pending = None;
        if value["result"]["status"] == "accepted" {
            dialog.preview = Some(value["result"]["result"].clone());
            if !dialog.conflict() {
                dialog.error = None;
            }
        } else {
            dialog.error = Some(lifecycle_receipt_error(value));
        }
    }

    /// A mutation's own receipt is the only thing that settles it; projection
    /// changes never close the dialog or clear the pending record.
    fn settle_lifecycle_mutation(
        &mut self,
        server: &str,
        command: &crate::protocol::Command,
        value: &Value,
    ) {
        let key = (server.to_string(), command.command_id);
        let change = self.lifecycle_pending.remove(&key).or_else(|| {
            self.brief
                .lifecycle
                .as_ref()
                .filter(|dialog| {
                    dialog.server == server && dialog.pending == Some(command.command_id)
                })
                .map(|dialog| LifecycleChange {
                    captain: dialog.captain,
                    action: dialog.action,
                })
        });
        let Some(change) = change else {
            return;
        };
        let name = self
            .captains
            .get(change.captain)
            .map(|captain| captain.name.clone())
            .unwrap_or_else(|| "The captain".into());
        if value["result"]["status"] == "accepted" {
            let dialog_hit = self.brief.lifecycle.as_ref().is_some_and(|dialog| {
                dialog.server == server && dialog.pending == Some(command.command_id)
            });
            if dialog_hit {
                self.brief.lifecycle = None;
            }
            if matches!(
                change.action,
                CaptainLifecycleAction::Demote | CaptainLifecycleAction::Delete
            ) {
                // Only a receipt for the selected machine may close Brief UI,
                // and only when it names the captain that Brief is showing.
                // A late receipt from another server toasts but leaves the
                // current Brief alone.
                let owns_brief = server == self.machines.selected
                    && (dialog_hit
                        || (self.current().captain_home
                            && self.current().captain == Some(change.captain))
                        || self.brief.record.as_ref()
                            == Some(&(server.to_string(), change.captain)));
                if owns_brief {
                    self.show_activity = false;
                    self.activity_full = false;
                    self.brief.confirm_reset = false;
                }
            }
            let title = match change.action {
                CaptainLifecycleAction::Archive => format!("Archived {name}."),
                CaptainLifecycleAction::Unarchive => format!("Unarchived {name}."),
                CaptainLifecycleAction::Demote => {
                    format!("{name} is now an ordinary thread.")
                }
                CaptainLifecycleAction::Delete => format!("Deleted {name}."),
            };
            self.push_toast(ToastKind::Success, title, None, None, Some(6000));
            return;
        }
        let message = lifecycle_receipt_error(value);
        let conflict = message.starts_with(LIFECYCLE_CONFLICT_PREFIX);
        let on_dialog = self
            .brief
            .lifecycle
            .as_ref()
            .is_some_and(|dialog| dialog.server == server && dialog.captain == change.captain);
        if on_dialog {
            if let Some(dialog) = self.brief.lifecycle.as_mut() {
                dialog.pending = None;
                dialog.error = Some(message);
            }
            // A stale version re-reads the Captain for a fresh explicit
            // confirmation. The typed name and switch stay; nothing re-sends
            // the old UUID or confirms on its own.
            if conflict {
                let _ = self.refresh_captain_lifecycle_preview();
            }
        } else {
            let title = match change.action {
                CaptainLifecycleAction::Archive => format!("Could not archive {name}"),
                CaptainLifecycleAction::Unarchive => format!("Could not unarchive {name}"),
                CaptainLifecycleAction::Demote => format!("Could not demote {name}"),
                CaptainLifecycleAction::Delete => format!("Could not delete {name}"),
            };
            self.push_toast(ToastKind::Error, title, Some(message), None, Some(6000));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CaptainLifecycleAction as Action;

    fn fixture() -> State {
        let mut state = State::new(
            std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()),
            false,
        );
        state.load_global_for_test();
        state.captains = crate::captains::Captain::presets();
        state.open_captain(0);
        state
    }

    fn command(id: uuid::Uuid, method: &str) -> crate::protocol::Command {
        crate::protocol::Command {
            command_id: id,
            method: method.into(),
            params: json!({}),
        }
    }

    fn accepted(result: Value) -> Value {
        json!({"result":{"status":"accepted","result":result}})
    }

    fn rejected(message: &str) -> Value {
        json!({"result":{"status":"rejected","result":{"error":message}}})
    }

    fn linked(
        state: &mut State,
        capabilities: &[&str],
    ) -> tokio::sync::mpsc::Receiver<crate::protocol::Command> {
        let (mut client, _local) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, commands) = crate::client::Client::test_connected(state.data_dir.clone());
        *remote.info.lock().unwrap() = json!({"capabilities":capabilities});
        client.linked.insert("studio".into(), remote);
        state.client = Some(client);
        commands
    }

    fn open(state: &mut State, server: &str, action: Action) {
        state.open_captain_lifecycle(server, 0, action).unwrap();
    }

    #[test]
    fn receipts_settle_only_on_exact_server_uuid_and_method() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_ARCHIVE]);
        state.machines.selected = "studio".into();
        open(&mut state, "studio", Action::Archive);
        let preview_id = commands.try_recv().unwrap().command_id;
        let dialog = state.brief.lifecycle.as_ref().unwrap();
        assert_eq!(dialog.pending, Some(preview_id));
        // Wrong server, wrong UUID and wrong method are all ignored.
        let preview = accepted(json!({"captainVersion":"1","crewSeen":[],"blockers":[]}));
        assert!(!state.captain_lifecycle_receipt(
            "local",
            &command(preview_id, LIFECYCLE_PREVIEW),
            &preview
        ));
        assert!(!state.captain_lifecycle_receipt(
            "studio",
            &command(uuid::Uuid::new_v4(), LIFECYCLE_PREVIEW),
            &preview
        ));
        assert!(!state.captain_lifecycle_receipt(
            "studio",
            &command(preview_id, "captain.archive"),
            &preview
        ));
        assert!(state.brief.lifecycle.as_ref().unwrap().preview.is_none());
        // The exact preview receipt fills the dialog.
        assert!(state.captain_lifecycle_receipt(
            "studio",
            &command(preview_id, LIFECYCLE_PREVIEW),
            &preview
        ));
        assert!(state.brief.lifecycle.as_ref().unwrap().pending.is_none());
        assert_eq!(
            state
                .brief
                .lifecycle
                .as_ref()
                .unwrap()
                .preview
                .as_ref()
                .unwrap()["captainVersion"],
            "1"
        );
        // Confirm sends the mutation with the preview's version, then ignores
        // a preview-method receipt carrying the same UUID.
        let mutation = state.confirm_captain_lifecycle().unwrap();
        let mutation_id = match mutation {
            CrewMutation::Pending(id) => id,
            _ => panic!("expected a pending mutation"),
        };
        let sent = commands.try_recv().unwrap();
        assert_eq!(sent.command_id, mutation_id);
        assert_eq!(sent.method, "captain.archive");
        assert_eq!(sent.params["expectedVersion"], "1");
        assert!(state.brief.lifecycle.as_ref().unwrap().preview.is_some());
        assert!(!state.captain_lifecycle_receipt(
            "studio",
            &command(mutation_id, LIFECYCLE_PREVIEW),
            &preview
        ));
        assert!(
            state
                .lifecycle_pending
                .contains_key(&("studio".into(), mutation_id))
        );
        // Rejection removes the pending record and reports on the dialog.
        assert!(state.captain_lifecycle_receipt(
            "studio",
            &command(mutation_id, "captain.archive"),
            &rejected("Stop 1 working thread first.")
        ));
        assert!(state.lifecycle_pending.is_empty());
        let dialog = state.brief.lifecycle.as_ref().unwrap();
        assert_eq!(
            dialog.error.as_deref(),
            Some("Stop 1 working thread first.")
        );
        assert!(dialog.pending.is_none());
        std::fs::remove_dir_all(&state.data_dir).ok();
    }

    #[test]
    fn a_stale_mutation_refreshes_and_needs_a_new_press_and_uuid() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_DELETE]);
        state.machines.selected = "studio".into();
        open(&mut state, "studio", Action::Delete);
        let preview_id = commands.try_recv().unwrap().command_id;
        let preview = accepted(
            json!({"captainVersion":"7","crewSeen":[],"blockers":[],"canDeleteConversations":true}),
        );
        assert!(state.captain_lifecycle_receipt(
            "studio",
            &command(preview_id, LIFECYCLE_PREVIEW),
            &preview
        ));
        {
            let dialog = state.brief.lifecycle.as_mut().unwrap();
            dialog.confirm_name.text = "Ember".into();
            dialog.keep_conversations = false;
        }
        let first = match state.confirm_captain_lifecycle().unwrap() {
            CrewMutation::Pending(id) => id,
            _ => panic!("expected a pending mutation"),
        };
        let sent = commands.try_recv().unwrap();
        assert_eq!(sent.method, "captain.delete");
        assert_eq!(sent.params["confirmName"], "Ember");
        assert_eq!(sent.params["keepConversations"], false);
        assert!(state.confirm_captain_lifecycle().is_err());
        // A stale-version rejection keeps the dialog, refreshes the preview and
        // keeps the typed name and switch. No new mutation is minted.
        assert!(state.captain_lifecycle_receipt(
            "studio",
            &command(first, "captain.delete"),
            &rejected("Captain changed since version 7. Review a fresh preview before confirming.")
        ));
        assert!(state.lifecycle_pending.is_empty());
        let refreshed = commands.try_recv().unwrap();
        assert_eq!(refreshed.method, LIFECYCLE_PREVIEW);
        assert!(commands.try_recv().is_err());
        let dialog = state.brief.lifecycle.as_ref().unwrap();
        assert!(dialog.conflict());
        assert_eq!(dialog.confirm_name.text, "Ember");
        assert!(!dialog.keep_conversations);
        assert_eq!(dialog.pending, Some(refreshed.command_id));
        assert!(dialog.preview.is_none());
        // The fresh preview lands but the review-again notice stays until the
        // user presses confirm again.
        assert!(state.captain_lifecycle_receipt(
            "studio",
            &command(refreshed.command_id, LIFECYCLE_PREVIEW),
            &accepted(json!({"captainVersion":"8","crewSeen":[],"blockers":[]}))
        ));
        assert!(state.brief.lifecycle.as_ref().unwrap().conflict());
        let second = match state.confirm_captain_lifecycle().unwrap() {
            CrewMutation::Pending(id) => id,
            _ => panic!("expected a pending mutation"),
        };
        assert_ne!(first, second);
        assert!(state.brief.lifecycle.as_ref().unwrap().error.is_none());
        assert_eq!(commands.try_recv().unwrap().params["expectedVersion"], "8");
        std::fs::remove_dir_all(&state.data_dir).ok();
    }

    #[test]
    fn a_wrong_machine_missing_capability_and_pending_all_send_nothing() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_ARCHIVE]);
        state.machines.selected = "studio".into();
        // A callback rendered for another machine queues nothing.
        assert!(
            state
                .open_captain_lifecycle("local", 0, Action::Archive)
                .is_err()
        );
        assert!(state.brief.lifecycle.is_none());
        assert!(commands.try_recv().is_err());
        // A machine without the capability reports the update reason.
        let mut no_caps = fixture();
        let mut bare = linked(&mut no_caps, &[]);
        no_caps.machines.selected = "studio".into();
        let reason = no_caps
            .captain_lifecycle_unavailable("studio", 0, Action::Archive)
            .unwrap();
        assert_eq!(reason, "Update Shina on studio to archive captains.");
        assert!(
            no_caps
                .open_captain_lifecycle("studio", 0, Action::Archive)
                .is_err()
        );
        assert!(bare.try_recv().is_err());
        // A pending command for the Captain gates every further action.
        state.lifecycle_pending.insert(
            ("studio".into(), uuid::Uuid::new_v4()),
            LifecycleChange {
                captain: 0,
                action: Action::Archive,
            },
        );
        assert_eq!(
            state
                .captain_lifecycle_unavailable("studio", 0, Action::Archive)
                .as_deref(),
            Some("Waiting for studio to confirm.")
        );
        std::fs::remove_dir_all(&state.data_dir).ok();
        std::fs::remove_dir_all(&no_caps.data_dir).ok();
    }

    #[test]
    fn unreachable_machine_is_reported_and_offline_sends_fail() {
        let mut state = fixture();
        let (mut client, _local) = crate::client::Client::test_connected(state.data_dir.clone());
        let remote = crate::client::Client::new(state.data_dir.clone());
        *remote.info.lock().unwrap() = json!({"capabilities":[crate::protocol::CAPTAIN_ARCHIVE]});
        client.linked.insert("studio".into(), remote);
        state.client = Some(client);
        state.machines.selected = "studio".into();
        // The linked client's queue was never marked ready, so it is unreachable.
        assert_eq!(
            state
                .captain_lifecycle_unavailable("studio", 0, Action::Archive)
                .as_deref(),
            Some("studio is unreachable.")
        );
        std::fs::remove_dir_all(&state.data_dir).ok();
    }

    #[test]
    fn delete_requires_the_exact_typed_name_before_sending() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_DELETE]);
        state.machines.selected = "studio".into();
        open(&mut state, "studio", Action::Delete);
        let preview_id = commands.try_recv().unwrap().command_id;
        assert!(state.captain_lifecycle_receipt(
            "studio",
            &command(preview_id, LIFECYCLE_PREVIEW),
            &accepted(json!({"captainVersion":"3","crewSeen":[],"blockers":[]}))
        ));
        let name = state.captains[0].name.clone();
        // Wrong case and partial names never reach the wire.
        state.brief.lifecycle.as_mut().unwrap().confirm_name.text = name.to_lowercase();
        assert!(state.confirm_captain_lifecycle().is_err());
        state.brief.lifecycle.as_mut().unwrap().confirm_name.text =
            name.chars().take(name.len() - 1).collect();
        assert!(state.confirm_captain_lifecycle().is_err());
        assert!(commands.try_recv().is_err());
        state.brief.lifecycle.as_mut().unwrap().confirm_name.text = format!(" {name} ");
        assert!(state.confirm_captain_lifecycle().is_ok());
        let sent = commands.try_recv().unwrap();
        assert_eq!(sent.method, "captain.delete");
        assert_eq!(sent.params["confirmName"], name);
        assert_eq!(sent.params["keepConversations"], true);
        std::fs::remove_dir_all(&state.data_dir).ok();
    }

    #[test]
    fn phase_and_earlier_home_gates() {
        let mut state = fixture();
        // An archived Captain offers unarchive and delete, never archive.
        let home = state.captain_home(0).unwrap();
        let home_archived_at = state
            .threads
            .iter()
            .find(|thread| thread.id == home)
            .and_then(|thread| thread.life.archived_at);
        state.captain_lifecycles.insert(
            0,
            super::captain_lifecycle::CaptainLifecycle {
                phase: CaptainPhase::Archived,
                archived_threads: [(home, home_archived_at)].into_iter().collect(),
                ..Default::default()
            },
        );
        assert_eq!(
            state
                .captain_lifecycle_unavailable("local", 0, Action::Archive)
                .as_deref(),
            Some("This captain is archived.")
        );
        assert!(
            state
                .captain_lifecycle_unavailable("local", 0, Action::Unarchive)
                .is_none()
        );
        assert!(
            state
                .captain_lifecycle_unavailable("local", 0, Action::Delete)
                .is_none()
        );
        state.captain_lifecycles.remove(&0);
        // An earlier home of an active Captain blocks every mutation.
        let earlier = {
            let id = state.next_id;
            state.next_id += 1;
            let mut thread = Thread::new(id);
            thread.captain_home = true;
            thread.captain = Some(0);
            state.threads.push(thread);
            id
        };
        let index = state.threads.iter().position(|t| t.id == earlier).unwrap();
        state.open_kept_tab(index);
        for action in [Action::Archive, Action::Demote, Action::Delete] {
            assert_eq!(
                state
                    .captain_lifecycle_unavailable("local", 0, action)
                    .as_deref(),
                Some("Read only: an earlier conversation is open.")
            );
        }
        // A retired record keeps Delete with no home at all: the record view
        // selected through the Archive is `brief.record`, not a conversation.
        state.captain_lifecycles.insert(
            0,
            super::captain_lifecycle::CaptainLifecycle {
                phase: CaptainPhase::Retired,
                ..Default::default()
            },
        );
        state.brief.record = Some(("local".into(), 0));
        assert!(
            state
                .captain_lifecycle_unavailable("local", 0, Action::Delete)
                .is_none()
        );
        assert!(
            state
                .captain_lifecycle_unavailable("local", 0, Action::Archive)
                .is_some()
        );
        assert!(
            state
                .captain_lifecycle_unavailable("local", 0, Action::Unarchive)
                .is_some()
        );
        std::fs::remove_dir_all(&state.data_dir).ok();
    }

    #[test]
    fn a_late_receipt_from_another_server_toasts_but_keeps_the_brief() {
        let mut state = fixture();
        state.show_activity = true;
        state.activity_full = true;
        // A demote queued on "studio" settles while the user has "local" and its
        // Brief open: the pending record clears, a toast lands, and the Brief
        // stays up because it belongs to the selected machine.
        let id = uuid::Uuid::new_v4();
        state.lifecycle_pending.insert(
            ("studio".into(), id),
            LifecycleChange {
                captain: 0,
                action: Action::Demote,
            },
        );
        assert!(state.captain_lifecycle_receipt(
            "studio",
            &command(id, "captain.demote"),
            &accepted(json!({"captainVersion":"2","lifecycle":"retired"}))
        ));
        assert!(state.lifecycle_pending.is_empty());
        assert!(state.show_activity);
        assert_eq!(state.toasts.len(), 1);
        // The same receipt on the selected machine closes it.
        let mut state = fixture();
        state.show_activity = true;
        state.activity_full = true;
        let id = uuid::Uuid::new_v4();
        state.lifecycle_pending.insert(
            ("local".into(), id),
            LifecycleChange {
                captain: 0,
                action: Action::Demote,
            },
        );
        assert!(state.captain_lifecycle_receipt(
            "local",
            &command(id, "captain.demote"),
            &accepted(json!({"captainVersion":"2","lifecycle":"retired"}))
        ));
        assert!(!state.show_activity);
        std::fs::remove_dir_all(&state.data_dir).ok();
    }

    #[test]
    fn preview_blockers_and_clientless_refusal_keep_confirm_safe() {
        let mut state = fixture();
        // Clientless local: the dialog opens on its own records.
        open(&mut state, "local", Action::Archive);
        let dialog = state.brief.lifecycle.as_ref().unwrap();
        assert!(dialog.preview.is_some());
        assert!(dialog.pending.is_none());
        // Mutations refuse rather than run outside a runtime transaction.
        let error = state.confirm_captain_lifecycle().unwrap_err();
        assert_eq!(error, "This Captain change requires the durable server.");
        assert!(state.lifecycle_pending.is_empty());
        // A preview blocker is returned before anything can be sent.
        state.brief.lifecycle.as_mut().unwrap().preview =
            Some(json!({"captainVersion":"1","crewSeen":[],"blockers":["Stop work first."]}));
        assert_eq!(
            state.confirm_captain_lifecycle().unwrap_err(),
            "Stop work first."
        );
        std::fs::remove_dir_all(&state.data_dir).ok();
    }
}
