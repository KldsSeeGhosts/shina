//! Update-watch recovery against the real `shina` binary. The platform service
//! manager commands are confined to a fixture directory;
//! every other piece (watch record, install lock, owner lock, outcome file) is real.
use super::*;

/// Unix stubs log each manager call; `launchctl print` reports "not loaded".
/// On Windows the empty override directory keeps the real scheduler out of
/// these tests. The native scheduler test in server_runtime is opt-in.
fn fake_manager_bin(profile: &Profile) {
    let bin = profile.root.join("fake-bin");
    fs::create_dir_all(&bin).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for (name, body) in [
            ("id", "#!/bin/sh\nprintf '1000\\n'\n"),
            (
                "launchctl",
                "#!/bin/sh\nprintf 'launchctl %s\\n' \"$*\" >> calls.log\n[ \"$1\" != print ]\n",
            ),
            (
                "systemctl",
                "#!/bin/sh\nprintf 'systemctl %s\\n' \"$*\" >> calls.log\n",
            ),
        ] {
            let path = bin.join(name);
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

fn write_service(profile: &Profile) {
    let directory = profile.root.join("Shina");
    fs::write(
        directory.join("service.json"),
        json!({
            "platform":if cfg!(windows) { "Windows" } else if cfg!(target_os="macos") { "Mac" } else { "Linux" },
            "name":"Shina-recovery-fixture","executable":env!("CARGO_BIN_EXE_shina"),
            "directory":directory,"unit":directory.join("unused-unit"),"home":profile.root,
            "environment":{}
        })
        .to_string(),
    )
    .unwrap();
}

fn write_watch(profile: &Profile, update_id: &str, registered_at: u64) {
    let updates = profile.root.join("Shina/updates");
    fs::create_dir_all(&updates).unwrap();
    fs::write(
        updates.join("update-watch.json"),
        json!({"updateId":update_id,"tag":"v9.9.9","service":"Shina-recovery-fixture",
            "registeredAt":registered_at})
        .to_string(),
    )
    .unwrap();
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn recover(profile: &Profile, update_id: &str) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_shina"));
    command
        .args(["service", "recover-update", update_id])
        .env("LOCALAPPDATA", &profile.root)
        .env("XDG_DATA_HOME", &profile.root)
        .env("SHINA_SERVICE_BIN_DIR", profile.root.join("fake-bin"))
        .current_dir(&profile.root)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.output().unwrap()
}

#[cfg(unix)]
#[test]
fn recover_update_starts_a_stopped_service_and_keeps_the_watch() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    write_service(&profile);
    write_watch(&profile, "watch-1", unix_now());
    let output = recover(&profile, "watch-1");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Started");
    assert!(output.status.success(), "{output:?}");
    let calls = fs::read_to_string(profile.root.join("calls.log")).unwrap();
    let start = if cfg!(target_os = "macos") {
        "launchctl bootstrap"
    } else {
        "systemctl --user start Shina-recovery-fixture.service"
    };
    assert!(calls.contains(start), "{calls}");
    // The service did not verify readiness yet: the watch stays for the next pass.
    assert!(
        profile
            .root
            .join("Shina/updates/update-watch.json")
            .is_file()
    );
    profile.stop();
}

#[test]
fn recover_update_at_expiry_records_an_honest_failure_and_removes_the_watch() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    write_service(&profile);
    write_watch(&profile, "watch-2", unix_now() - 46 * 60);
    let output = recover(&profile, "watch-2");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Expired");
    assert!(output.status.success(), "{output:?}");
    let outcome: Value =
        serde_json::from_slice(&fs::read(profile.root.join("Shina/updates/outcome.json")).unwrap())
            .unwrap();
    assert_eq!(outcome["updateId"], "watch-2");
    assert_eq!(outcome["success"], false);
    assert!(
        outcome["reason"]
            .as_str()
            .unwrap()
            .contains("did not come back"),
        "{outcome}"
    );
    assert!(
        !profile
            .root
            .join("Shina/updates/update-watch.json")
            .exists()
    );
    // A second pass finds no record: a missing watch is a pure no-op that only
    // unloads the stale job, and the recorded outcome is never duplicated.
    let output = recover(&profile, "watch-2");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Stale");
    let again: Value =
        serde_json::from_slice(&fs::read(profile.root.join("Shina/updates/outcome.json")).unwrap())
            .unwrap();
    assert_eq!(again, outcome);
    profile.stop();
}

#[cfg(target_os = "macos")]
fn write_journal(profile: &Profile, update_id: &str, phase: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let updates = profile.root.join("Shina/updates");
    let backup = profile.root.join("Shina/backups/before-update-test");
    fs::create_dir_all(&backup).unwrap();
    fs::write(backup.join("notes.json"), json!({"kept":true}).to_string()).unwrap();
    fs::write(
        updates.join("update-journal.json"),
        json!({"updateId":update_id,"tag":"v9.9.9","service":"Shina-recovery-fixture",
            "backup":backup,"at":unix_now(),"phase":phase,
            "previousSha256":format!("{:x}",Sha256::digest(b"previous binary")),
            "replacementSha256":format!("{:x}",Sha256::digest(b"new binary")),
            "previousVersion":env!("CARGO_PKG_VERSION")})
        .to_string(),
    )
    .unwrap();
    backup
}

#[cfg(target_os = "macos")]
fn write_service_missing_bundle(profile: &Profile) -> (PathBuf, PathBuf) {
    // The installed app bundle is absent mid-swap; only the previous copy remains.
    let install = profile.root.join("install");
    let app = install.join("Shina.app");
    let previous = install.join("Shina-app.previous");
    fs::create_dir_all(previous.join("Contents/MacOS")).unwrap();
    fs::write(previous.join("Contents/MacOS/shina"), b"previous binary").unwrap();
    let directory = profile.root.join("Shina");
    fs::write(
        directory.join("service.json"),
        json!({
            "platform":"Mac","name":"Shina-recovery-fixture",
            "executable":app.join("Contents/MacOS/shina"),
            "directory":directory,"unit":directory.join("unused-unit"),"home":profile.root,
            "environment":{}
        })
        .to_string(),
    )
    .unwrap();
    (app, previous)
}

#[cfg(target_os = "macos")]
#[test]
fn an_interrupted_swap_restores_the_matched_binary_and_backup_before_starting() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    let (app, previous) = write_service_missing_bundle(&profile);
    write_watch(&profile, "watch-6", unix_now());
    write_journal(&profile, "watch-6", "RollingBack");
    fs::write(profile.root.join("Shina/update-validation"), b"hold").unwrap();
    let output = recover(&profile, "watch-6");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Started");
    assert!(output.status.success(), "{output:?}");
    // The previous bundle is back in place, the matched backup was replayed into
    // the profile, and the hold and journal are cleared before the service starts.
    assert!(!previous.exists());
    assert!(app.join("Contents/MacOS/shina").is_file());
    assert_eq!(
        serde_json::from_str::<Value>(
            &fs::read_to_string(profile.root.join("Shina/notes.json")).unwrap()
        )
        .unwrap(),
        json!({"kept":true})
    );
    assert!(!profile.root.join("Shina/update-validation").exists());
    assert!(
        !profile
            .root
            .join("Shina/updates/update-journal.json")
            .exists()
    );
    // The watch itself stays: the service has not proven readiness yet.
    assert!(
        profile
            .root
            .join("Shina/updates/update-watch.json")
            .is_file()
    );
    let calls = fs::read_to_string(profile.root.join("calls.log")).unwrap();
    assert!(calls.contains("bootstrap"), "{calls}");
    profile.stop();
}

#[test]
fn a_running_server_makes_the_watch_remove_itself() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    write_service(&profile);
    write_watch(&profile, "watch-5", unix_now());
    // A real serve process owns the profile without any service manager involved.
    profile.launch(&["serve"]);
    profile.discovery();
    let output = recover(&profile, "watch-5");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Healthy");
    assert!(output.status.success(), "{output:?}");
    assert!(
        !profile
            .root
            .join("Shina/updates/update-watch.json")
            .exists()
    );
    profile.stop();
}
