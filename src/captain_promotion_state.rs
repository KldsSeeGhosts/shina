//! Client-side Captain promotion: an existing thread becomes a Captain's
//! primary home through the `captain_editor` in promote mode
//! (`CaptainEditor.promotion`). The backend's `captain.promote.preview` read is
//! authoritative for gate blockers and the `threadVersion` the mutation sends
//! back; only an exact `captain.promote` receipt settles the change, never a
//! projection. Mutations never run outside a runtime transaction, so a
//! clientless State refuses them instead of applying. Design: the follow-on UI
//! design's "Promote an existing thread" flow; there is no KSG Code equivalent.
use super::*;
use crate::captains::{CaptainPromotion, PROMOTE_CONFLICT_PREFIX};
use crate::protocol::Command;

pub(crate) const PROMOTE_PREVIEW: &str = "captain.promote.preview";
pub(crate) const PROMOTE: &str = "captain.promote";

/// A `captain.promote` mutation queued on a machine, awaiting its exact command
/// receipt. `profile` and `restore` keep the confirmed intent so a rejection can
/// reopen the editor with the fields the user reviewed.
#[derive(Clone, Debug)]
pub struct PromotionChange {
    pub thread: usize,
    pub title: String,
    pub profile: Captain,
    pub restore: Option<usize>,
}

/// The message every rejected promotion receipt surfaces, wherever it landed.
fn promotion_receipt_error(value: &Value) -> String {
    value["error"]["message"]
        .as_str()
        .or_else(|| value["result"]["result"]["error"].as_str())
        .unwrap_or("The promotion was rejected.")
        .to_string()
}

impl State {
    /// Queue a command on `server`'s channel. Local goes through the same queue
    /// as linked machines, so every mutation lands inside a runtime batch.
    fn promotion_send(
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

    /// The `captain-promote-v1` negotiated-capability check for one machine. A
    /// clientless local State has no peer to negotiate with and passes; the
    /// send still refuses below.
    fn promote_capability(&self, server: &str) -> bool {
        let Some(client) = &self.client else {
            return true;
        };
        client.supports_capability(server, crate::protocol::CAPTAIN_PROMOTE)
    }

    /// The thread-identity and crew gates, in the order the menu checks them.
    /// Structural exclusions (draft, Captain home, delegated or peer worker,
    /// child thread) hide the entry point entirely; crew membership, archive
    /// and busy states disable it with these reasons. The busy checks reuse
    /// `join_gate`'s text, as the design specifies.
    fn promotion_thread_gate(&self, index: usize) -> Result<(), String> {
        let thread = &self.threads[index];
        if thread.client_draft {
            return Err("Send the thread's first message before making it a Captain.".into());
        }
        if thread.captain_home
            || thread.delegated
            || thread.peer_origin.is_some()
            || thread.parent.is_some()
        {
            return Err("Only an ordinary independent thread can become a Captain.".into());
        }
        if let Some(captain) = thread.crew_captain {
            let name = self
                .captains
                .get(captain)
                .map(|captain| captain.name.as_str())
                .unwrap_or("the Captain");
            return Err(format!("Release it from {name}'s crew first."));
        }
        if thread.archived() {
            return Err("Restore the thread first.".into());
        }
        self.join_gate(index)
    }

    /// The retired Captain whose profile this thread can restore: the record
    /// must be `Retired`, name this thread as its surviving former home, and
    /// still start its workspace scope on the thread's current project, matching
    /// the backend's own eligibility check in `promotion_gate`.
    fn promotion_restore_candidate(
        &self,
        thread_id: usize,
        project: Option<usize>,
    ) -> Option<usize> {
        (0..self.captains.len()).find(|&captain| {
            self.captain_phase(captain) == CaptainPhase::Retired
                && self.captain_former_home(captain) == Some(thread_id)
                && self.captain_scope(captain).first().copied() == project
        })
    }

    /// The fresh suggested name and portrait for a new profile: the first
    /// shipped portrait whose face and name are both unused, else a numbered
    /// "Captain {n}" on a cycling portrait. Same rule `begin_new_captain` uses.
    fn fresh_captain_suggestion(&self) -> (String, usize) {
        let used_names: std::collections::HashSet<String> = self
            .captains
            .iter()
            .map(|c| c.name.to_lowercase())
            .collect();
        let used_portraits: std::collections::HashSet<usize> =
            self.captains.iter().map(|c| c.portrait).collect();
        (0..crate::visuals::PORTRAITS)
            .find(|i| {
                !used_portraits.contains(i)
                    && !used_names.contains(&crate::visuals::CAPTAIN_NAMES[*i].to_lowercase())
            })
            .map(|i| (crate::visuals::CAPTAIN_NAMES[i].to_string(), i))
            .unwrap_or_else(|| {
                let mut idx = self.captains.len() + 1;
                let candidate = loop {
                    let c = format!("Captain {idx}");
                    if !used_names.contains(&c.to_lowercase()) {
                        break c;
                    }
                    idx += 1;
                };
                (candidate, self.captains.len() % crate::visuals::PORTRAITS)
            })
    }

    /// Copy a roster profile into the open editor's fields. Used to fill (and,
    /// while `restore` is on, re-pin) the retired Captain's record.
    fn apply_captain_profile(&mut self, captain: usize) {
        let Some(profile) = self.captains.get(captain).cloned() else {
            return;
        };
        let Some(editor) = self.captain_editor.as_mut() else {
            return;
        };
        editor.name.replace_text(&profile.name);
        editor.role.replace_text(&profile.role);
        editor.instructions.replace_text(&profile.instructions);
        editor.portrait = profile.portrait;
    }

    /// Why promotion controls for `thread_id` are unavailable right now, or
    /// `None` when they may proceed. Checked both to hide or disable the menu
    /// item and again at press time. `expected_server` is the machine the
    /// control rendered for; a stale click from another selection sends nothing.
    pub fn captain_promotion_unavailable(
        &self,
        expected_server: &str,
        thread_id: usize,
    ) -> Option<String> {
        if expected_server != self.machines.selected {
            return Some("That action belonged to another machine.".into());
        }
        let Some(index) = self
            .threads
            .iter()
            .position(|thread| thread.id == thread_id)
        else {
            return Some("This thread is no longer available on that machine.".into());
        };
        if let Err(reason) = self.promotion_thread_gate(index) {
            return Some(reason);
        }
        let server = self.machines.selected.clone();
        let label = self.machine_label(&server);
        if self
            .promotion_pending
            .iter()
            .any(|((host, _), change)| host == &server && change.thread == thread_id)
        {
            return Some(format!("Waiting for {label} to confirm."));
        }
        if self.client.is_some() && !self.machine_connected(&server) {
            return Some(format!("{label} is unreachable."));
        }
        if !self.promote_capability(&server) {
            return Some(format!("Update Shina on {label} to promote captains."));
        }
        None
    }

    /// Open the `captain_editor` in promote mode for `thread_id`, reusing
    /// `begin_new_captain`'s suggested name and portrait, then pin the thread's
    /// identity and immediately ask the owning machine for its authoritative
    /// preview. When the thread is a retired Captain's surviving former home,
    /// restore defaults on with that record's profile. The dialog is bound to
    /// `expected_server` for its whole life.
    pub fn begin_captain_promotion(
        &mut self,
        expected_server: &str,
        thread_id: usize,
    ) -> Result<(), String> {
        if let Some(reason) = self.captain_promotion_unavailable(expected_server, thread_id) {
            return Err(reason);
        }
        let index = self
            .threads
            .iter()
            .position(|thread| thread.id == thread_id)
            .ok_or("This thread is no longer available on that machine.")?;
        let title = self.threads[index].title.clone();
        let project = self.threads[index].project;
        self.begin_new_captain();
        let restore_candidate = self.promotion_restore_candidate(thread_id, project);
        let editor = self.captain_editor.as_mut().expect("begin_new_captain");
        editor.server = expected_server.to_owned();
        editor.machine_picker = false;
        editor.promotion = Some(CaptainPromotion {
            server: expected_server.to_owned(),
            thread: thread_id,
            title,
            project,
            restore: restore_candidate,
            restore_candidate,
            preview: None,
            pending: None,
        });
        // The workspace is the thread's own and never a choice in promote mode.
        self.captain_project = Some(project);
        if let Some(captain) = restore_candidate {
            self.apply_captain_profile(captain);
        }
        self.refresh_captain_promotion_preview()
    }

    /// Re-read the preview for the open promotion editor. Remote machines
    /// answer through a `captain.promote.preview` read command matched by exact
    /// receipt; a clientless local State reads its own records, which is safe
    /// because a preview is a `&self` query, not a mutation. Typed fields and
    /// the restore switch are kept; a conflict notice survives the refresh it
    /// triggers until a fresh explicit press clears it.
    pub fn refresh_captain_promotion_preview(&mut self) -> Result<(), String> {
        let Some(promotion) = self
            .captain_editor
            .as_ref()
            .and_then(|editor| editor.promotion.as_ref())
        else {
            return Err("No captain promotion is open.".into());
        };
        if promotion.pending.is_some() {
            return Err("Waiting for the machine to answer.".into());
        }
        let server = promotion.server.clone();
        let thread_id = promotion.thread;
        let restore = promotion.restore_captain();
        let editor = self.captain_editor.as_mut().expect("editor");
        if !CaptainPromotion::conflict(editor.error.as_deref()) {
            editor.error = None;
        }
        editor.promotion.as_mut().expect("promotion").preview = None;
        if self.client.is_none() && server == "local" {
            let preview = self.captain_promote_preview(thread_id, restore)?;
            if let Some(promotion) = self
                .captain_editor
                .as_mut()
                .and_then(|editor| editor.promotion.as_mut())
            {
                promotion.preview = Some(preview);
            }
            return Ok(());
        }
        let mut params = json!({"threadId":thread_id});
        if let Some(captain) = restore {
            params["restoreCaptain"] = json!(captain);
        }
        let id = self.promotion_send(&server, PROMOTE_PREVIEW, params)?;
        if let Some(promotion) = self
            .captain_editor
            .as_mut()
            .and_then(|editor| editor.promotion.as_mut())
        {
            promotion.pending = Some(id);
        }
        Ok(())
    }

    /// Flip the former-home restore switch. On fills and locks the profile
    /// fields from the retired record; off unlocks them with a fresh suggested
    /// profile. The preview is re-read either way, because `restoreCaptain`
    /// changes the backend's `threadVersion`. A preview already in flight is
    /// dropped as stale: its id never matches the new pending command.
    pub fn set_captain_promotion_restore(&mut self, restore: bool) -> Result<(), String> {
        let Some(promotion) = self
            .captain_editor
            .as_ref()
            .and_then(|editor| editor.promotion.as_ref())
        else {
            return Err("No captain promotion is open.".into());
        };
        if restore && promotion.restore_candidate.is_none() {
            return Err("This thread has no retired Captain profile to restore.".into());
        }
        if let Some(id) = promotion.pending {
            // A sent mutation is never cancelled by the switch.
            let in_flight = self
                .promotion_pending
                .contains_key(&(promotion.server.clone(), id));
            if in_flight {
                return Err("This change is already waiting for a reply.".into());
            }
        }
        let candidate = if restore {
            promotion.restore_candidate
        } else {
            None
        };
        self.captain_editor
            .as_mut()
            .and_then(|editor| editor.promotion.as_mut())
            .expect("promotion")
            .restore = candidate;
        self.captain_editor
            .as_mut()
            .and_then(|editor| editor.promotion.as_mut())
            .expect("promotion")
            .pending = None;
        if let Some(captain) = candidate {
            self.apply_captain_profile(captain);
        } else {
            let (name, portrait) = self.fresh_captain_suggestion();
            let editor = self.captain_editor.as_mut().expect("editor");
            editor.name.replace_text(&name);
            editor.role.replace_text("");
            editor.instructions.replace_text("");
            editor.portrait = portrait;
        }
        self.refresh_captain_promotion_preview()
    }

    /// Send `captain.promote` with the preview's exact `threadVersion`. Every
    /// press mints a fresh UUID; an editor with a command in flight refuses, so
    /// a double press never submits twice. The editor stays open on Pending
    /// until the exact mutation receipt arrives; a stale-version refusal
    /// re-reads the preview and keeps every field for a review-again press.
    /// A clientless State refuses rather than mutating outside a transaction.
    pub fn save_captain_promotion(&mut self) -> Result<(), String> {
        let Some(editor) = self.captain_editor.as_ref() else {
            return Err("No captain editor is open.".into());
        };
        let Some(promotion) = editor.promotion.as_ref() else {
            return Err("This editor is not promoting a thread.".into());
        };
        if promotion.pending.is_some() {
            return Err("This change is already waiting for a reply.".into());
        }
        let server = promotion.server.clone();
        let thread_id = promotion.thread;
        let title = promotion.title.clone();
        let restore = promotion.restore_captain();
        if let Some(reason) = self.captain_promotion_unavailable(&server, thread_id) {
            return Err(reason);
        }
        let profile = editor.profile();
        profile.validate()?;
        if let Some(captain) = restore
            && self.captains.get(captain) != Some(&profile)
        {
            return Err("Restore uses the retired Captain's unchanged profile.".into());
        }
        let Some(preview) = promotion.preview.clone() else {
            return Err("Wait for the preview before confirming.".into());
        };
        if let Some(blocker) = preview["blockers"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(Value::as_str)
        {
            return Err(blocker.into());
        }
        let Some(expected) = preview["threadVersion"].as_str() else {
            return Err("The preview carried no thread version.".into());
        };
        let mut params = json!({
            "threadId":thread_id,
            "record":profile.to_value(),
            "expectedVersion":expected,
        });
        if let Some(captain) = restore {
            params["restoreCaptain"] = json!(captain);
        }
        let id = self.promotion_send(&server, PROMOTE, params)?;
        self.promotion_pending.insert(
            (server, id),
            PromotionChange {
                thread: thread_id,
                title,
                profile,
                restore,
            },
        );
        let editor = self.captain_editor.as_mut().expect("editor");
        if let Some(promotion) = editor.promotion.as_mut() {
            promotion.pending = Some(id);
        }
        editor.error = None;
        Ok(())
    }

    /// Match a receipt to the open editor's in-flight command or a pending
    /// mutation by exact server, UUID and method. Returns `true` only when all
    /// three match, so receipts from other machines, other commands or other
    /// methods fall through to the generic hooks untouched.
    pub(crate) fn captain_promotion_receipt(
        &mut self,
        server: &str,
        command: &Command,
        value: &Value,
    ) -> bool {
        let key = (server.to_string(), command.command_id);
        let on_editor = self
            .captain_editor
            .as_ref()
            .and_then(|editor| editor.promotion.as_ref())
            .is_some_and(|promotion| {
                promotion.server == server && promotion.pending == Some(command.command_id)
            });
        if on_editor {
            // A mutation receipt settles only a mutation that was actually sent:
            // the pending record is written at save time.
            if command.method == PROMOTE && self.promotion_pending.contains_key(&key) {
                self.settle_promotion_mutation(server, command, value);
                return true;
            }
            if command.method == PROMOTE_PREVIEW && !self.promotion_pending.contains_key(&key) {
                self.settle_promotion_preview(value);
                return true;
            }
            return false;
        }
        // The editor may already be closed: the pending record still owns the
        // command's intent, so its receipt settles with a toast.
        if command.method == PROMOTE && self.promotion_pending.contains_key(&key) {
            self.settle_promotion_mutation(server, command, value);
            return true;
        }
        false
    }

    fn settle_promotion_preview(&mut self, value: &Value) {
        let accepted = value["result"]["status"] == "accepted";
        let Some(editor) = self.captain_editor.as_mut() else {
            return;
        };
        if let Some(promotion) = editor.promotion.as_mut() {
            promotion.pending = None;
            if accepted {
                promotion.preview = Some(value["result"]["result"].clone());
            }
        }
        if accepted {
            if !CaptainPromotion::conflict(editor.error.as_deref()) {
                editor.error = None;
            }
        } else {
            editor.error = Some(promotion_receipt_error(value));
        }
    }

    /// A mutation's own receipt is the only thing that settles it; projection
    /// changes never close the editor or clear the pending record. Acceptance
    /// closes only the editor still bound to that exact command, so a late
    /// receipt never steals focus or closes a different editor. A stale-version
    /// rejection refreshes the preview and keeps every field for an explicit
    /// review-again press; it never re-sends the old UUID.
    fn settle_promotion_mutation(&mut self, server: &str, command: &Command, value: &Value) {
        let key = (server.to_string(), command.command_id);
        let Some(change) = self.promotion_pending.remove(&key) else {
            return;
        };
        let editor_match = self
            .captain_editor
            .as_ref()
            .and_then(|editor| editor.promotion.as_ref())
            .is_some_and(|promotion| {
                promotion.server == server
                    && promotion.thread == change.thread
                    && promotion.pending == Some(command.command_id)
            });
        if value["result"]["status"] == "accepted" {
            if editor_match {
                self.captain_editor = None;
                self.captain_project = None;
                self.projects.captain_target = None;
            }
            self.push_toast(
                ToastKind::Success,
                format!("Made \"{}\" a captain.", change.title),
                None,
                None,
                Some(6000),
            );
            return;
        }
        let message = promotion_receipt_error(value);
        let conflict = message.starts_with(PROMOTE_CONFLICT_PREFIX);
        let on_editor = self
            .captain_editor
            .as_ref()
            .and_then(|editor| editor.promotion.as_ref())
            .is_some_and(|promotion| {
                promotion.server == server && promotion.thread == change.thread
            });
        if on_editor {
            if let Some(editor) = self.captain_editor.as_mut() {
                if let Some(promotion) = editor.promotion.as_mut() {
                    promotion.pending = None;
                }
                editor.error = Some(message);
            }
            // A stale version re-reads the thread for a fresh explicit
            // confirmation. The typed fields and restore switch stay; nothing
            // re-sends the old UUID or confirms on its own.
            if conflict {
                let _ = self.refresh_captain_promotion_preview();
            }
        } else {
            self.push_toast(
                ToastKind::Error,
                format!("Could not make \"{}\" a captain", change.title),
                Some(message),
                Some((
                    "Edit again".into(),
                    ToastAction::ReviewCaptainPromotion {
                        server: server.to_owned(),
                        thread_id: change.thread,
                        profile: change.profile,
                        restore: change.restore,
                    },
                )),
                Some(6000),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::captain_lifecycle::CaptainLifecycle;
    use super::*;

    fn fixture() -> State {
        let mut state = State::new(
            std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()),
            false,
        );
        state.load_global_for_test();
        state.captains = crate::captains::Captain::presets();
        state
    }

    fn command(id: uuid::Uuid, method: &str) -> Command {
        Command {
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

    fn linked(state: &mut State, capabilities: &[&str]) -> tokio::sync::mpsc::Receiver<Command> {
        let (mut client, _local) = crate::client::Client::test_connected(state.data_dir.clone());
        let (remote, commands) = crate::client::Client::test_connected(state.data_dir.clone());
        *remote.info.lock().unwrap() = json!({"capabilities":capabilities});
        client.linked.insert("studio".into(), remote);
        state.client = Some(client);
        commands
    }

    fn open(state: &mut State, server: &str) -> usize {
        let thread = state.current().id;
        state.begin_captain_promotion(server, thread).unwrap();
        thread
    }

    #[test]
    fn receipts_settle_only_on_exact_server_uuid_and_method() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_PROMOTE]);
        state.machines.selected = "studio".into();
        let thread = open(&mut state, "studio");
        let preview_id = commands.try_recv().unwrap().command_id;
        let promotion = state
            .captain_editor
            .as_ref()
            .unwrap()
            .promotion
            .as_ref()
            .unwrap();
        assert_eq!(promotion.pending, Some(preview_id));
        assert_eq!(promotion.thread, thread);
        assert_eq!(promotion.server, "studio");
        // Wrong server, wrong UUID and wrong method are all ignored.
        let preview = accepted(
            json!({"threadId":thread,"threadVersion":"v1","blockers":[],"permissionsChanged":false}),
        );
        assert!(!state.captain_promotion_receipt(
            "local",
            &command(preview_id, PROMOTE_PREVIEW),
            &preview
        ));
        assert!(!state.captain_promotion_receipt(
            "studio",
            &command(uuid::Uuid::new_v4(), PROMOTE_PREVIEW),
            &preview
        ));
        assert!(!state.captain_promotion_receipt(
            "studio",
            &command(preview_id, PROMOTE),
            &preview
        ));
        assert!(
            state
                .captain_editor
                .as_ref()
                .unwrap()
                .promotion
                .as_ref()
                .unwrap()
                .preview
                .is_none()
        );
        // The exact preview receipt fills the editor.
        assert!(state.captain_promotion_receipt(
            "studio",
            &command(preview_id, PROMOTE_PREVIEW),
            &preview
        ));
        let promotion = state
            .captain_editor
            .as_ref()
            .unwrap()
            .promotion
            .as_ref()
            .unwrap();
        assert!(promotion.pending.is_none());
        assert_eq!(promotion.preview.as_ref().unwrap()["threadVersion"], "v1");
        // Confirm sends the mutation with the preview's version, then ignores a
        // preview-method receipt carrying the same UUID.
        state.save_captain_promotion().unwrap();
        let sent = commands.try_recv().unwrap();
        assert_eq!(sent.method, PROMOTE);
        assert_eq!(sent.params["threadId"].as_u64(), Some(thread as u64));
        assert_eq!(sent.params["expectedVersion"], "v1");
        assert!(sent.params.get("restoreCaptain").is_none());
        let name = state
            .captain_editor
            .as_ref()
            .unwrap()
            .name
            .text
            .trim()
            .to_string();
        assert_eq!(sent.params["record"]["name"].as_str(), Some(name.as_str()));
        assert!(
            state
                .promotion_pending
                .contains_key(&("studio".into(), sent.command_id))
        );
        assert!(!state.captain_promotion_receipt(
            "studio",
            &command(sent.command_id, PROMOTE_PREVIEW),
            &preview
        ));
        assert!(!state.captain_promotion_receipt(
            "local",
            &command(sent.command_id, PROMOTE),
            &accepted(json!({}))
        ));
        // Only the exact mutation receipt closes the editor.
        assert!(state.captain_promotion_receipt(
            "studio",
            &command(sent.command_id, PROMOTE),
            &accepted(json!({"captain":4,"threadId":thread,"captainVersion":"v2"}))
        ));
        assert!(state.captain_editor.is_none());
        assert!(state.promotion_pending.is_empty());
    }

    #[test]
    fn stale_mutation_refreshes_preview_and_keeps_fields() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_PROMOTE]);
        state.machines.selected = "studio".into();
        open(&mut state, "studio");
        let preview_id = commands.try_recv().unwrap().command_id;
        state
            .captain_editor
            .as_mut()
            .unwrap()
            .name
            .replace_text("Ember Two");
        assert!(state.captain_promotion_receipt(
            "studio",
            &command(preview_id, PROMOTE_PREVIEW),
            &accepted(json!({"threadId":1,"threadVersion":"v1","blockers":[]}))
        ));
        state.save_captain_promotion().unwrap();
        let mutation = commands.try_recv().unwrap();
        assert!(state.captain_promotion_receipt(
            "studio",
            &mutation,
            &rejected("Thread changed since version. Review a fresh promotion preview.")
        ));
        // The editor stays open with its fields and a review-again notice; a
        // fresh preview was re-read and the pending record is gone.
        let editor = state.captain_editor.as_ref().unwrap();
        assert_eq!(editor.name.text.trim(), "Ember Two");
        assert!(CaptainPromotion::conflict(editor.error.as_deref()));
        assert!(state.promotion_pending.is_empty());
        let refresh = commands.try_recv().unwrap();
        assert_eq!(refresh.method, PROMOTE_PREVIEW);
        assert!(state.captain_promotion_receipt(
            "studio",
            &refresh,
            &accepted(json!({"threadId":1,"threadVersion":"v2","blockers":[]}))
        ));
        // The next explicit press mints a new UUID with the fresh version.
        state.save_captain_promotion().unwrap();
        let retry = commands.try_recv().unwrap();
        assert_eq!(retry.method, PROMOTE);
        assert_ne!(retry.command_id, mutation.command_id);
        assert_eq!(retry.params["expectedVersion"], "v2");
    }

    #[test]
    fn former_home_restores_the_retired_profile_and_the_toggle_unlocks_it() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_PROMOTE]);
        state.machines.selected = "studio".into();
        let thread = state.current().id;
        state.captain_lifecycles.insert(
            0,
            CaptainLifecycle {
                phase: CaptainPhase::Retired,
                retired_at: Some(7),
                former_home: Some(thread),
                ..Default::default()
            },
        );
        state.begin_captain_promotion("studio", thread).unwrap();
        commands.try_recv().unwrap();
        let editor = state.captain_editor.as_ref().unwrap();
        let promotion = editor.promotion.as_ref().unwrap();
        assert_eq!(promotion.restore_candidate, Some(0));
        assert_eq!(promotion.restore, Some(0));
        let retired = state.captains[0].clone();
        assert_eq!(editor.name.text.trim(), retired.name);
        assert_eq!(editor.role.text.trim(), retired.role);
        assert_eq!(editor.instructions.text.trim(), retired.instructions);
        assert_eq!(editor.portrait, retired.portrait);
        // Off, the fields unlock with a fresh suggestion and the preview is re-read.
        state.set_captain_promotion_restore(false).unwrap();
        let toggle = commands.try_recv().unwrap();
        assert_eq!(toggle.method, PROMOTE_PREVIEW);
        assert!(toggle.params.get("restoreCaptain").is_none());
        let editor = state.captain_editor.as_ref().unwrap();
        let promotion = editor.promotion.as_ref().unwrap();
        assert_eq!(promotion.restore_candidate, Some(0));
        assert!(promotion.restore.is_none());
        assert_ne!(editor.name.text.trim(), retired.name);
        assert!(editor.role.text.trim().is_empty());
        // Back on, the record's profile is pinned again.
        state.set_captain_promotion_restore(true).unwrap();
        let toggle = commands.try_recv().unwrap();
        assert_eq!(toggle.params["restoreCaptain"].as_u64(), Some(0));
        let editor = state.captain_editor.as_ref().unwrap();
        assert_eq!(editor.name.text.trim(), retired.name);
        assert_eq!(editor.instructions.text.trim(), retired.instructions);
    }

    #[test]
    fn blocked_threads_and_machines_refuse_without_sending() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_PROMOTE]);
        state.machines.selected = "studio".into();
        let thread = state.current().id;
        // Another machine's action is refused outright.
        assert_eq!(
            state
                .captain_promotion_unavailable("local", thread)
                .as_deref(),
            Some("That action belonged to another machine.")
        );
        // Crew membership.
        state.current_mut().crew_captain = Some(0);
        assert!(
            state
                .captain_promotion_unavailable("studio", thread)
                .unwrap()
                .contains("crew first")
        );
        assert!(state.begin_captain_promotion("studio", thread).is_err());
        state.current_mut().crew_captain = None;
        // Archived.
        state.current_mut().life.archived_at = Some(1);
        assert_eq!(
            state
                .captain_promotion_unavailable("studio", thread)
                .as_deref(),
            Some("Restore the thread first.")
        );
        state.current_mut().life.archived_at = None;
        // A never-sent client draft.
        state.current_mut().client_draft = true;
        assert!(
            state
                .captain_promotion_unavailable("studio", thread)
                .unwrap()
                .contains("first message")
        );
        state.current_mut().client_draft = false;
        // Busy.
        state.current_mut().configuring = true;
        assert!(
            state
                .captain_promotion_unavailable("studio", thread)
                .is_some()
        );
        state.current_mut().configuring = false;
        assert!(
            state
                .captain_promotion_unavailable("studio", thread)
                .is_none()
        );
        // Nothing was ever sent.
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn unsupported_capability_and_unreachable_machine_refuse() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_STATE]);
        state.machines.selected = "studio".into();
        let thread = state.current().id;
        assert!(
            state
                .captain_promotion_unavailable("studio", thread)
                .unwrap()
                .contains("Update Shina")
        );
        assert!(state.begin_captain_promotion("studio", thread).is_err());
        // A selected machine with no link at all reports unreachable.
        state.client.as_mut().unwrap().linked.remove("studio");
        assert!(
            state
                .captain_promotion_unavailable("studio", thread)
                .unwrap()
                .contains("unreachable")
        );
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn pending_survives_editor_close_and_toasts_the_receipt() {
        let mut state = fixture();
        let mut commands = linked(&mut state, &[crate::protocol::CAPTAIN_PROMOTE]);
        state.machines.selected = "studio".into();
        open(&mut state, "studio");
        let preview_id = commands.try_recv().unwrap().command_id;
        assert!(state.captain_promotion_receipt(
            "studio",
            &command(preview_id, PROMOTE_PREVIEW),
            &accepted(json!({"threadId":1,"threadVersion":"v1","blockers":[]}))
        ));
        state.save_captain_promotion().unwrap();
        let mutation = commands.try_recv().unwrap();
        // Closing the editor keeps the pending record; a rejection toasts instead.
        let title = state
            .captain_editor
            .as_ref()
            .unwrap()
            .promotion
            .as_ref()
            .unwrap()
            .title
            .clone();
        state.captain_editor = None;
        assert!(state.captain_promotion_receipt(
            "studio",
            &mutation,
            &rejected("The machine said no.")
        ));
        assert!(state.promotion_pending.is_empty());
        assert_eq!(
            state.toasts.last().unwrap().title,
            format!("Could not make \"{title}\" a captain")
        );
        // Reopen and confirm again; an accepted late receipt never reopens or
        // steals anything.
        open(&mut state, "studio");
        let preview_id = commands.try_recv().unwrap().command_id;
        assert!(state.captain_promotion_receipt(
            "studio",
            &command(preview_id, PROMOTE_PREVIEW),
            &accepted(json!({"threadId":1,"threadVersion":"v1","blockers":[]}))
        ));
        state.save_captain_promotion().unwrap();
        let mutation = commands.try_recv().unwrap();
        let title = state
            .captain_editor
            .as_ref()
            .unwrap()
            .promotion
            .as_ref()
            .unwrap()
            .title
            .clone();
        state.captain_editor = None;
        assert!(state.captain_promotion_receipt(
            "studio",
            &mutation,
            &accepted(json!({"captain":4,"threadId":1,"captainVersion":"v2"}))
        ));
        assert_eq!(
            state.toasts.last().unwrap().title,
            format!("Made \"{title}\" a captain.")
        );
        // A stranger's receipt for an unrelated editor is ignored.
        state.begin_new_captain();
        assert!(!state.captain_promotion_receipt(
            "studio",
            &command(uuid::Uuid::new_v4(), PROMOTE),
            &accepted(json!({}))
        ));
    }
}
