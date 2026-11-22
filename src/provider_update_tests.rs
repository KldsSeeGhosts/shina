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

// ---------- request validation (pre-commit) ----------

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

// ---------- small helpers ----------

// ---------- release-blocker coverage ----------

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
    let npm_entry = win.join("node_modules\\npm\\bin\\npm-cli.js");
    std::fs::create_dir_all(npm_entry.parent().unwrap()).unwrap();
    std::fs::write(&npm_entry, "// npm\n").unwrap();
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
