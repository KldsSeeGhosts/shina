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

#[test]
fn a_stale_invocation_never_touches_a_newer_watch() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    write_service(&profile);
    // A live, unexpired watch for a newer update.
    write_watch(&profile, "new-watch", unix_now());
    let output = recover(&profile, "old-update-id");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Stale");
    assert!(output.status.success(), "{output:?}");
    // The newer watch record survives untouched, the service was not started for
    // someone else's watch, and no outcome was written.
    assert!(
        profile
            .root
            .join("Shina/updates/update-watch.json")
            .is_file()
    );
    assert!(
        !profile.root.join("calls.log").exists(),
        "a stale invocation must not start the service"
    );
    assert!(!profile.root.join("Shina/updates/outcome.json").exists());
    // A watch record for another service is just as fenced.
    write_watch(&profile, "new-watch", unix_now());
    fs::write(
        profile.root.join("Shina/updates/update-watch.json"),
        json!({"updateId":"new-watch","tag":"v9.9.9","service":"Shina-other",
            "registeredAt":unix_now()})
        .to_string(),
    )
    .unwrap();
    let output = recover(&profile, "new-watch");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Stale");
    assert!(!profile.root.join("calls.log").exists());
    profile.stop();
}

#[test]
fn recover_update_runs_from_the_profile_argument_not_the_environment() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    write_service(&profile);
    write_watch(&profile, "watch-pinned", unix_now() - 46 * 60);
    // The generated job pins --profile: the environment may resolve a different,
    // even missing, data directory and the watch still finds its own record.
    let decoy = profile.root.join("decoy-profile");
    fs::create_dir_all(decoy.join("Shina")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_shina"));
    command
        .args([
            "service",
            "recover-update",
            "watch-pinned",
            "--profile",
            profile.root.join("Shina").to_str().unwrap(),
        ])
        .env("LOCALAPPDATA", &decoy)
        .env("XDG_DATA_HOME", &decoy)
        .env("SHINA_SERVICE_BIN_DIR", profile.root.join("fake-bin"))
        .current_dir(&profile.root)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Expired");
    let outcome: Value =
        serde_json::from_slice(&fs::read(profile.root.join("Shina/updates/outcome.json")).unwrap())
            .unwrap();
    assert_eq!(outcome["updateId"], "watch-pinned");
    assert!(!decoy.join("Shina/updates").exists());
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

#[cfg(target_os = "macos")]
#[test]
fn a_replaced_but_held_update_starts_once_and_keeps_its_journal() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    let (app, _previous) = write_service_missing_bundle(&profile);
    // This time the new bundle landed: the executable is present.
    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    fs::write(app.join("Contents/MacOS/shina"), b"new binary").unwrap();
    write_watch(&profile, "watch-7", unix_now());
    write_journal(&profile, "watch-7", "Replaced");
    fs::write(profile.root.join("Shina/update-validation"), b"hold").unwrap();
    let output = recover(&profile, "watch-7");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Started");
    assert!(output.status.success(), "{output:?}");
    // Starting an unvalidated binary must not clear the hold or the journal: the
    // next tick, with the server owning the profile, validates or rolls back.
    assert!(profile.root.join("Shina/update-validation").exists());
    assert!(
        profile
            .root
            .join("Shina/updates/update-journal.json")
            .is_file()
    );
    profile.stop();
}

#[cfg(target_os = "macos")]
#[test]
fn a_repeated_recovery_keeps_the_restored_binary_and_new_user_data() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    let (app, previous) = write_service_missing_bundle(&profile);
    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    fs::write(app.join("Contents/MacOS/shina"), b"previous binary").unwrap();
    fs::write(previous.join("Contents/MacOS/shina"), b"new binary").unwrap();
    fs::write(
        profile.root.join("Shina/notes.json"),
        b"post-recovery user data",
    )
    .unwrap();
    write_watch(&profile, "watch-repeat", unix_now());
    write_journal(&profile, "watch-repeat", "Restored");
    let output = recover(&profile, "watch-repeat");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Started");
    assert_eq!(
        fs::read(app.join("Contents/MacOS/shina")).unwrap(),
        b"previous binary"
    );
    assert_eq!(
        fs::read(profile.root.join("Shina/notes.json")).unwrap(),
        b"post-recovery user data"
    );
    let outcome: Value =
        serde_json::from_slice(&fs::read(profile.root.join("Shina/updates/outcome.json")).unwrap())
            .unwrap();
    assert_eq!(outcome["updateId"], "watch-repeat");
    assert_eq!(outcome["success"], false);
    profile.stop();
}

#[test]
fn a_corrupt_journal_or_a_hold_without_a_journal_never_guesses_a_backup() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    write_service(&profile);
    write_watch(&profile, "watch-corrupt", unix_now());
    fs::write(profile.root.join("Shina/update-validation"), b"hold").unwrap();
    let journal = profile.root.join("Shina/updates/update-journal.json");
    fs::write(&journal, b"{broken").unwrap();
    let output = recover(&profile, "watch-corrupt");
    assert!(!output.status.success());
    assert_eq!(fs::read(&journal).unwrap(), b"{broken");
    fs::remove_file(&journal).unwrap();
    let output = recover(&profile, "watch-corrupt");
    assert!(!output.status.success());
    assert!(profile.root.join("Shina/update-validation").exists());
    assert!(!profile.root.join("calls.log").exists());
    profile.stop();
}

#[test]
fn recover_update_never_overwrites_the_helpers_own_outcome() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    write_service(&profile);
    write_watch(&profile, "watch-3", unix_now() - 46 * 60);
    let updates = profile.root.join("Shina/updates");
    fs::write(
        updates.join("outcome.json"),
        json!({"updateId":"watch-3","tag":"v9.9.9","success":true,"reason":null}).to_string(),
    )
    .unwrap();
    let output = recover(&profile, "watch-3");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Expired");
    let outcome: Value =
        serde_json::from_slice(&fs::read(updates.join("outcome.json")).unwrap()).unwrap();
    assert_eq!(outcome["success"], true, "{outcome}");
    profile.stop();
}

#[test]
fn recover_update_waits_quietly_while_the_helper_holds_the_install_lock() {
    let mut profile = Profile::new();
    fake_manager_bin(&profile);
    write_service(&profile);
    write_watch(&profile, "watch-4", unix_now());
    // Hold the same file lock as the update helper while the child runs recovery.
    let updates = profile.root.join("Shina/updates");
    let lock = fs::File::create(updates.join("install.lock")).unwrap();
    lock.lock().unwrap();
    let output = recover(&profile, "watch-4");
    drop(lock);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "HelperWorking"
    );
    assert!(
        !profile.root.join("calls.log").exists(),
        "a held lock must not start anything"
    );
    assert!(
        profile
            .root
            .join("Shina/updates/update-watch.json")
            .is_file()
    );
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
