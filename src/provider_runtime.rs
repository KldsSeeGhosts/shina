//! Provider mutations commit their command receipt before any installer can start.
use super::*;

pub(super) struct ProviderStart {
    pub driver: String,
    pub target: String,
    pub id: String,
    fences: Vec<crate::backend::ShutdownFence>,
    started: std::time::Instant,
    requested_at: u64,
}

impl RuntimeState {
    pub(super) fn provider_statuses(&self) -> Value {
        let mut statuses = json!(self.providers.statuses());
        // An accepted intent may still be waiting for idle process cleanup.
        for pending in &self.provider_starts {
            if let Some(status) = statuses.as_array_mut().and_then(|statuses| {
                statuses
                    .iter_mut()
                    .find(|status| status["driver"] == pending.driver)
            }) {
                status["update"] = json!({
                    "updateId":pending.id,"status":"queued",
                    "message":"Waiting for the provider's idle processes to exit.",
                    "startedAt":pending.requested_at,"finishedAt":null,
                    "fromVersion":status["version"],"targetVersion":pending.target
                });
            }
        }
        statuses
    }

    pub(super) fn validate_provider_update(&self, params: &Value) -> Result<Value, String> {
        let args = params
            .as_object()
            .ok_or("Provider arguments must be an object.")?;
        if args
            .keys()
            .any(|key| !matches!(key.as_str(), "driver" | "targetVersion"))
        {
            return Err("Only driver and targetVersion are accepted.".into());
        }
        let driver = params["driver"].as_str().ok_or("driver must be text.")?;
        let target = params["targetVersion"]
            .as_str()
            .ok_or("targetVersion must be text.")?;
        if self.maintenance.fenced() || self.validation_hold || self.updates.active() {
            return Err(
                "Server is draining for an update. Retry after restart or cancellation.".into(),
            );
        }
        if let Some(blocker) = self.state.provider_work_blocker() {
            return Err(blocker.into());
        }
        if self
            .provider_starts
            .iter()
            .any(|pending| pending.driver == driver)
        {
            return Err("This provider is already updating.".into());
        }
        if !self.pending_effects.is_empty() {
            return Err(
                "Waiting for committed harness effects to settle. Retry after they finish.".into(),
            );
        }
        self.providers.validate_request(driver, target)?;
        Ok(json!({
            "updateAccepted":true,"updateId":uuid::Uuid::new_v4(),
            "providerUpdate":true,"driver":driver,"targetVersion":target
        }))
    }

    pub(super) fn start_provider_update(&mut self, result: &Value) {
        let driver = result["driver"]
            .as_str()
            .expect("validated driver")
            .to_owned();
        let fences = self.state.hold_provider(&driver);
        self.provider_starts.push(ProviderStart {
            driver,
            target: result["targetVersion"]
                .as_str()
                .expect("validated target")
                .into(),
            id: result["updateId"].as_str().expect("accepted id").into(),
            fences,
            started: std::time::Instant::now(),
            requested_at: crate::lifecycle::now(),
        });
    }

    pub(super) fn poll_provider_updates(&mut self) -> Result<bool, String> {
        let mut changed = self.providers.poll();
        if let Some(error) = self.providers.failure()
            && (!self.provider_starts.is_empty() || !self.state.provider_update_drivers.is_empty())
        {
            // An accepted update cannot release its launch gate on an unsaved outcome.
            return Err(error);
        }
        let mut waiting = Vec::new();
        let mut write_error = None;
        for pending in std::mem::take(&mut self.provider_starts) {
            if write_error.is_some() {
                waiting.push(pending);
                continue;
            }
            let ready = pending
                .fences
                .iter()
                .try_fold(true, |ready, fence| fence.ready().map(|next| ready && next));
            let failure = match ready {
                Err(error) => Some(error),
                Ok(false) if pending.started.elapsed() < Duration::from_secs(15) => {
                    waiting.push(pending);
                    continue;
                }
                Ok(false) => Some("The previous provider process did not exit. No update command ran. Restart Shina before retrying.".into()),
                Ok(true) => self.providers
                    .request(&pending.driver, &pending.target, pending.id.clone())
                    .err(),
            };
            if let Some(error) = failure {
                if let Err(error) = self.providers.record_failure(
                    &pending.driver,
                    &pending.target,
                    pending.id.clone(),
                    error,
                ) {
                    write_error = Some(error);
                    waiting.push(pending);
                }
            }
            changed = true;
        }
        self.provider_starts = waiting;
        if let Some(error) = write_error {
            return Err(error);
        }
        let held: Vec<_> = self.state.provider_update_drivers.iter().cloned().collect();
        for driver in held {
            if !self
                .provider_starts
                .iter()
                .any(|pending| pending.driver == driver)
                && self.providers.admits(&driver)
            {
                self.state.release_provider(&driver);
                changed = true;
            }
        }
        Ok(changed)
    }

    pub(super) fn reconcile_provider_intents(&mut self) -> Result<(), String> {
        let Some(database) = &self.database else {
            return Ok(());
        };
        let intents = database.provider_update_intents()?;
        if self.providers.failure().is_some() {
            // Quarantine this optional updater, not the server's conversations.
            return Ok(());
        }
        let statuses = json!(self.providers.statuses());
        for intent in intents {
            let (Some(driver), Some(id), Some(target)) = (
                intent["driver"].as_str(),
                intent["updateId"].as_str(),
                intent["targetVersion"].as_str(),
            ) else {
                self.state.recovery_notice = Some("A provider update receipt was incomplete. Check installed versions before updating again.".into());
                continue;
            };
            let found = statuses.as_array().is_some_and(|statuses| {
                statuses
                    .iter()
                    .any(|status| status["driver"] == driver && status["update"]["updateId"] == id)
            });
            if !found {
                if let Err(error) = self.providers.record_failure(
                    driver,
                    target,
                    id.into(),
                    "Shina stopped before this provider update could start. Refresh and retry explicitly.".into(),
                ) {
                    self.state.recovery_notice = Some(error);
                    break;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    fn runtime() -> RuntimeState {
        let runtime = RuntimeState::open(
            crate::store::directory().unwrap(),
            std::sync::Arc::new(|| {}),
        )
        .unwrap();
        runtime
            .database
            .as_ref()
            .unwrap()
            .issue_scoped_device("updater", "fixture-token", &["update"])
            .unwrap();
        runtime
    }

    #[cfg(unix)]
    fn ready_runtime() -> (RuntimeState, Command, Arc<AtomicBool>) {
        use crate::provider_update::{CommandResult, Driver, Hooks, ProbeResult, testkit};
        let mut runtime = runtime();
        let fixture = testkit::Fixture::new();
        let cli = fixture.npm_global("provider-fixture", "codex", "@openai/codex");
        let prefix = fixture
            .dir
            .join("provider-fixture")
            .to_string_lossy()
            .into_owned();
        let installed = Arc::new(AtomicBool::new(false));
        let version = installed.clone();
        let prober = testkit::FakeProber::new(move |argv| {
            let stdout = if argv.ends_with(&["prefix".into(), "-g".into()]) {
                prefix.clone()
            } else if argv.contains(&"outdated".into()) {
                r#"{"@openai/codex":{"latest":"1.1.0"}}"#.into()
            } else if version.load(Ordering::Acquire) {
                "codex-cli 1.1.0".into()
            } else {
                "codex-cli 1.0.0".into()
            };
            ProbeResult {
                exit_code: Some(0),
                stdout,
                stderr: String::new(),
            }
        });
        let mut providers = testkit::service(
            &runtime.state.data_dir,
            std::collections::HashMap::from([(Driver::Codex, vec![cli])]),
            prober,
            testkit::FakeRunner::script(vec![testkit::QueuedRun {
                result: CommandResult {
                    exit_code: Some(0),
                    ..Default::default()
                },
                delay: Duration::ZERO,
            }]),
        );
        let command = Command::new(
            "providers.update",
            json!({"driver":"codex","targetVersion":"1.1.0"}),
        );
        let id = command.command_id;
        let database = runtime.database.clone().unwrap();
        let observed = installed.clone();
        providers.set_hooks(Hooks {
            busy: Arc::new(|_| false),
            before_run: Arc::new(move |_| {
                let receipt = database
                    .receipt("updater", id)
                    .unwrap()
                    .expect("receipt must commit before install");
                assert!(matches!(receipt.status, ReceiptStatus::Accepted));
                observed.store(true, Ordering::Release);
            }),
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            providers.poll();
            if providers.validate_request("codex", "1.1.0").is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "fixture discovery did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        runtime.providers = providers;
        (runtime, command, installed)
    }

    #[cfg(unix)]
    #[test]
    fn accepted_provider_receipt_commits_before_the_installer_and_replays_once() {
        let (mut runtime, command, installed) = ready_runtime();
        let accepted = runtime.command("updater", command.clone()).unwrap();
        assert_eq!(accepted["status"], "accepted");
        assert!(!installed.load(Ordering::Acquire));
        assert!(runtime.state.provider_update_drivers.contains("codex"));
        assert_eq!(
            runtime.command("updater", command.clone()).unwrap(),
            accepted
        );
        assert_eq!(runtime.provider_starts.len(), 1);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !runtime.provider_starts.is_empty() || runtime.providers.active() {
            runtime.poll_provider_updates().unwrap();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(installed.load(Ordering::Acquire));
        let statuses = runtime.provider_statuses();
        assert_eq!(
            statuses[0]["update"]["updateId"],
            accepted["result"]["updateId"]
        );
        assert_eq!(statuses[0]["update"]["status"], "succeeded");
        assert!(!runtime.state.provider_update_drivers.contains("codex"));
        assert_eq!(runtime.command("updater", command).unwrap(), accepted);
        runtime.shutdown().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_provider_receipt_commit_failure_never_starts_an_installer() {
        let (mut runtime, command, installed) = ready_runtime();
        let database = runtime.database.clone().unwrap();
        database.test_connection(|connection| {
            connection
                .execute_batch(
                    "CREATE TRIGGER fail_provider_receipt BEFORE INSERT ON command_receipts
                 WHEN json_extract(NEW.payload,'$.result.providerUpdate')=1
                 BEGIN SELECT RAISE(ABORT,'injected provider receipt commit failure'); END;",
                )
                .unwrap();
        });
        assert!(runtime.command("updater", command).is_err());
        assert!(!installed.load(Ordering::Acquire));
        assert!(runtime.provider_starts.is_empty());
        assert!(!runtime.providers.active());
        assert!(runtime.state.provider_update_drivers.is_empty());
        assert!(runtime.failure().is_some());
    }

    #[test]
    fn provider_update_rejections_have_replayable_receipts_and_send_nothing() {
        let mut runtime = runtime();
        let command = Command::new(
            "providers.update",
            json!({"driver":"unknown","targetVersion":"1.0.0"}),
        );
        let rejected = runtime.command("updater", command.clone()).unwrap();
        assert_eq!(rejected["status"], "rejected");
        assert_eq!(
            runtime.command("updater", command.clone()).unwrap(),
            rejected
        );
        let mut conflicting = command;
        conflicting.params["targetVersion"] = json!("2.0.0");
        assert!(
            runtime
                .command("updater", conflicting)
                .unwrap_err()
                .contains("different payload")
        );
        assert!(runtime.provider_starts.is_empty());
        assert!(runtime.state.provider_update_drivers.is_empty());
        assert!(!runtime.providers.active());
        runtime.shutdown().unwrap();
    }

    #[test]
    fn provider_refresh_is_update_authorized_and_has_no_command_receipt() {
        let mut runtime = runtime();
        runtime
            .database
            .as_ref()
            .unwrap()
            .issue_scoped_device("reader", "fixture-reader", &["read"])
            .unwrap();
        let command = Command::new("providers.refresh", json!({}));
        assert!(runtime.command("reader", command.clone()).is_err());
        let value = runtime.command("updater", command.clone()).unwrap();
        assert!(value["providers"].is_array());
        assert!(
            runtime
                .database
                .as_ref()
                .unwrap()
                .receipt("updater", command.command_id)
                .unwrap()
                .is_none()
        );
        assert!(
            runtime
                .command(
                    "updater",
                    Command::new("providers.refresh", json!({"command":"anything"}))
                )
                .is_err()
        );
        runtime.shutdown().unwrap();
    }

    #[test]
    fn server_provider_and_running_work_blockers_apply_before_any_install() {
        let mut runtime = runtime();
        let params = json!({"driver":"codex","targetVersion":"1.0.0"});
        runtime.updates.phase = "downloading".into();
        let value = runtime
            .command("updater", Command::new("providers.update", params.clone()))
            .unwrap();
        assert_eq!(value["status"], "rejected");
        assert!(
            value["result"]["error"]
                .as_str()
                .unwrap()
                .contains("draining")
        );
        runtime.updates.phase = "idle".into();
        runtime.state.threads[0].status = crate::model::Status::Running;
        let value = runtime
            .command("updater", Command::new("providers.update", params))
            .unwrap();
        assert_eq!(value["status"], "rejected");
        assert!(
            value["result"]["error"]
                .as_str()
                .unwrap()
                .contains("thread")
        );
        runtime.state.threads[0].status = crate::model::Status::Ready;
        runtime.state.hold_provider("codex");
        let value = runtime
            .command("updater", Command::new("server.requestUpdate", json!({})))
            .unwrap();
        assert_eq!(value["status"], "rejected");
        assert!(
            value["result"]["error"]
                .as_str()
                .unwrap()
                .contains("provider")
        );
        assert!(runtime.provider_starts.is_empty());
        runtime.state.release_provider("codex");
        runtime.shutdown().unwrap();
    }

    #[test]
    fn committed_intent_without_a_runner_becomes_failure_and_is_never_reissued() {
        let directory = crate::store::directory().unwrap();
        let command = Command::new(
            "providers.update",
            json!({"driver":"codex","targetVersion":"999.0.0"}),
        );
        let id = uuid::Uuid::new_v4().to_string();
        let receipt = {
            let mut runtime = runtime();
            let receipt = Receipt {
                command_id: command.command_id,
                device_id: "updater".into(),
                hash: command.hash(),
                status: ReceiptStatus::Accepted,
                seq: runtime.seq + 1,
                result: json!({"providerUpdate":true,"updateAccepted":true,"driver":"codex","targetVersion":"999.0.0","updateId":id}),
            };
            let database = runtime.database.clone().unwrap();
            database.begin().unwrap();
            runtime.state.record_command(receipt.clone(), None);
            runtime.persist(Some(&receipt), Vec::new()).unwrap();
            // Simulate the commit-to-runner crash gap. No installer was requested.
            runtime.shutdown().unwrap();
            json!(receipt)
        };
        let mut runtime = RuntimeState::open(directory, std::sync::Arc::new(|| {})).unwrap();
        assert_eq!(runtime.command("updater", command).unwrap(), receipt);
        let statuses = runtime.provider_statuses();
        let status = statuses
            .as_array()
            .unwrap()
            .iter()
            .find(|status| status["driver"] == "codex")
            .unwrap();
        assert_eq!(status["update"]["updateId"], id);
        assert_eq!(status["update"]["status"], "failed");
        assert!(
            status["update"]["message"]
                .as_str()
                .unwrap()
                .contains("before")
        );
        assert!(runtime.provider_starts.is_empty());
        assert!(!runtime.providers.active());
        runtime.shutdown().unwrap();
    }
}
