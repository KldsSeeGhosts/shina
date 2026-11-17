//! Durable per-Captain report wake policy. Each crew report snapshots its
//! Captain's policy at capture; the stored record alone decides delivery,
//! membership blocking and restore validation. Later policy changes never
//! rewrite queued or outstanding snapshots.
use super::*;
use serde::{Deserialize, Serialize};

/// How a Captain handles a captured crew report. `react` queues a wake on the
/// current home, `context` retains the report until an explicit user turn or
/// resume action releases it, `ignore` never injects or wakes and the record
/// simply stays undelivered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportPolicy {
    #[default]
    React,
    Context,
    Ignore,
}

impl ReportPolicy {
    pub fn id(&self) -> &'static str {
        match self {
            ReportPolicy::React => "react",
            ReportPolicy::Context => "context",
            ReportPolicy::Ignore => "ignore",
        }
    }
}

impl State {
    /// The Captain's current wake policy for future report captures. Implicit
    /// `react` entries are omitted from the durable map.
    pub fn captain_report_policy(&self, captain: usize) -> ReportPolicy {
        self.captain_report_policies
            .get(&captain)
            .copied()
            .unwrap_or_default()
    }

    /// `captain-events-v1` gate: the reportPolicy user command is only
    /// negotiated for clients that declare minor 35 support.
    pub(crate) fn require_captain_events_capability(&self) -> Result<(), String> {
        let Some(client) = &self.client else {
            return Ok(());
        };
        let target = if self.machines.selected == "local" {
            Some(client)
        } else {
            client.linked.get(&self.machines.selected)
        };
        match target {
            Some(target) if !target.has_capability(crate::protocol::CAPTAIN_EVENTS) => {
                Err(crate::protocol::CAPTAIN_EVENTS_UPDATE_REQUIRED.into())
            }
            None if self.machines.selected != "local" => Err("This machine is not linked.".into()),
            _ => Ok(()),
        }
    }

    /// `captain.reportPolicy` user command: compare-and-set the durable wake
    /// policy. `expectedPolicy` must name the current policy so a stale
    /// confirmation cannot overwrite an intervening change. Future reports
    /// snapshot the new value; queued and outstanding reports are untouched.
    pub fn set_captain_report_policy(
        &mut self,
        captain: usize,
        policy: ReportPolicy,
        expected_policy: ReportPolicy,
    ) -> Result<Value, String> {
        self.require_captain_events_capability()?;
        if let Some(result) = self.remote(
            "captain.reportPolicy",
            json!({"captain":captain,"policy":policy,"expectedPolicy":expected_policy}),
        ) {
            return result.map(|id| json!({"pending":true,"command":id.to_string()}));
        }
        self.require_active_captain(captain)?;
        let current = self.captain_report_policy(captain);
        if current != expected_policy {
            return Err(format!(
                "Report policy changed since '{}'. Read again and confirm the new value.",
                expected_policy.id()
            ));
        }
        let database = crate::database::for_path(&self.data_dir.join("index.json"))
            .ok_or("This Captain change requires the durable server.")?;
        database.require_batch()?;
        if policy == ReportPolicy::React {
            self.captain_report_policies.remove(&captain);
        } else {
            self.captain_report_policies.insert(captain, policy);
        }
        self.refresh_captain_contexts();
        self.persistence_scope = None;
        Ok(
            json!({"captain":captain,"reportPolicy":policy.id(),"saved":true,
            "captainVersion":self.captain_version(captain)?}),
        )
    }

    /// Validates restored `captainReportPolicies` after threads, reports and
    /// lifecycle records exist. Keys name existing non-deleted Captains; the
    /// stored map never decides report validity, the per-record policy does.
    pub(super) fn restore_captain_report_policies(&mut self, index: &Value) -> Result<(), String> {
        let mut policies = std::collections::BTreeMap::new();
        if let Some(value) = index.get("captainReportPolicies") {
            let records = value
                .as_object()
                .ok_or("Invalid Captain report policies.")?;
            for (key, value) in records {
                let captain: usize = key
                    .parse()
                    .map_err(|_| "Invalid Captain report policy id.".to_string())?;
                if *key != captain.to_string() || captain >= self.captains.len() {
                    return Err("Invalid Captain report policy id.".into());
                }
                if self.captain_phase(captain) == CaptainPhase::Deleted {
                    return Err("A deleted Captain cannot keep a report policy.".into());
                }
                let policy: ReportPolicy = serde_json::from_value(value.clone())
                    .map_err(|_| "Invalid Captain report policy.".to_string())?;
                policies.insert(captain, policy);
            }
        }
        self.captain_report_policies = policies;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (State, crate::database::Database) {
        let directory = std::env::temp_dir().join(format!("shina-policy-{}", uuid::Uuid::new_v4()));
        let database = crate::database::Database::open(&directory).unwrap();
        let mut state = State::headless(directory);
        state.load_global_for_test();
        state.demo = true;
        state.captains = Captain::presets();
        state.captain_reports.clear();
        state.captain_homes.clear();
        state
            .threads
            .retain(|t| t.crew_captain.is_none() && !t.captain_home);
        state.active = 0;
        state
            .tabs
            .retain(|id| state.threads.iter().any(|t| t.id == *id));
        state.open_captain(0);
        database.begin().unwrap();
        (state, database)
    }

    fn crew_thread(state: &mut State) -> usize {
        let id = state.next_id;
        state.next_id += 1;
        let mut thread = Thread::new(id);
        thread.crew_captain = Some(0);
        thread.captain = Some(0);
        thread.title = "Crew worker".into();
        thread.run_id = Some("run-x".into());
        state.threads.push(thread);
        state.refresh_captain_contexts();
        id
    }

    fn index_of(state: &State, id: usize) -> usize {
        state.threads.iter().position(|t| t.id == id).unwrap()
    }

    fn thread(state: &mut State, id: usize) -> &mut Thread {
        state.threads.iter_mut().find(|t| t.id == id).unwrap()
    }

    fn home_index(state: &State) -> usize {
        index_of(state, state.captain_home(0).unwrap())
    }

    fn complete_turn(state: &mut State, id: usize, run: &str) {
        let thread = thread(state, id);
        thread.run_id = Some(run.into());
        thread.arm_task();
        thread.complete_task("done");
        state.poll_tasks(Instant::now());
    }

    fn report_injections(state: &State, index: usize, id: &str) -> usize {
        state.threads[index]
            .pending_injections
            .iter()
            .filter(|i| tasks::delivery_id(&i.text) == Some(id))
            .count()
    }

    #[test]
    fn legacy_records_default_to_react_and_malformed_maps_fail_closed() {
        let (mut state, _database) = fixture();
        let worker = crew_thread(&mut state);
        complete_turn(&mut state, worker, "run-1");
        assert_eq!(state.captain_report_policy(0), ReportPolicy::React);
        assert_eq!(state.captain_reports[0].policy, ReportPolicy::React);
        let mut legacy = state.snapshot();
        for report in legacy["captainReports"].as_array_mut().unwrap() {
            report.as_object_mut().unwrap().remove("policy");
        }
        legacy
            .as_object_mut()
            .unwrap()
            .remove("captainReportPolicies");
        let mut restored = fixture().0;
        restored.restore_global(&legacy).unwrap();
        assert_eq!(restored.captain_reports[0].policy, ReportPolicy::React);
        assert!(restored.captain_report_policies.is_empty());
        // A stored map round-trips and its keys may omit implicit react.
        state
            .set_captain_report_policy(0, ReportPolicy::Context, ReportPolicy::React)
            .unwrap();
        let snapshot = state.snapshot();
        assert_eq!(snapshot["captainReportPolicies"]["0"], json!("context"));
        let mut restored = fixture().0;
        restored.restore_global(&snapshot).unwrap();
        assert_eq!(restored.captain_report_policy(0), ReportPolicy::Context);
        for invalid in [
            json!("context"),
            json!(["context"]),
            json!(null),
            json!({"x":"ignore"}),
            json!({"00":"ignore"}),
            json!({"99":"ignore"}),
            json!({"0":"silence"}),
            json!({"0":5}),
            json!({"0":null}),
        ] {
            let mut broken = snapshot.clone();
            broken["captainReportPolicies"] = invalid.clone();
            assert!(fixture().0.restore_global(&broken).is_err(), "{invalid}");
        }
        // A deleted Captain cannot keep a policy in the durable map.
        let mut deleted = snapshot.clone();
        deleted["captainLifecycles"]["0"] = json!({"version":1,"phase":"deleted"});
        deleted["captainHomes"].as_object_mut().unwrap().remove("0");
        deleted["captains"][0]["name"] = json!("Deleted Captain 0");
        deleted["captains"][0]["role"] = json!("");
        deleted["captains"][0]["instructions"] = json!("");
        deleted["captains"][0]["portrait"] = json!(0);
        deleted["captainBooks"].as_object_mut().unwrap().remove("0");
        deleted["captainScopes"]
            .as_object_mut()
            .unwrap()
            .remove("0");
        deleted
            .as_object_mut()
            .unwrap()
            .remove("captainRemoteScopes");
        // Deletion requires report settlement before it releases the source.
        for report in deleted["captainReports"].as_array_mut().unwrap() {
            report["delivered"] = json!(true);
        }
        for record in deleted["threads"].as_array_mut().unwrap() {
            if record["captain"] == json!(0) {
                record["captain"] = Value::Null;
                record["crewCaptain"] = Value::Null;
                record["captainHome"] = json!(false);
            }
        }
        assert!(fixture().0.restore_global(&deleted).is_err());
        deleted["captainReportPolicies"]
            .as_object_mut()
            .unwrap()
            .remove("0");
        fixture().0.restore_global(&deleted).unwrap();
    }

    #[test]
    fn captured_reports_keep_their_policy_across_later_edits() {
        let (mut state, _database) = fixture();
        let worker = crew_thread(&mut state);
        complete_turn(&mut state, worker, "run-1");
        let first = state.captain_reports[0].id.clone();
        // The queued React report is not rewritten or retracted by the change.
        state
            .set_captain_report_policy(0, ReportPolicy::Ignore, ReportPolicy::React)
            .unwrap();
        assert_eq!(
            thread(&mut state, worker).crew_report_policy,
            ReportPolicy::Ignore
        );
        let home = home_index(&state);
        assert_eq!(report_injections(&state, home, &first), 1);
        // New captures snapshot Ignore; the outstanding one stays React.
        complete_turn(&mut state, worker, "run-2");
        let second = state.captain_reports[1].id.clone();
        assert_eq!(state.captain_reports[1].policy, ReportPolicy::Ignore);
        assert_eq!(state.captain_reports[0].policy, ReportPolicy::React);
        state.poll_tasks(Instant::now());
        assert_eq!(report_injections(&state, home, &second), 0);
        assert_eq!(report_injections(&state, home, &first), 1);
        // An explicit claim snapshots the accessor value too.
        state.demo = false;
        let saved = state
            .mcp_call(
                worker,
                "captain_crew_report",
                &json!({
                    "requestId":uuid::Uuid::new_v4().to_string(),
                    "outcome":"progress","summary":"half"
                }),
            )
            .unwrap();
        state.demo = true;
        let explicit = saved["reportId"].as_str().unwrap().to_owned();
        let stored = state
            .captain_reports
            .iter()
            .find(|r| r.id == explicit)
            .unwrap();
        assert_eq!(stored.policy, ReportPolicy::Ignore);
        state
            .set_captain_report_policy(0, ReportPolicy::React, ReportPolicy::Ignore)
            .unwrap();
        let stored = state
            .captain_reports
            .iter()
            .find(|r| r.id == explicit)
            .unwrap();
        assert_eq!(stored.policy, ReportPolicy::Ignore);
    }

    #[test]
    fn ignore_reports_never_inject_wake_or_block_membership() {
        let (mut state, _database) = fixture();
        state
            .set_captain_report_policy(0, ReportPolicy::Ignore, ReportPolicy::React)
            .unwrap();
        let worker = crew_thread(&mut state);
        complete_turn(&mut state, worker, "run-1");
        assert_eq!(state.captain_reports.len(), 1);
        let id = state.captain_reports[0].id.clone();
        let home = home_index(&state);
        assert_eq!(report_injections(&state, home, &id), 0);
        assert!(state.threads[home].pi.is_none());
        for _ in 0..3 {
            assert!(!state.poll_crew_reports());
        }
        assert!(!state.captain_reports[0].delivered);
        // An undelivered Ignore report does not pin the source's membership.
        let worker_index = index_of(&state, worker);
        assert!(state.crew_idle(worker_index).is_ok());
        // A stale queued copy is scrubbed, never flushed.
        let text = state.captain_reports[0].message();
        state.threads[home].pending_injections.push_back(Injection {
            text,
            shown: false,
            steer: None,
            peer_input: None,
        });
        assert!(state.scrub_crew_report_injections());
        assert_eq!(report_injections(&state, home, &id), 0);
        // Releasing holds cannot resurrect it either.
        let home_id = state.captain_home(0).unwrap();
        state.release_task_results(home_id);
        state.poll_tasks(Instant::now());
        assert_eq!(report_injections(&state, home, &id), 0);
        // The record survives release and reopen still undelivered.
        let snapshot = state.snapshot();
        let mut moved = snapshot.clone();
        for record in moved["threads"].as_array_mut().unwrap() {
            if record["id"] == json!(worker) {
                record["crewCaptain"] = Value::Null;
                record["captain"] = Value::Null;
            }
        }
        let mut restored = fixture().0;
        restored.restore_global(&moved).unwrap();
        restored.prepare_task_recovery();
        assert_eq!(restored.captain_reports.len(), 1);
        assert!(!restored.captain_reports[0].delivered);
        assert!(!restored.captain_reports[0].hold_delivery);
        restored.poll_tasks(Instant::now());
        let home = home_index(&restored);
        assert_eq!(report_injections(&restored, home, &id), 0);
    }

    #[test]
    fn context_reports_hold_until_an_explicit_turn_and_block_settlement() {
        let (mut state, _database) = fixture();
        state
            .set_captain_report_policy(0, ReportPolicy::Context, ReportPolicy::React)
            .unwrap();
        let worker = crew_thread(&mut state);
        complete_turn(&mut state, worker, "run-1");
        assert_eq!(state.captain_reports.len(), 1);
        let id = state.captain_reports[0].id.clone();
        assert!(state.captain_reports[0].hold_delivery);
        let home = home_index(&state);
        assert_eq!(report_injections(&state, home, &id), 0);
        // Held reports pin membership and the Captain's settlement checks.
        let worker_index = index_of(&state, worker);
        assert!(state.crew_idle(worker_index).is_err());
        assert!(!state.captain_settlement_blockers(0).is_empty());
        // A quiet poll still injects nothing.
        assert!(!state.poll_crew_reports());
        // The explicit user turn releases the hold; delivery then settles it.
        let home_id = state.captain_home(0).unwrap();
        state.release_task_results(home_id);
        assert!(!state.captain_reports[0].hold_delivery);
        state.poll_tasks(Instant::now());
        assert_eq!(report_injections(&state, home, &id), 1);
        state.threads[home].accepted_deliveries.push(id.clone());
        assert!(state.reconcile_crew_report_acceptances());
        assert!(state.crew_idle(index_of(&state, worker)).is_ok());
        // Across a restart an undelivered Context report holds again.
        let (mut state, _database) = fixture();
        state
            .set_captain_report_policy(0, ReportPolicy::Context, ReportPolicy::React)
            .unwrap();
        let worker = crew_thread(&mut state);
        complete_turn(&mut state, worker, "run-1");
        let snapshot = state.snapshot();
        let mut restored = fixture().0;
        restored.restore_global(&snapshot).unwrap();
        restored.prepare_task_recovery();
        assert!(restored.captain_reports[0].hold_delivery);
        restored.poll_tasks(Instant::now());
        let home = home_index(&restored);
        assert!(restored.threads[home].pending_injections.is_empty());
    }

    #[test]
    fn report_policy_cas_failures_never_mutate_and_need_an_active_captain() {
        let (mut state, _database) = fixture();
        let error = state
            .set_captain_report_policy(0, ReportPolicy::Ignore, ReportPolicy::Context)
            .unwrap_err();
        assert!(error.contains("changed since 'context'"), "{error}");
        assert!(state.captain_report_policies.is_empty());
        // Unknown and non-active Captains reject before the CAS check lands.
        assert!(
            state
                .set_captain_report_policy(99, ReportPolicy::Ignore, ReportPolicy::React)
                .is_err()
        );
        assert!(
            state
                .set_captain_report_policy(0, ReportPolicy::Context, ReportPolicy::React)
                .is_ok()
        );
        // React is the implicit default: setting it removes the stored entry.
        state
            .set_captain_report_policy(0, ReportPolicy::React, ReportPolicy::Context)
            .unwrap();
        assert!(state.captain_report_policies.is_empty());
        // Without a durable batch the change refuses before mutating.
        let (mut no_batch, _db) = {
            let (state, database) = fixture();
            drop(database);
            let directory = state.data_dir.clone();
            drop(state);
            let database = crate::database::Database::open(&directory).unwrap();
            let mut state = State::headless(directory);
            state.demo = true;
            state.captains = Captain::presets();
            state.open_captain(0);
            (state, database)
        };
        assert!(
            no_batch
                .set_captain_report_policy(0, ReportPolicy::Ignore, ReportPolicy::React)
                .is_err()
        );
        assert!(no_batch.captain_report_policies.is_empty());
    }

    #[test]
    fn open_questions_block_membership_under_every_policy() {
        for policy in [
            ReportPolicy::React,
            ReportPolicy::Context,
            ReportPolicy::Ignore,
        ] {
            let (mut state, _database) = fixture();
            state
                .set_captain_report_policy(0, policy, ReportPolicy::React)
                .unwrap();
            let worker = crew_thread(&mut state);
            // MCP mutations intentionally refuse demo mode. The open database
            // batch keeps explicit claims inside the durable command path.
            state.demo = false;
            state
                .mcp_call(
                    worker,
                    "captain_crew_report",
                    &json!({
                        "requestId":uuid::Uuid::new_v4().to_string(),
                        "outcome":"blocked","summary":"waiting",
                        "question":{"text":"Which branch?","recipient":"captain"}
                    }),
                )
                .unwrap();
            let report_id = state.captain_reports[0].id.clone();
            assert_eq!(state.captain_reports[0].policy, policy);
            // Settle deliverable reports with acceptance evidence. Ignore stays
            // undelivered. Either way, the unanswered question still blocks.
            if policy != ReportPolicy::Ignore {
                let home = home_index(&state);
                state.threads[home]
                    .accepted_deliveries
                    .push(report_id.clone());
                assert!(state.reconcile_crew_report_acceptances());
            }
            let worker_index = index_of(&state, worker);
            assert!(state.crew_idle(worker_index).is_err(), "{policy:?}");
            assert!(
                !state.captain_settlement_blockers(0).is_empty(),
                "{policy:?}"
            );
            // A saved-but-undelivered answer keeps blocking.
            state
                .mcp_call(
                    state.captain_home(0).unwrap(),
                    "captain_question_answer",
                    &json!({
                        "reportId":report_id,"expectedRevision":0,
                        "requestId":uuid::Uuid::new_v4().to_string(),"answer":"main"
                    }),
                )
                .unwrap();
            assert!(state.crew_idle(worker_index).is_err(), "{policy:?}");
            state.captain_question_answers[0].delivered = true;
            assert!(state.crew_idle(worker_index).is_ok(), "{policy:?}");
        }
    }
}
