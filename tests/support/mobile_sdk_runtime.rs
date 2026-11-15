//! Cross-language acceptance: the actual packages/mobile-client TypeScript
//! core against a real `shina serve` process and its fake Pi harness. The
//! test launches serve, mints a real pairing code through devices.pair,
//! hands the Node test a user-private config file through one environment
//! variable, then validates the fixture prompt log and the push outbox.
//! Credentials travel only inside that file; nothing prints them.

use super::*;

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("poll node test") {
            return Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The paired device pushes only through the SDK's own pinned transport; the
/// fixture config pins a synthetic tailnet literal the test adapter maps to
/// the loopback port serve actually chose.
#[test]
fn mobile_sdk_node_client_interoperates_with_a_live_serve() {
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packages/mobile-client");
    assert!(
        package.join("test/live-server.test.ts").is_file(),
        "missing packages/mobile-client/test/live-server.test.ts"
    );
    match Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let major: u32 = version
                .trim_start_matches('v')
                .split('.')
                .next()
                .and_then(|part| part.parse().ok())
                .unwrap_or(0);
            assert!(
                major >= 22,
                "the live mobile SDK test needs Node.js >= 22.6 (it runs TypeScript tests natively); found {version}"
            );
        }
        _ => panic!(
            "the live mobile SDK test needs Node.js >= 22.6 on PATH, plus `npm install` inside packages/mobile-client"
        ),
    }
    assert!(
        package.join("node_modules/ws/package.json").is_file(),
        "the live mobile SDK test needs dev dependencies: run `npm install` inside packages/mobile-client (ws drives the test WebSocket adapter)"
    );

    let mut profile = Profile::new();
    profile.launch(&["serve"]);
    let host = profile.discovery();
    let mut operator = profile.client(uuid::Uuid::new_v4());
    let issued = operator.rpc("devices.pair", json!({}), json!("mint"));
    assert!(issued["error"].is_null(), "{issued}");
    let code = issued["result"]["code"].as_str().unwrap();
    let expires = issued["result"]["expiresAt"].as_u64().unwrap();
    drop(operator);

    let link = format!(
        "shina://pair/v1?host=100.64.0.10&port={}&serverId={}&expiresAt={expires}#code={code}",
        host["port"].as_u64().unwrap(),
        host["serverId"].as_str().unwrap()
    );
    let config = json!({
        "pairLink": link,
        "pinnedIp": "100.64.0.10",
        "dialHost": "127.0.0.1",
        "promptLog": profile.root.join("Shina/chats/1/fixture-prompts.jsonl"),
    });
    let config_path = profile.root.join("live-mobile-config.json");
    fs::write(&config_path, config.to_string()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    let mut child = Command::new("node")
        .args([
            "--test",
            "--test-reporter=tap",
            "--experimental-strip-types",
            "test/live-server.test.ts",
        ])
        .current_dir(&package)
        .env("SHINA_LIVE_CONFIG", &config_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn node --test");
    let status = wait_with_timeout(&mut child, Duration::from_secs(300));
    let output = child.wait_with_output().expect("collect node test output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stdout.lines() {
        if line.starts_with("ok ") || line.starts_with("not ok ") || line.contains("# ") {
            eprintln!("live-sdk tap: {line}");
        }
    }
    for line in stderr.lines() {
        if line.contains("# SKIP") {
            eprintln!("live SDK coverage note: {line}");
        }
    }
    assert!(
        status.is_some_and(|status| status.success()),
        "node live test failed or timed out\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    // The push section skips when the server never negotiates mobile-push-v1;
    // a minor-28 client against this server must always negotiate it.
    assert!(
        stdout.contains("# push-negotiated"),
        "server did not negotiate mobile-push-v1 with a minor-28 client\n{stdout}"
    );

    let prompts: Vec<String> =
        fs::read_to_string(profile.root.join("Shina/chats/1/fixture-prompts.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<String>(line).unwrap_or_else(|_| line.to_owned()))
            .collect();
    assert_eq!(prompts.len(), 4, "unexpected fixture prompts: {prompts:?}");
    for expected in [
        "sdk-live-fixture",
        "sdk-uncertain-fixture",
        "history-stream-fixture",
        "remote-confirm-fixture",
    ] {
        assert_eq!(
            prompts.iter().filter(|prompt| *prompt == expected).count(),
            1,
            "fake harness saw the wrong count for {expected}"
        );
    }

    // The fake Expo token must never be deliverable: no push job was ever
    // enqueued, and no registration still holds a token after unregister.
    let database = rusqlite::Connection::open(profile.root.join("Shina/state.sqlite")).unwrap();
    let jobs: i64 = database
        .query_row("SELECT count(*) FROM mobile_push_jobs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(jobs, 0, "a push job would send the fake token to real Expo");
    let live_tokens: i64 = database
        .query_row(
            "SELECT count(*) FROM mobile_push_registrations WHERE token != ''",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(live_tokens, 0, "the fake token must be unregistered");
    profile.stop();
}
