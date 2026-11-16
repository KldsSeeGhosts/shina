//! Client seam for the `captain.reportPolicy` command: the radio control's
//! compare-and-set send, its pending record and the exact-receipt settle.
//! Every change goes through the machine's command queue, even on the local
//! durable server, so no policy mutates outside a runtime transaction. A
//! pending change stays queued across a link drop and settles only on its own
//! receipt, never on a projection update.
use super::*;

/// A report-policy change queued on a machine, awaiting its exact command
/// receipt. `expected` is the CAS value sent, kept so a rejection can say what
/// the user confirmed against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptainPolicyChange {
    pub captain: usize,
    pub policy: ReportPolicy,
    pub expected: ReportPolicy,
}

/// The message every rejected policy receipt surfaces in the section.
fn policy_receipt_error(value: &Value) -> String {
    value["error"]["message"]
        .as_str()
        .or_else(|| value["result"]["result"]["error"].as_str())
        .unwrap_or("The report policy change was rejected.")
        .to_string()
}

impl State {
    /// Queue a command on `server`'s channel. Local goes through the same
    /// queue as linked machines, so the mutation lands inside a runtime batch.
    fn policy_send(&self, server: &str, method: &str, params: Value) -> Result<uuid::Uuid, String> {
        let Some(client) = &self.client else {
            return Err("This Captain change requires the durable server.".into());
        };
        client
            .commands_for(server)
            .ok_or_else(|| "This machine is not linked.".to_string())?
            .send(method, params)
    }

    /// The negotiated `captain-events-v1` check for one machine. A clientless
    /// local State has no peer to negotiate with and passes; the send still
    /// refuses below.
    fn policy_capability(&self, server: &str) -> bool {
        let Some(client) = &self.client else {
            return true;
        };
        if server == "local" {
            client.has_capability(crate::protocol::CAPTAIN_EVENTS)
        } else {
            client
                .linked
                .get(server)
                .is_some_and(|link| link.has_capability(crate::protocol::CAPTAIN_EVENTS))
        }
    }

    /// Why the report-policy control is unavailable right now, or `None` when
    /// it may send. Checked both to disable the radios and again at press
    /// time. The change needs the selected machine's pin, the captain's actual
    /// current primary home (full access, never an earlier conversation or a
    /// record), a live connection, the negotiated capability and no
    /// already-queued change.
    pub fn captain_policy_unavailable(
        &self,
        expected_server: &str,
        captain: usize,
    ) -> Option<String> {
        if expected_server != self.machines.selected {
            return Some("That action belonged to another machine.".into());
        }
        let phase = self.captain_phase(captain);
        if captain >= self.captains.len() || phase == CaptainPhase::Deleted {
            return Some("No captain records.".into());
        }
        if phase == CaptainPhase::Archived {
            return Some("This captain is archived.".into());
        }
        if phase == CaptainPhase::Retired {
            return Some("This captain is already an ordinary thread.".into());
        }
        let server = self.machines.selected.clone();
        let label = self.machine_label(&server);
        if self.client.is_some() && !self.machine_connected(&server) {
            return Some(format!("{label} is unreachable."));
        }
        if !self.policy_capability(&server) {
            return Some(format!("Update Shina on {label} to change report policy."));
        }
        if self.captain_access(captain) != CaptainAccess::Full {
            return Some("Read only: an earlier conversation is open.".into());
        }
        if self
            .captain_policy_pending
            .iter()
            .any(|((host, _), change)| *host == server && change.captain == captain)
        {
            return Some(format!("Waiting for {label} to confirm."));
        }
        None
    }

    /// Send `captain.reportPolicy` with the current policy as the CAS
    /// `expectedPolicy`. The send is pinned to `expected_server`; the pending
    /// record is written at send time so a receipt can never settle a change
    /// that was never sent.
    pub fn change_captain_report_policy(
        &mut self,
        expected_server: &str,
        captain: usize,
        policy: ReportPolicy,
    ) -> Result<CrewMutation, String> {
        if let Some(reason) = self.captain_policy_unavailable(expected_server, captain) {
            return Err(reason);
        }
        let expected = self.captain_report_policy(captain);
        let id = self.policy_send(
            expected_server,
            "captain.reportPolicy",
            json!({"captain":captain,"policy":policy,"expectedPolicy":expected}),
        )?;
        self.captain_policy_pending.insert(
            (expected_server.to_string(), id),
            CaptainPolicyChange {
                captain,
                policy,
                expected,
            },
        );
        self.captain_policy_errors
            .remove(&(expected_server.to_string(), captain));
        Ok(CrewMutation::Pending(id))
    }

    /// Match a receipt to a queued policy change by exact server, UUID and
    /// method. Accepted clears that captain's stored error; a rejection writes
    /// it so the section shows the machine's own words. Nothing toasts: the
    /// Brief must never steal focus for a background rejection. Returns `true`
    /// only when all three match, so receipts from other machines, other
    /// commands or other methods fall through to the generic hooks untouched.
    pub(crate) fn captain_policy_receipt(
        &mut self,
        server: &str,
        command: &crate::protocol::Command,
        value: &Value,
    ) -> bool {
        let key = (server.to_string(), command.command_id);
        if command.method != "captain.reportPolicy" {
            return false;
        }
        let Some(change) = self.captain_policy_pending.remove(&key) else {
            return false;
        };
        if value["result"]["status"] == "accepted" {
            self.captain_policy_errors
                .remove(&(server.to_string(), change.captain));
        } else {
            self.captain_policy_errors.insert(
                (server.to_string(), change.captain),
                policy_receipt_error(value),
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> State {
        let mut state = State::new(
            std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()),
            false,
        );
        state.load_global_for_test();
        state.captains = Captain::presets();
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

    /// A linked "studio" machine whose local client also reports the local
    /// durable server's capabilities. Returns the remote command channel.
    fn linked(
        state: &mut State,
        local: &[&str],
        remote: &[&str],
    ) -> tokio::sync::mpsc::Receiver<crate::protocol::Command> {
        let (mut client, _local_commands) =
            crate::client::Client::test_connected(state.data_dir.clone());
        *client.info.lock().unwrap() = json!({"capabilities":local});
        let (remote_client, commands) =
            crate::client::Client::test_connected(state.data_dir.clone());
        *remote_client.info.lock().unwrap() = json!({"capabilities":remote});
        client.linked.insert("studio".into(), remote_client);
        state.client = Some(client);
        commands
    }

    #[test]
    fn the_change_sends_a_compare_and_set_command_on_the_pinned_machine() {
        let mut state = fixture();
        let mut commands = linked(
            &mut state,
            &[
                crate::protocol::CAPTAIN_STATE,
                crate::protocol::CAPTAIN_EVENTS,
            ],
            &[
                crate::protocol::CAPTAIN_STATE,
                crate::protocol::CAPTAIN_EVENTS,
            ],
        );
        state.machines.selected = "studio".into();
        // The studio view must name the same home for full access; pin only
        // affects which channel carries the command.
        let sent = state
            .change_captain_report_policy("studio", 0, ReportPolicy::Context)
            .unwrap();
        let CrewMutation::Pending(id) = sent else {
            panic!("expected a pending mutation");
        };
        let command = commands.try_recv().unwrap();
        assert_eq!(command.command_id, id);
        assert_eq!(command.method, "captain.reportPolicy");
        assert_eq!(command.params["captain"], 0);
        assert_eq!(command.params["policy"], "context");
        assert_eq!(command.params["expectedPolicy"], "react");
        assert!(
            state
                .captain_policy_pending
                .contains_key(&("studio".into(), id))
        );
        // A second press while one is in flight refuses instead of sending.
        let error = state
            .change_captain_report_policy("studio", 0, ReportPolicy::Ignore)
            .unwrap_err();
        assert!(error.contains("Waiting"), "{error}");
        assert!(commands.try_recv().is_err());
    }

    #[test]
    fn nothing_sends_for_the_wrong_machine_or_a_missing_capability() {
        let mut state = fixture();
        let mut commands = linked(
            &mut state,
            &[
                crate::protocol::CAPTAIN_STATE,
                crate::protocol::CAPTAIN_EVENTS,
            ],
            &[],
        );
        // Wrong machine pin: the selected machine is local, not studio.
        let error = state
            .change_captain_report_policy("studio", 0, ReportPolicy::Ignore)
            .unwrap_err();
        assert_eq!(error, "That action belonged to another machine.");
        // Old host: studio lacks captain-events-v1.
        state.machines.selected = "studio".into();
        let error = state
            .change_captain_report_policy("studio", 0, ReportPolicy::Ignore)
            .unwrap_err();
        assert!(error.contains("Update Shina"), "{error}");
        assert!(commands.try_recv().is_err());
        assert!(state.captain_policy_pending.is_empty());
    }

    #[test]
    fn receipts_settle_only_on_exact_server_uuid_and_method() {
        let mut state = fixture();
        let mut commands = linked(
            &mut state,
            &[
                crate::protocol::CAPTAIN_STATE,
                crate::protocol::CAPTAIN_EVENTS,
            ],
            &[
                crate::protocol::CAPTAIN_STATE,
                crate::protocol::CAPTAIN_EVENTS,
            ],
        );
        state.machines.selected = "studio".into();
        let CrewMutation::Pending(id) = state
            .change_captain_report_policy("studio", 0, ReportPolicy::Ignore)
            .unwrap()
        else {
            panic!("expected a pending mutation");
        };
        let _ = commands.try_recv();
        let ok = accepted(json!({"captain":0,"reportPolicy":"ignore","saved":true}));
        // Wrong server, wrong UUID and wrong method are all ignored and leave
        // the change queued.
        assert!(!state.captain_policy_receipt("local", &command(id, "captain.reportPolicy"), &ok));
        assert!(!state.captain_policy_receipt(
            "studio",
            &command(uuid::Uuid::new_v4(), "captain.reportPolicy"),
            &ok
        ));
        assert!(!state.captain_policy_receipt("studio", &command(id, "captain.archive"), &ok));
        assert_eq!(state.captain_policy_pending.len(), 1);
        // The exact receipt settles it.
        assert!(state.captain_policy_receipt("studio", &command(id, "captain.reportPolicy"), &ok));
        assert!(state.captain_policy_pending.is_empty());
        assert!(
            state
                .captain_policy_errors
                .get(&("studio".into(), 0))
                .is_none()
        );
    }

    #[test]
    fn a_rejection_writes_the_per_machine_error_without_a_toast() {
        let mut state = fixture();
        let mut commands = linked(
            &mut state,
            &[
                crate::protocol::CAPTAIN_STATE,
                crate::protocol::CAPTAIN_EVENTS,
            ],
            &[
                crate::protocol::CAPTAIN_STATE,
                crate::protocol::CAPTAIN_EVENTS,
            ],
        );
        state.machines.selected = "studio".into();
        let CrewMutation::Pending(id) = state
            .change_captain_report_policy("studio", 0, ReportPolicy::Context)
            .unwrap()
        else {
            panic!("expected a pending mutation");
        };
        let _ = commands.try_recv();
        assert!(state.captain_policy_receipt(
            "studio",
            &command(id, "captain.reportPolicy"),
            &rejected(
                "Report policy changed since 'context'. Read again and confirm the new value."
            )
        ));
        assert_eq!(
            state
                .captain_policy_errors
                .get(&("studio".into(), 0))
                .map(String::as_str),
            Some("Report policy changed since 'context'. Read again and confirm the new value.")
        );
        assert!(state.captain_policy_pending.is_empty());
        // The stored error does not leak onto another machine's section.
        assert!(
            state
                .captain_policy_errors
                .get(&("local".into(), 0))
                .is_none()
        );
        // A fresh press clears it once the new send lands.
        state
            .change_captain_report_policy("studio", 0, ReportPolicy::Context)
            .unwrap();
        let _ = commands.try_recv();
        assert!(
            state
                .captain_policy_errors
                .get(&("studio".into(), 0))
                .is_none()
        );
    }
}
