//! Tests for `provider_update` (contract section 11, provider-module rows).
//! Fakes own every process and network effect; nothing here touches a real
//! install, PATH tool, CPA path, or the user's profile.

use super::*;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

fn prober(run: impl Fn(&[String]) -> ProbeResult + Send + Sync + 'static) -> testkit::FakeProber {
    testkit::FakeProber::new(run)
}

/// A prober for an npm-global Claude install: installed version is 2.0.5 until
/// `updated` flips (the fake runner does that), latest is always 2.1.0.
fn update_prober(prefix: String, updated: Arc<AtomicBool>) -> testkit::FakeProber {
    prober(move |argv| {
        if argv.ends_with(&["prefix".into(), "-g".into()]) {
            ProbeResult {
                exit_code: Some(0),
                stdout: format!("{prefix}\n"),
                ..Default::default()
            }
        } else if argv.contains(&"outdated".into()) {
            ProbeResult {
                exit_code: Some(1),
                stdout: r#"{"@anthropic-ai/claude-code":{"latest":"2.1.0"}}"#.into(),
                ..Default::default()
            }
        } else {
            let version = if updated.load(Ordering::Relaxed) {
                "2.1.0"
            } else {
                "2.0.5"
            };
            ProbeResult {
                exit_code: Some(0),
                stdout: version.into(),
                ..Default::default()
            }
        }
    })
}

/// A scripted runner that flips `updated` once the update command starts.
struct FlipRunner {
    updated: Arc<AtomicBool>,
    script: Vec<testkit::QueuedRun>,
}
impl Runner for FlipRunner {
    fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
        let run = testkit::FakeRunner::script(self.script.clone());
        let handle = run.start(argv)?;
        self.updated.store(true, Ordering::Relaxed);
        Ok(handle)
    }
}

fn ok_update() -> testkit::QueuedRun {
    testkit::QueuedRun {
        result: CommandResult {
            exit_code: Some(0),
            stdout_tail: "done".into(),
            stderr_tail: String::new(),
        },
        delay: Duration::ZERO,
    }
}

fn no_run() -> testkit::FakeRunner {
    testkit::FakeRunner::script(vec![])
}

fn status(service: &ProviderUpdates, driver: &str) -> ProviderStatus {
    service
        .statuses()
        .into_iter()
        .find(|s| s.driver == driver)
        .expect("provider status")
}

fn update_record(service: &ProviderUpdates, driver: &str) -> UpdateRecord {
    status(service, driver).update.expect("an update record")
}

fn detected(service: &mut ProviderUpdates) {
    testkit::settle(service, Duration::from_secs(5));
}

// ---------- detection and strategies ----------

#[test]
fn npm_global_install_proves_the_prefix_strategy() {
    let fixture = testkit::Fixture::new();
    let claude = fixture.npm_global("claude-npm", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture.dir.join("claude-npm");
    let prefix_s = prefix.to_string_lossy().into_owned();
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        prober(move |argv| {
            if argv.ends_with(&["prefix".into(), "-g".into()]) {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: format!("{prefix_s}\n"),
                    ..Default::default()
                }
            } else if argv.contains(&"outdated".into()) {
                ProbeResult {
                    exit_code: Some(1),
                    stdout: r#"{"@anthropic-ai/claude-code":{"latest":"2.1.0"}}"#.into(),
                    ..Default::default()
                }
            } else {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: "2.0.5 (Claude Code)".into(),
                    ..Default::default()
                }
            }
        }),
        no_run(),
    );
    detected(&mut service);
    let status = status(&service, "claude");
    assert!(status.installed);
    assert_eq!(status.strategy.as_deref(), Some("npm-global"));
    assert_eq!(status.version.as_deref(), Some("2.0.5"));
    assert_eq!(status.latest_version.as_deref(), Some("2.1.0"));
    assert_eq!(status.advisory, "behind");
}

#[test]
fn homebrew_keg_proves_with_its_own_brew() {
    let fixture = testkit::Fixture::new();
    let brew = fixture.native("brew/bin/brew");
    let codex = fixture.native("brew/Cellar/codex/0.48.0/bin/codex");
    let prefix = fixture.dir.join("brew").to_string_lossy().into_owned();
    let mut overrides = Overrides::default();
    overrides.candidates.insert(Driver::Codex, vec![codex]);
    overrides.tools.insert("brew".into(), PathBuf::from(&brew));
    let mut service = ProviderUpdates::build(
        &fixture.dir,
        Hooks::default(),
        Arc::new(prober(move |argv| {
            if argv.ends_with(&["--prefix".into()]) && argv[0].contains("brew") {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: format!("{prefix}\n"),
                    ..Default::default()
                }
            } else if argv.contains(&"info".into()) {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: r#"{"formulae":[{"versions":{"stable":"0.50.0"}}]}"#.into(),
                    ..Default::default()
                }
            } else {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: "codex 0.48.0".into(),
                    ..Default::default()
                }
            }
        })),
        Arc::new(no_run()),
        Some(overrides),
        Duration::from_secs(30),
    );
    detected(&mut service);
    let status = status(&service, "codex");
    assert_eq!(status.strategy.as_deref(), Some("homebrew-formula"));
    assert_eq!(status.latest_version.as_deref(), Some("0.50.0"));
}

#[test]
fn claude_native_versions_dir_uses_the_cli_update() {
    let fixture = testkit::Fixture::new();
    // The strategy matcher keys on the versions/ path segment, not HOME.
    let claude = fixture.native("home/.local/share/claude/versions/2.0.4/claude");
    let mut prober = prober(|argv| {
        if argv.last().is_some_and(|a| a == "--version") {
            ProbeResult {
                exit_code: Some(0),
                stdout: "2.0.4".into(),
                ..Default::default()
            }
        } else {
            ProbeResult::default()
        }
    });
    prober.http = Some("2.1.3".into());
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        prober,
        no_run(),
    );
    detected(&mut service);
    let status = status(&service, "claude");
    assert_eq!(status.strategy.as_deref(), Some("claude-native"));
    assert_eq!(status.latest_version.as_deref(), Some("2.1.3"));
}

#[test]
fn env_override_and_wrappers_are_manual() {
    let fixture = testkit::Fixture::new();
    // A wrapper script: the resolved file is its own real path and starts with #!.
    let wrapper = fixture.wrapper("wrapped/claude");
    let prober = prober(|_| ProbeResult::default());
    let detected = detect(
        Driver::Claude,
        &prober,
        Some(&Overrides {
            candidates: HashMap::from([(Driver::Claude, vec![wrapper])]),
            ..Default::default()
        }),
    );
    assert!(detected.strategy.is_none());
    assert!(detected.disabled_reason.unwrap().contains("wrapper"));
}

#[test]
fn unproven_managers_report_honest_reasons() {
    let fixture = testkit::Fixture::new();
    for (path, expected) in [
        ("mise/installs/claude/2.0/claude", "mise"),
        ("nix-store/pnpm/global/claude", "pnpm"),
        ("bun/.bun/install/global/claude", "bun"),
    ] {
        let binary = fixture.native(&format!("x/{path}"));
        let mut overrides = Overrides::default();
        overrides.candidates.insert(Driver::Claude, vec![binary]);
        let detected = detect(
            Driver::Claude,
            &prober(|_| ProbeResult::default()),
            Some(&overrides),
        );
        // Fixture paths live under the temp dir, so only the manager matchers
        // that appear inside the path can fire; project layouts still refuse.
        assert!(detected.strategy.is_none(), "{path}");
        let reason = detected.disabled_reason.unwrap_or_default();
        assert!(
            reason.contains(expected) || reason.contains("installed"),
            "{path}: {reason}"
        );
    }
}

// ---------- request validation (pre-commit) ----------

fn claude_npm_service(fixture: &testkit::Fixture) -> ProviderUpdates {
    let claude = fixture.npm_global("claude-npm", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("claude-npm")
        .to_string_lossy()
        .into_owned();
    testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        prober(move |argv| {
            if argv.ends_with(&["prefix".into(), "-g".into()]) {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: format!("{prefix}\n"),
                    ..Default::default()
                }
            } else if argv.contains(&"outdated".into()) {
                ProbeResult {
                    exit_code: Some(1),
                    stdout: r#"{"@anthropic-ai/claude-code":{"latest":"2.1.0"}}"#.into(),
                    ..Default::default()
                }
            } else {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: "2.0.5".into(),
                    ..Default::default()
                }
            }
        }),
        no_run(),
    )
}

#[test]
fn validate_request_is_pure_and_honest() {
    let fixture = testkit::Fixture::new();
    let mut service = claude_npm_service(&fixture);
    detected(&mut service);
    assert_eq!(
        service.validate_request("codex", "2.1.0").unwrap_err(),
        "Codex is not installed."
    );
    assert_eq!(
        service.validate_request("claude", "9.9.9").unwrap_err(),
        "Refresh and try again."
    );
    assert!(service.validate_request("claude", "2.1.0").is_ok());
    // Nothing was written and nothing was queued.
    assert!(!service.active());
    assert!(status(&service, "claude").update.is_none());
    assert!(!fixture.dir.join("providers/update-state.json").exists());
}

#[test]
fn unsupported_install_rejects_with_its_reason() {
    let fixture = testkit::Fixture::new();
    let claude = fixture.wrapper("wrapped/claude");
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        prober(|_| ProbeResult {
            exit_code: Some(0),
            stdout: "2.0.5".into(),
            ..Default::default()
        }),
        no_run(),
    );
    detected(&mut service);
    let error = service.validate_request("claude", "2.1.0").unwrap_err();
    assert!(error.contains("wrapper"), "{error}");
    assert_eq!(
        service.request("claude", "2.1.0", uuid::Uuid::new_v4().to_string()),
        Err(error)
    );
}

// ---------- execution ----------

/// A runner that records whether the durable ledger already held a
/// queued/running record when `start` was called.
struct DurableGuard {
    ledger: PathBuf,
    seen: Arc<Mutex<bool>>,
    calls: Arc<Mutex<usize>>,
    updated: Arc<AtomicBool>,
}
impl Runner for DurableGuard {
    fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
        *self.calls.lock().unwrap() += 1;
        let text = std::fs::read_to_string(&self.ledger).unwrap_or_default();
        if text.contains("\"queued\"") || text.contains("\"running\"") {
            *self.seen.lock().unwrap() = true;
        }
        let handle = testkit::FakeRunner::script(vec![ok_update()]).start(argv)?;
        self.updated.store(true, Ordering::Relaxed);
        Ok(handle)
    }
}

#[test]
fn update_runs_durable_first_and_verifies() {
    let fixture = testkit::Fixture::new();
    let claude = fixture.npm_global("claude-npm", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("claude-npm")
        .to_string_lossy()
        .into_owned();
    let ledger = fixture.dir.join("providers/update-state.json");
    let durable_seen = Arc::new(Mutex::new(false));
    let calls = Arc::new(Mutex::new(0usize));
    let updated = Arc::new(AtomicBool::new(false));
    let runner = DurableGuard {
        ledger,
        seen: Arc::clone(&durable_seen),
        calls: Arc::clone(&calls),
        updated: Arc::clone(&updated),
    };
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        update_prober(prefix, updated),
        runner,
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .expect("accepted");
    assert!(service.active());
    assert!(!service.admits("claude"));
    assert!(service.admits("codex"));
    testkit::settle(&mut service, Duration::from_secs(5));
    let record = update_record(&service, "claude");
    assert_eq!(record.status, "succeeded");
    assert_eq!(record.message, "Claude Code updated: v2.1.0");
    assert_eq!(*calls.lock().unwrap(), 1);
    assert!(*durable_seen.lock().unwrap());
    assert!(!service.active());
    assert!(service.admits("claude"));
}

#[test]
fn replay_same_id_never_reruns_and_conflict_is_refused() {
    let fixture = testkit::Fixture::new();
    let runner_calls = Arc::new(Mutex::new(0usize));
    let claude = fixture.npm_global("claude-npm2", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("claude-npm2")
        .to_string_lossy()
        .into_owned();
    let calls = Arc::clone(&runner_calls);
    let updated = Arc::new(AtomicBool::new(false));
    struct Counting(Arc<Mutex<usize>>, Arc<AtomicBool>);
    impl Runner for Counting {
        fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
            *self.0.lock().unwrap() += 1;
            let handle = testkit::FakeRunner::script(vec![ok_update()]).start(argv)?;
            self.1.store(true, Ordering::Relaxed);
            Ok(handle)
        }
    }
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        update_prober(prefix, Arc::clone(&updated)),
        Counting(calls, updated),
    );
    detected(&mut service);
    let id = uuid::Uuid::new_v4().to_string();
    service.request("claude", "2.1.0", id.clone()).unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    assert_eq!(*runner_calls.lock().unwrap(), 1);
    // Same id, same payload: stored receipt replays, no second run.
    service.request("claude", "2.1.0", id.clone()).unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    assert_eq!(*runner_calls.lock().unwrap(), 1);
    // Same id, different payload: refused.
    assert!(service.request("claude", "9.9.9", id).is_err());
}

#[test]
fn busy_gate_holds_until_idle_and_admits_blocks_new_work() {
    let fixture = testkit::Fixture::new();
    let claude = fixture.npm_global("claude-npm", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("claude-npm")
        .to_string_lossy()
        .into_owned();
    let busy = Arc::new(AtomicBool::new(true));
    let busy_hook = {
        let busy = Arc::clone(&busy);
        Arc::new(move |_: &str| busy.load(Ordering::Relaxed))
            as Arc<dyn Fn(&str) -> bool + Send + Sync>
    };
    let updated = Arc::new(AtomicBool::new(false));
    let mut service = ProviderUpdates::build(
        &fixture.dir,
        Hooks {
            busy: busy_hook,
            before_run: Arc::new(|_| {}),
        },
        Arc::new(update_prober(prefix, Arc::clone(&updated))),
        Arc::new(FlipRunner {
            updated,
            script: vec![ok_update()],
        }),
        Some(Overrides {
            candidates: HashMap::from([(Driver::Claude, vec![claude]), (Driver::Codex, vec![])]),
            ..Default::default()
        }),
        Duration::from_secs(30),
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    for _ in 0..50 {
        service.poll();
        thread::sleep(Duration::from_millis(2));
        let message = update_record(&service, "claude").message;
        if message.contains("Waiting for running") {
            break;
        }
    }
    assert!(!service.admits("claude"));
    // Still waiting on the busy turn; the runner was never reached.
    for _ in 0..10 {
        service.poll();
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        update_record(&service, "claude").message,
        "Waiting for running Claude Code work to finish."
    );
    busy.store(false, Ordering::Relaxed);
    testkit::settle(&mut service, Duration::from_secs(5));
    assert_eq!(update_record(&service, "claude").status, "succeeded");
}

#[test]
fn interrupted_record_recovers_failed_without_rerunning() {
    let fixture = testkit::Fixture::new();
    std::fs::create_dir_all(fixture.dir.join("providers")).unwrap();
    crate::store::write_atomic(
        &fixture.dir.join("providers/update-state.json"),
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "records": [{
                "updateId": "req-1", "driver": "claude", "status": "running",
                "message": "Running npm install", "startedAt": 1,
                "targetVersion": "2.1.0", "lockKey": "claude-native"
            }]
        }))
        .unwrap()
        .as_slice(),
    )
    .unwrap();
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::new(),
        prober(|_| ProbeResult::default()),
        no_run(),
    );
    detected(&mut service);
    let record = update_record(&service, "claude");
    assert_eq!(record.status, "failed");
    assert!(record.message.contains("stopped while"));
}

#[test]
fn already_current_precheck_skips_the_command() {
    let fixture = testkit::Fixture::new();
    let claude = fixture.npm_global("claude-npm", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("claude-npm")
        .to_string_lossy()
        .into_owned();
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        prober(move |argv| {
            if argv.ends_with(&["prefix".into(), "-g".into()]) {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: format!("{prefix}\n"),
                    ..Default::default()
                }
            } else if argv.contains(&"outdated".into()) {
                // Latest equals installed: request still validates against latest.
                ProbeResult {
                    exit_code: Some(1),
                    stdout: r#"{"@anthropic-ai/claude-code":{"latest":"2.0.5"}}"#.into(),
                    ..Default::default()
                }
            } else {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: "2.0.5".into(),
                    ..Default::default()
                }
            }
        }),
        no_run(),
    );
    detected(&mut service);
    service
        .request("claude", "2.0.5", uuid::Uuid::new_v4().to_string())
        .unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    let record = update_record(&service, "claude");
    assert_eq!(record.status, "succeeded");
    assert_eq!(record.message, "Already up to date.");
}

#[test]
fn exit_zero_without_version_change_is_unchanged() {
    let fixture = testkit::Fixture::new();
    // Exit 0 but the version never moves.
    let claude = fixture.npm_global("claude-npm3", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("claude-npm3")
        .to_string_lossy()
        .into_owned();
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        update_prober(prefix, Arc::new(AtomicBool::new(false))),
        testkit::FakeRunner::script(vec![ok_update()]),
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    let record = update_record(&service, "claude");
    assert_eq!(record.status, "unchanged");
    assert_eq!(record.message, "Claude Code still needs an update");
}

#[test]
fn failure_and_timeout_surface_ksg_wording() {
    for (i, (run, expected)) in [
        (
            CommandResult {
                exit_code: Some(1),
                stdout_tail: String::new(),
                stderr_tail: "hint\nerror: EACCES".into(),
            },
            "Update command exited with code 1: error: EACCES",
        ),
        (
            CommandResult {
                exit_code: None,
                ..Default::default()
            },
            "Update command failed",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = testkit::Fixture::new();
        let mut service = npm_service(&fixture, &i.to_string(), run.clone());
        detected(&mut service);
        service
            .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
            .unwrap();
        testkit::settle(&mut service, Duration::from_secs(5));
        let record = update_record(&service, "claude");
        assert_eq!(record.status, "failed");
        assert!(record.message.contains(expected), "{}", record.message);
    }
    // Timeout: the runner never finishes; the short build timeout settles it.
    let fixture = testkit::Fixture::new();
    let claude = fixture.npm_global("claude-npm", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("claude-npm")
        .to_string_lossy()
        .into_owned();
    struct Never;
    impl RunningUpdate for Never {
        fn progress(&mut self) -> Option<String> {
            None
        }
        fn result(&mut self) -> Option<CommandResult> {
            None
        }
        fn kill(&mut self) {}
    }
    struct NeverRunner;
    impl Runner for NeverRunner {
        fn start(&self, _: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
            Ok(Box::new(Never))
        }
    }
    let mut service = ProviderUpdates::build(
        &fixture.dir,
        Hooks::default(),
        Arc::new(update_prober(prefix, Arc::new(AtomicBool::new(false)))),
        Arc::new(NeverRunner),
        Some(Overrides {
            candidates: HashMap::from([(Driver::Claude, vec![claude]), (Driver::Codex, vec![])]),
            ..Default::default()
        }),
        Duration::from_millis(30),
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    let record = update_record(&service, "claude");
    assert_eq!(record.status, "failed");
    assert!(record.message.contains("timed out"));
}

fn npm_service(fixture: &testkit::Fixture, tag: &str, run: CommandResult) -> ProviderUpdates {
    let claude = fixture.npm_global(
        &format!("claude-npm-{tag}"),
        "claude",
        "@anthropic-ai/claude-code",
    );
    let prefix = fixture
        .dir
        .join(format!("claude-npm-{tag}"))
        .to_string_lossy()
        .into_owned();
    testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        update_prober(prefix, Arc::new(AtomicBool::new(false))),
        testkit::FakeRunner::script(vec![testkit::QueuedRun {
            result: run,
            delay: Duration::ZERO,
        }]),
    )
}

#[test]
fn record_failure_preserves_the_accepted_id() {
    let fixture = testkit::Fixture::new();
    let mut service = claude_npm_service(&fixture);
    detected(&mut service);
    let id = uuid::Uuid::new_v4().to_string();
    service
        .record_failure(
            "claude",
            "2.1.0",
            id.clone(),
            "Install blocked by the restart gate.".into(),
        )
        .unwrap();
    let record = update_record(&service, "claude");
    assert_eq!(record.update_id, id);
    assert_eq!(record.status, "failed");
    assert!(record.message.contains("blocked"));
    // The accepted id still replays as the same payload.
    assert!(service.request("claude", "2.1.0", id).is_ok());
}

#[test]
fn statuses_strip_output_but_refresh_keeps_it() {
    let fixture = testkit::Fixture::new();
    let mut service = npm_service(
        &fixture,
        "strip",
        CommandResult {
            exit_code: Some(1),
            stdout_tail: "sensitive /path/tail".into(),
            stderr_tail: String::new(),
        },
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    assert!(update_record(&service, "claude").output.is_none());
    let refreshed = service.refresh(true);
    let record = refreshed
        .into_iter()
        .find(|s| s.driver == "claude")
        .and_then(|s| s.update)
        .expect("record");
    assert!(record.output.is_some());
}

// ---------- small helpers ----------

#[test]
fn version_and_progress_parsing() {
    assert_eq!(parse_version("codex 0.48.0\n").as_deref(), Some("0.48.0"));
    assert_eq!(
        parse_version("2.0.5 (Claude Code)").as_deref(),
        Some("2.0.5")
    );
    assert_eq!(parse_version("no version"), None);
    assert_eq!(
        progress_line_raw("\u{1b}[32mDownloading  42%\u{1b}[0m").as_deref(),
        Some("Downloading 42%")
    );
    assert_eq!(compare_versions("2.0.5", "2.1.0"), std::cmp::Ordering::Less);
    assert_eq!(
        compare_versions("2.1.0", "2.1.0-alpha"),
        std::cmp::Ordering::Greater
    );
}

// ---------- release-blocker coverage ----------

#[test]
fn write_failure_fails_closed_before_any_install() {
    let fixture = testkit::Fixture::new();
    let claude = fixture.npm_global("claude-npm", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("claude-npm")
        .to_string_lossy()
        .into_owned();
    let fail = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(0usize));
    struct Counting(Arc<Mutex<usize>>);
    impl Runner for Counting {
        fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
            *self.0.lock().unwrap() += 1;
            testkit::FakeRunner::script(vec![ok_update()]).start(argv)
        }
    }
    let mut overrides = Overrides::default();
    overrides.candidates.insert(Driver::Claude, vec![claude]);
    overrides.fail_writes = Arc::clone(&fail);
    let mut service = ProviderUpdates::build(
        &fixture.dir,
        Hooks::default(),
        Arc::new(update_prober(prefix, Arc::new(AtomicBool::new(false)))),
        Arc::new(Counting(Arc::clone(&calls))),
        Some(overrides),
        Duration::from_secs(30),
    );
    detected(&mut service);
    fail.store(true, Ordering::Relaxed);
    // A request whose record cannot be persisted is refused outright; the
    // installer never runs.
    let error = service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap_err();
    assert!(error.contains("persist"), "{error}");
    assert!(service.failure().is_some());
    assert!(!service.active());
    for _ in 0..10 {
        service.poll();
    }
    assert_eq!(*calls.lock().unwrap(), 0);
    // Everything stays refused while the store is broken.
    assert!(service.validate_request("claude", "2.1.0").is_err());
    assert!(
        service
            .record_failure(
                "claude",
                "2.1.0",
                uuid::Uuid::new_v4().to_string(),
                "x".into()
            )
            .is_err()
    );
}

#[test]
fn malformed_ledger_disables_updates_and_preserves_file() {
    let fixture = testkit::Fixture::new();
    let ledger = fixture.dir.join("providers/update-state.json");
    for contents in ["not json at all", r#"{"version": 99, "records": []}"#] {
        std::fs::create_dir_all(ledger.parent().unwrap()).unwrap();
        std::fs::write(&ledger, contents).unwrap();
        let mut service = testkit::service(
            &fixture.dir,
            HashMap::new(),
            prober(|_| ProbeResult::default()),
            no_run(),
        );
        detected(&mut service);
        assert!(service.failure().is_some(), "{contents}");
        assert!(
            service
                .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
                .is_err()
        );
        // The unreadable file is preserved exactly.
        assert_eq!(std::fs::read_to_string(&ledger).unwrap(), contents);
        std::fs::remove_file(&ledger).unwrap();
    }
}

#[test]
fn locks_survive_owner_death_and_serialize_shared_prefixes() {
    // A lock held by a dead owner is free again: a second service retries.
    let fixture = testkit::Fixture::new();
    let lock_key = "npm-global:/shared".to_owned();
    {
        let _held = try_lock_file(&fixture.dir, &lock_key).expect("first lock");
        assert!(
            try_lock_file(&fixture.dir, &lock_key).is_none(),
            "held lock must exclude others"
        );
    } // drop releases the OS lock
    assert!(try_lock_file(&fixture.dir, &lock_key).is_some());
}

#[test]
fn shared_prefix_serializes_two_drivers() {
    let fixture = testkit::Fixture::new();
    // One npm prefix owns both CLIs.
    let claude = fixture.npm_global("shared", "claude", "@anthropic-ai/claude-code");
    let codex = fixture.npm_global("shared", "codex", "@openai/codex");
    let prefix = fixture.dir.join("shared").to_string_lossy().into_owned();
    let updated = Arc::new(AtomicBool::new(false));
    let prober = {
        let updated = Arc::clone(&updated);
        prober(move |argv| {
            if argv.ends_with(&["prefix".into(), "-g".into()]) {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: format!("{prefix}\n"),
                    ..Default::default()
                }
            } else if argv.contains(&"outdated".into()) {
                let pkg = argv.last().cloned().unwrap_or_default();
                ProbeResult {
                    exit_code: Some(1),
                    stdout: format!(r#"{{"{pkg}":{{"latest":"2.1.0"}}}}"#),
                    ..Default::default()
                }
            } else {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: if updated.load(Ordering::Relaxed) {
                        "2.1.0".into()
                    } else {
                        "2.0.5".into()
                    },
                    ..Default::default()
                }
            }
        })
    };
    struct SlowFlip(Arc<AtomicBool>);
    impl Runner for SlowFlip {
        fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
            let run = testkit::FakeRunner::script(vec![testkit::QueuedRun {
                result: CommandResult {
                    exit_code: Some(0),
                    stdout_tail: "done".into(),
                    stderr_tail: String::new(),
                },
                delay: Duration::from_millis(80),
            }]);
            let handle = run.start(argv)?;
            self.0.store(true, Ordering::Relaxed);
            Ok(handle)
        }
    }
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude]), (Driver::Codex, vec![codex])]),
        prober,
        SlowFlip(updated),
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    service
        .request("codex", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    // Same install lock: whichever driver steps first holds the prefix while
    // the other stays queued; a second installer must never run concurrently.
    let mut saw_waiting = false;
    for _ in 0..80 {
        service.poll();
        thread::sleep(Duration::from_millis(3));
        let statuses = service.statuses();
        let queued = statuses
            .iter()
            .filter(|s| s.update.as_ref().is_some_and(|r| r.status == "queued"))
            .count();
        let running = statuses
            .iter()
            .filter(|s| s.update.as_ref().is_some_and(|r| r.status == "running"))
            .count();
        if queued > 0 && running > 0 {
            saw_waiting = true;
        }
    }
    assert!(saw_waiting, "no driver ever waited on the shared lock");
    testkit::settle(&mut service, Duration::from_secs(10));
    assert_eq!(update_record(&service, "claude").status, "succeeded");
    assert_eq!(update_record(&service, "codex").status, "succeeded");
}

#[test]
fn refresh_while_active_never_drops_the_command() {
    let fixture = testkit::Fixture::new();
    // Parallel updates on distinct lock keys: npm-global Claude, keg Codex.
    let claude = fixture.npm_global("claude-a", "claude", "@anthropic-ai/claude-code");
    let brew = fixture.native("brew/bin/brew");
    let codex = fixture.native("brew/Cellar/codex/0.48.0/bin/codex");
    let npm_prefix = fixture.dir.join("claude-a").to_string_lossy().into_owned();
    let brew_prefix = fixture.dir.join("brew").to_string_lossy().into_owned();
    let updated = Arc::new(AtomicBool::new(false));
    let prober = {
        let updated = Arc::clone(&updated);
        prober(move |argv| {
            if argv.ends_with(&["prefix".into(), "-g".into()]) {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: format!("{npm_prefix}\n"),
                    ..Default::default()
                }
            } else if argv.ends_with(&["--prefix".into()]) && argv[0].contains("brew") {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: format!("{brew_prefix}\n"),
                    ..Default::default()
                }
            } else if argv.contains(&"outdated".into()) {
                ProbeResult {
                    exit_code: Some(1),
                    stdout: r#"{"@anthropic-ai/claude-code":{"latest":"2.1.0"}}"#.into(),
                    ..Default::default()
                }
            } else if argv.contains(&"info".into()) {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: r#"{"formulae":[{"versions":{"stable":"2.1.0"}}]}"#.into(),
                    ..Default::default()
                }
            } else {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: if updated.load(Ordering::Relaxed) {
                        "2.1.0".into()
                    } else {
                        "2.0.5".into()
                    },
                    ..Default::default()
                }
            }
        })
    };
    let updated2 = Arc::clone(&updated);
    struct Flip(Arc<AtomicBool>);
    impl Runner for Flip {
        fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
            let run = testkit::FakeRunner::script(vec![testkit::QueuedRun {
                result: CommandResult {
                    exit_code: Some(0),
                    stdout_tail: "done".into(),
                    stderr_tail: String::new(),
                },
                delay: Duration::from_millis(60),
            }]);
            let handle = run.start(argv)?;
            self.0.store(true, Ordering::Relaxed);
            Ok(handle)
        }
    }
    let mut overrides = Overrides::default();
    overrides.candidates.insert(Driver::Claude, vec![claude]);
    overrides.candidates.insert(Driver::Codex, vec![codex]);
    overrides.tools.insert("brew".into(), PathBuf::from(&brew));
    let mut service = ProviderUpdates::build(
        &fixture.dir,
        Hooks::default(),
        Arc::new(prober),
        Arc::new(Flip(updated2)),
        Some(overrides),
        Duration::from_secs(30),
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    service
        .request("codex", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    for _ in 0..30 {
        service.poll();
        let _ = service.refresh(true); // must not strand the running commands
        thread::sleep(Duration::from_millis(4));
    }
    testkit::settle(&mut service, Duration::from_secs(10));
    assert_eq!(update_record(&service, "claude").status, "succeeded");
    assert_eq!(update_record(&service, "codex").status, "succeeded");
}

#[cfg(unix)]
#[test]
fn real_command_output_is_bounded_and_utf8_safe() {
    let fixture = testkit::Fixture::new();
    // ~5 MB of multi-byte output, far beyond the capture cap.
    let big = fixture.dir.join("big.txt");
    let chunk = "héllo wörld  🌊\n".repeat(300_000);
    std::fs::write(&big, &chunk).unwrap();
    let mut handle = SystemRunner
        .start(&["/bin/cat".into(), big.to_string_lossy().into_owned()])
        .unwrap();
    let result = loop {
        if let Some(result) = handle.result() {
            break result;
        }
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(result.exit_code, Some(0));
    assert!(result.stdout_tail.len() <= OUTPUT_MAX_BYTES + 64);
    assert!(result.stdout_tail.ends_with("🌊\n") || !result.stdout_tail.is_empty());
}

#[test]
fn windows_npm_cmd_shim_becomes_node_argv() {
    let fixture = testkit::Fixture::new();
    // A Windows-style npm shim: npm.cmd referencing %dp0%\...\npm-cli.js.
    let win = fixture.dir.join("win-npm-tool");
    std::fs::create_dir_all(&win).unwrap();
    std::fs::write(
        win.join("npm.cmd"),
        "@echo off\r\n\"%dp0%\\node_modules\\npm\\bin\\npm-cli.js\" %*\r\n",
    )
    .unwrap();
    // Backslashes are legal in unix filenames; this is the shim's JS target.
    std::fs::write(win.join("node_modules\\npm\\bin\\npm-cli.js"), "// npm\n").unwrap();
    let claude = fixture.npm_global("win-prefix", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("win-prefix")
        .to_string_lossy()
        .into_owned();
    let mut overrides = Overrides::default();
    overrides.candidates.insert(Driver::Claude, vec![claude]);
    overrides.tools.insert("npm".into(), win.join("npm.cmd"));
    let updated = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(Mutex::new(vec![]));
    struct Record(Arc<Mutex<Vec<Vec<String>>>>, Arc<AtomicBool>);
    impl Runner for Record {
        fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
            self.0.lock().unwrap().push(argv.to_vec());
            let handle = testkit::FakeRunner::script(vec![ok_update()]).start(argv)?;
            self.1.store(true, Ordering::Relaxed);
            Ok(handle)
        }
    }
    let mut service = ProviderUpdates::build(
        &fixture.dir,
        Hooks::default(),
        Arc::new(update_prober(prefix, Arc::clone(&updated))),
        Arc::new(Record(Arc::clone(&calls), updated)),
        Some(overrides),
        Duration::from_secs(30),
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    let argv = &calls.lock().unwrap()[0];
    // The update ran `node <npm-cli.js> install -g ...`, never a bare shim.
    assert!(argv[0].ends_with("node") || argv[0] == "node", "{argv:?}");
    assert!(argv[1].ends_with("npm-cli.js"), "{argv:?}");
}

#[test]
fn mixed_case_prefix_is_preserved_not_folded() {
    let fixture = testkit::Fixture::new();
    let claude = fixture.npm_global("MixedCasePrefix", "claude", "@anthropic-ai/claude-code");
    let prefix_path = fixture.dir.join("MixedCasePrefix");
    let prefix = prefix_path.to_string_lossy().into_owned();
    let detected = detect(
        Driver::Claude,
        &prober(move |argv| {
            if argv.ends_with(&["prefix".into(), "-g".into()]) {
                ProbeResult {
                    exit_code: Some(0),
                    stdout: format!("{prefix}\n"),
                    ..Default::default()
                }
            } else {
                ProbeResult::default()
            }
        }),
        Some(&Overrides {
            candidates: HashMap::from([(Driver::Claude, vec![claude])]),
            ..Default::default()
        }),
    );
    let Some(Strategy::NpmGlobal { prefix, .. }) = detected.strategy else {
        panic!("expected npm-global, got {detected:?}");
    };
    // Original case survives canonicalization and extraction; only the
    // comparison helpers fold.
    assert_eq!(prefix, std::fs::canonicalize(&prefix_path).unwrap());
}

#[test]
fn malformed_installed_output_never_succeeds() {
    let fixture = testkit::Fixture::new();
    let claude = fixture.npm_global("claude-npm", "claude", "@anthropic-ai/claude-code");
    let prefix = fixture
        .dir
        .join("claude-npm")
        .to_string_lossy()
        .into_owned();
    let updated = Arc::new(AtomicBool::new(false));
    let prober = prober(move |argv| {
        if argv.ends_with(&["prefix".into(), "-g".into()]) {
            ProbeResult {
                exit_code: Some(0),
                stdout: format!("{prefix}\n"),
                ..Default::default()
            }
        } else if argv.contains(&"outdated".into()) {
            ProbeResult {
                exit_code: Some(1),
                stdout: r#"{"@anthropic-ai/claude-code":{"latest":"2.1.0"}}"#.into(),
                ..Default::default()
            }
        } else {
            // Unparseable both before and after the command.
            ProbeResult {
                exit_code: Some(0),
                stdout: "1.2.garbage".into(),
                ..Default::default()
            }
        }
    });
    struct Flip(Arc<AtomicBool>);
    impl Runner for Flip {
        fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
            let handle = testkit::FakeRunner::script(vec![ok_update()]).start(argv)?;
            self.0.store(true, Ordering::Relaxed);
            Ok(handle)
        }
    }
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(Driver::Claude, vec![claude])]),
        prober,
        Flip(updated),
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    let record = update_record(&service, "claude");
    assert_eq!(record.status, "failed");
    assert!(record.message.contains("verify"), "{}", record.message);
}

#[test]
fn strict_semver_rules_versions() {
    assert!(valid_semver("2.1.0"));
    assert!(valid_semver("2.1.0-rc.1"));
    assert!(valid_semver("2.1.0+build.5"));
    assert!(!valid_semver("1.2.garbage"));
    assert!(!valid_semver("v2.1"));
    assert!(!valid_semver("2.1.0-"));
    assert_eq!(parse_version("codex 0.48.0\n").as_deref(), Some("0.48.0"));
    assert_eq!(parse_version("1.2.garbage"), None);
    assert_eq!(
        compare_versions("2.1.0-alpha", "2.1.0"),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        compare_versions("2.1.0", "2.1.0-alpha"),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        compare_versions("2.1.0-rc.1", "2.1.0-rc.2"),
        std::cmp::Ordering::Less
    );
}

#[test]
fn new_id_while_active_is_rejected_even_same_payload() {
    let fixture = testkit::Fixture::new();
    let mut service = claude_npm_service(&fixture);
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    // Different UUID, identical payload: a new intent, refused while active.
    let error = service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap_err();
    assert!(error.contains("already running"), "{error}");
}

#[test]
fn captured_output_is_secret_redacted() {
    let fixture = testkit::Fixture::new();
    let mut service = npm_service(
        &fixture,
        "redact",
        CommandResult {
            exit_code: Some(1),
            stdout_tail: String::new(),
            stderr_tail:
                "fetch https://user:secret-token@registry.example/x failed\nghp_abc123def456".into(),
        },
    );
    detected(&mut service);
    service
        .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    let record = service
        .refresh(true)
        .into_iter()
        .find(|s| s.driver == "claude")
        .and_then(|s| s.update)
        .unwrap();
    let output = record.output.unwrap_or_default();
    assert!(!output.contains("secret-token"), "{output}");
    assert!(!output.contains("ghp_abc123def456"), "{output}");
    assert!(
        !record.message.contains("secret-token"),
        "{}",
        record.message
    );
}

#[cfg(unix)]
fn version_directory_update(driver: Driver) {
    let fixture = testkit::Fixture::new();
    let prefix = fixture.dir.join("MixedCasePrefix");
    let (old, new) = if driver == Driver::Codex {
        (
            fixture.native("MixedCasePrefix/Cellar/codex/2.0.0/bin/codex"),
            fixture.native("MixedCasePrefix/Cellar/codex/2.1.0/bin/codex"),
        )
    } else {
        (
            fixture.native("Home/.local/share/claude/versions/2.0.0/claude"),
            fixture.native("Home/.local/share/claude/versions/2.1.0/claude"),
        )
    };
    fixture.native("MixedCasePrefix/bin/brew");
    let launcher = fixture.dir.join(format!("launch/{}", driver.id()));
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(old, &launcher).unwrap();
    let probe_launcher = launcher.clone();
    let mut prober = prober(move |argv| {
        let stdout = if argv.last().is_some_and(|arg| arg == "--prefix") {
            prefix.to_string_lossy().into_owned()
        } else if argv.iter().any(|arg| arg == "info") {
            r#"{"formulae":[{"versions":{"stable":"2.1.0"}}]}"#.into()
        } else if probe_launcher
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .contains("/2.1.0/")
        {
            "2.1.0".into()
        } else {
            "2.0.0".into()
        };
        ProbeResult {
            exit_code: Some(0),
            stdout,
            ..Default::default()
        }
    });
    prober.http = Some("2.1.0".into());
    struct MoveLauncher {
        launcher: PathBuf,
        replacement: String,
    }
    impl Runner for MoveLauncher {
        fn start(&self, _argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
            std::fs::remove_file(&self.launcher).unwrap();
            std::os::unix::fs::symlink(&self.replacement, &self.launcher).unwrap();
            testkit::FakeRunner::script(vec![ok_update()]).start(_argv)
        }
    }
    let mut service = testkit::service(
        &fixture.dir,
        HashMap::from([(driver, vec![launcher.to_string_lossy().into_owned()])]),
        prober,
        MoveLauncher {
            launcher,
            replacement: new,
        },
    );
    detected(&mut service);
    service
        .request(driver.id(), "2.1.0", uuid::Uuid::new_v4().to_string())
        .unwrap();
    testkit::settle(&mut service, Duration::from_secs(5));
    assert_eq!(
        status(&service, driver.id()).update.unwrap().status,
        "succeeded"
    );
}

#[cfg(unix)]
#[test]
fn brew_update_accepts_a_new_version_keg_under_the_same_owned_prefix() {
    version_directory_update(Driver::Codex);
}

#[cfg(unix)]
#[test]
fn native_claude_update_accepts_a_new_version_under_the_same_installation() {
    version_directory_update(Driver::Claude);
}

#[test]
fn disconnected_probe_workers_fail_instead_of_holding_the_driver_forever() {
    let fixture = testkit::Fixture::new();
    for (index, phase) in [Phase::Reproving, Phase::CheckingLatest, Phase::Verifying]
        .into_iter()
        .enumerate()
    {
        let mut service = npm_service(
            &fixture,
            &format!("worker-stopped-{index}"),
            CommandResult::default(),
        );
        detected(&mut service);
        service
            .request("claude", "2.1.0", uuid::Uuid::new_v4().to_string())
            .unwrap();
        service.active.get_mut(&Driver::Claude).unwrap().phase = phase;
        let job = match phase {
            Phase::Reproving => {
                let (sender, receiver) = mpsc::channel();
                drop(sender);
                Job::Reprove(receiver)
            }
            Phase::CheckingLatest => {
                let (sender, receiver) = mpsc::channel();
                drop(sender);
                Job::Latest(receiver)
            }
            _ => {
                let (sender, receiver) = mpsc::channel();
                drop(sender);
                Job::Verify(receiver)
            }
        };
        service.jobs.insert(Driver::Claude, job);
        service.poll();
        assert!(!service.active());
        let record = status(&service, "claude").update.unwrap();
        assert_eq!(record.status, "failed");
        assert!(record.message.contains("worker stopped"));
        assert!(service.admits("claude"));
    }
}
