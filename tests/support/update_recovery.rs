//! Update-watch recovery against the real `shina` binary. The platform service
//! manager is faked by putting `launchctl`/`systemctl` stubs on the child's PATH;
//! every other piece (watch record, install lock, owner lock, outcome file) is real.
use super::*;

/// A directory whose `launchctl`/`systemctl` stubs log every call to `calls.log`
/// and behave like the manager would: `launchctl print` reports "not loaded",
/// everything else succeeds.
fn fake_manager_bin(profile: &Profile) -> PathBuf {
    let bin = profile.root.join("fake-bin");
    fs::create_dir_all(&bin).unwrap();
    let log = profile
        .root
        .join("calls.log")
        .to_string_lossy()
        .into_owned();
    let launchctl = format!(
        "#!/bin/sh\necho \"launchctl $*\" >> '{}'\nif [ \"$1\" = print ]; then exit 1; fi\nexit 0\n",
        log
    );
    let systemctl = format!("#!/bin/sh\necho \"systemctl $*\" >> '{}'\nexit 0\n", log);
    for (name, body) in [("launchctl", launchctl), ("systemctl", systemctl)] {
        let path = bin.join(name);
        fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    bin
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
    assert!(calls.contains("bootstrap"), "{calls}");
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
    // A second pass finds nothing to do and never duplicates the outcome.
    let output = recover(&profile, "watch-2");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "Expired");
    let again: Value =
        serde_json::from_slice(&fs::read(profile.root.join("Shina/updates/outcome.json")).unwrap())
            .unwrap();
    assert_eq!(again, outcome);
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
    // flock on the install lock is the cross-process signal the helper holds.
    let updates = profile.root.join("Shina/updates");
    let python = Command::new("python3")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !python {
        eprintln!("skipping lock-held check: python3 unavailable");
        return;
    }
    let mut holder = Command::new("python3")
        .args([
            "-c",
            "import fcntl,sys,time; f=open(sys.argv[1],'w'); fcntl.flock(f,fcntl.LOCK_EX); time.sleep(30)",
            updates.join("install.lock").to_str().unwrap(),
        ])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let output = recover(&profile, "watch-4");
    let _ = holder.kill();
    let _ = holder.wait();
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
