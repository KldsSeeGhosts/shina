//! Opt-in per-user host service, ported from KSG cli/service.ts and cloud/bootService.ts.
//! The descriptor contains paths, never a snapshot of provider credentials.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

fn confirm_launcher(
    previous: Option<u32>,
    mut observe: impl FnMut() -> Option<u32>,
    mut alive: impl FnMut(u32) -> bool,
    mut pause: impl FnMut(),
) -> Result<(), String> {
    for _ in 0..100 {
        if let Some(pid) = observe()
            && Some(pid) != previous
            && alive(pid)
        {
            return Ok(());
        }
        pause();
    }
    Err("The new service launcher never started within 10 seconds. Task Scheduler may have ignored the start request. See service.log.".into())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Platform {
    Linux,
    Mac,
    Windows,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Service {
    pub platform: Platform,
    pub name: String,
    pub executable: PathBuf,
    pub directory: PathBuf,
    pub unit: PathBuf,
    pub home: PathBuf,
    pub environment: BTreeMap<String, String>,
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn unit_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    )
}

fn windows_task_arguments(executable: &str, arguments: &[&str]) -> String {
    let arguments = arguments
        .iter()
        .map(|argument| format!("'{}'", argument.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    // Start-Process -Wait waits for the entire descendant tree, including an
    // updater trying to restart this task. The launcher already waits for the
    // server, so wait for that one process and preserve its exit code.
    format!(
        "-NoProfile -NonInteractive -WindowStyle Hidden -Command \"$p = Start-Process -FilePath '{}' -ArgumentList {arguments} -WindowStyle Hidden -PassThru; $p.WaitForExit(); exit $p.ExitCode\"",
        executable.replace('\'', "''")
    )
}

#[cfg(windows)]
fn current_windows_user_sid() -> Result<String, String> {
    // USERDOMAIN may be WORKGROUP over OpenSSH, not the local account's
    // authority. Task Scheduler accepts the current token's stable SID.
    let output = crate::process::output(
        Command::new("whoami.exe").args(["/user", "/fo", "csv", "/nh"]),
        Duration::from_secs(5),
        8192,
    )
    .map_err(|_| "Could not resolve the current Windows user SID.")?;
    if !output.status.success() {
        return Err("Could not resolve the current Windows user SID.".into());
    }
    String::from_utf8_lossy(&output.stdout)
        .split('"')
        .find(|value| {
            value.starts_with("S-1-")
                && value.len() > 4
                && value[4..]
                    .split('-')
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(str::to_owned)
        .ok_or_else(|| "Missing current Windows user SID.".into())
}

impl Service {
    fn launcher_pid(&self) -> Option<u32> {
        fs::read_to_string(self.directory.join("service-launcher.pid"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// Upgrade the legacy tree-wide wait without changing task grants or settings.
    /// IgnoreNew can discard /Run until the old action exits, even after both Shina
    /// processes have gone. COM state numbers do not depend on locale.
    fn wait_windows_task_idle(&self) -> Result<(), String> {
        let arguments = windows_task_arguments(
            &self.executable.to_string_lossy(),
            &[
                "--service-run",
                &format!("\\\"{}\\\"", self.descriptor().display()),
            ],
        );
        let legacy = arguments.replace("-PassThru; $p.WaitForExit();", "-PassThru -Wait;");
        let name = self.name.replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference='Stop'; $s=New-Object -ComObject Schedule.Service; $s.Connect(); $f=$s.GetFolder('\\'); $t=$f.GetTask('{name}'); $d=$t.Definition; if ($d.Actions.Count -eq 1 -and $d.Actions.Item(1).Path -ieq 'powershell.exe' -and $d.Actions.Item(1).Arguments -ceq '{legacy}') {{ $d.Actions.Item(1).Arguments='{arguments}'; $null=$f.RegisterTaskDefinition('{name}',$d,4,$null,$null,3,$null) }}; $w=[Diagnostics.Stopwatch]::StartNew(); while ($t.State -eq 4 -or $t.State -eq 2) {{ if ($w.Elapsed.TotalSeconds -ge 15) {{ throw 'The previous service task did not exit within 15 seconds. No new start was requested.' }}; Start-Sleep -Milliseconds 100; $t=$f.GetTask('{name}') }}; if ($t.State -ne 3) {{ throw 'The service task is not ready to start.' }}",
            legacy = legacy.replace('\'', "''"),
            arguments = arguments.replace('\'', "''"),
        );
        self.run(
            "powershell.exe",
            &["-NoProfile", "-NonInteractive", "-Command", &script],
        )?;
        Ok(())
    }

    pub fn descriptor(&self) -> PathBuf {
        self.directory.join("service.json")
    }
    /// Where the server's output goes. macOS keeps it in ~/Library/Logs: launchd opens the
    /// file before Shina runs, and its sandbox refuses paths on other volumes, which a
    /// symlinked ~/.local can point at.
    pub fn log_path(&self) -> PathBuf {
        match self.platform {
            Platform::Mac => self
                .home
                .join("Library")
                .join("Logs")
                .join("Shina")
                .join("service.log"),
            _ => self.directory.join("service.log"),
        }
    }
    /// No provider secret or service.env content is included in any generated unit.
    pub fn render(&self) -> String {
        let exe = self.executable.to_string_lossy();
        let descriptor = self.descriptor().to_string_lossy().into_owned();
        let log = self.log_path().to_string_lossy().into_owned();
        let windows_arguments =
            windows_task_arguments(&exe, &["--service-run", &format!("\\\"{descriptor}\\\"")]);
        match self.platform {
            Platform::Linux => format!(
                "[Unit]\nDescription=Shina always-on server\nStartLimitIntervalSec=300\nStartLimitBurst=5\n\n[Service]\nType=simple\nWorkingDirectory=%h\nExecStart={} --service-run {}\nRestart=on-failure\nRestartSec=5\nKillMode=mixed\nTimeoutStopSec=90\nStandardOutput=append:{}\nStandardError=append:{}\n\n[Install]\nWantedBy=default.target\n",
                unit_quote(&exe),
                unit_quote(&descriptor),
                log.replace('%', "%%"),
                log.replace('%', "%%")
            ),
            Platform::Mac => format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--service-run</string><string>{}</string></array>\n<key>WorkingDirectory</key><string>{}</string>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n<key>ThrottleInterval</key><integer>5</integer>\n<key>ExitTimeOut</key><integer>90</integer>\n<key>ProcessType</key><string>Interactive</string>\n<key>StandardOutPath</key><string>{}</string>\n<key>StandardErrorPath</key><string>{}</string>\n</dict></plist>\n",
                xml(&self.name),
                xml(&exe),
                xml(&descriptor),
                xml(&self.home.to_string_lossy()),
                xml(&log),
                xml(&log)
            ),
            Platform::Windows => format!(
                "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n<RegistrationInfo><Description>Shina opt-in always-on server</Description></RegistrationInfo>\n<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{}</UserId></LogonTrigger></Triggers>\n<Principals><Principal id=\"User\"><UserId>{}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>\n<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Hidden>true</Hidden><RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings>\n<Actions Context=\"User\"><Exec><Command>powershell.exe</Command><Arguments>{}</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions>\n</Task>\n",
                xml(self
                    .environment
                    .get("SHINA_SERVICE_USER")
                    .map(String::as_str)
                    .unwrap_or("")),
                xml(self
                    .environment
                    .get("SHINA_SERVICE_USER")
                    .map(String::as_str)
                    .unwrap_or("")),
                xml(&windows_arguments),
                xml(&self.home.to_string_lossy())
            ),
        }
    }
    fn current(name: Option<String>) -> Result<Self, String> {
        let directory = crate::store::directory().ok_or("No data directory.")?;
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .ok_or("No home directory.")?;
        let name = name.unwrap_or_else(|| {
            use sha2::{Digest, Sha256};
            format!(
                "Shina-server-{:x}",
                Sha256::digest(directory.to_string_lossy().as_bytes())
            )[..33]
                .to_owned()
        });
        if !name.starts_with("Shina-")
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err("Service names must start with Shina- and contain only ASCII letters, digits and hyphens.".into());
        }
        let platform = if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Linux
        };
        let unit = match platform {
            Platform::Windows => directory.join("service-task.xml"),
            Platform::Mac => home
                .join("Library/LaunchAgents")
                .join(format!("{name}.plist")),
            Platform::Linux => std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("systemd/user")
                .join(format!("{name}.service")),
        };
        let mut environment = BTreeMap::new();
        // Reconcile discovery paths like KSG. Authentication stays in the host's own files
        // or inherited environment. Do not serialize arbitrary environment variables.
        for key in [
            "PATH",
            "HOME",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "PI_CODING_AGENT_DIR",
            "CODEX_HOME",
            "SHINA_PI_CLI",
            "STILL_PI_CLI",
        ] {
            if let Ok(value) = std::env::var(key) {
                environment.insert(key.into(), value);
            }
        }
        #[cfg(windows)]
        {
            environment.insert("SHINA_SERVICE_USER".into(), current_windows_user_sid()?);
        }
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        if [&directory, &home, &unit, &executable]
            .into_iter()
            .any(|path| path.to_string_lossy().chars().any(char::is_control))
        {
            return Err("Service paths cannot contain control characters.".into());
        }
        Ok(Self {
            platform,
            name,
            executable,
            directory,
            unit,
            home,
            environment,
        })
    }
    pub fn load() -> Result<Option<Self>, String> {
        let directory = crate::store::directory().ok_or("No data directory.")?;
        Self::load_from(&directory)
    }
    /// Loads the descriptor of an explicitly named profile. The update watch runs with
    /// `--profile` so a non-default data location can never resolve another profile's
    /// service, credentials or state.
    pub fn load_from(directory: &Path) -> Result<Option<Self>, String> {
        let path = directory.join("service.json");
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        // An explicitly supplied fixture directory must not fall back to the
        // machine's real service manager if an override is missing.
        let program = std::env::var_os("SHINA_SERVICE_BIN_DIR")
            .map(|dir| PathBuf::from(dir).join(program))
            .unwrap_or_else(|| PathBuf::from(program));
        let output = crate::process::output(
            Command::new(&program).args(args).stdin(Stdio::null()),
            Duration::from_secs(30),
            64 * 1024,
        )
        .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "{} failed with {}. No elevation was requested.\n{}",
                program.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    fn domain(&self) -> Result<String, String> {
        Ok(format!("gui/{}", self.run("id", &["-u"])?.trim()))
    }
    pub fn start(&self) -> Result<(), String> {
        match self.platform {
            Platform::Linux => {
                self.run(
                    "systemctl",
                    &["--user", "start", &format!("{}.service", self.name)],
                )?;
            }
            Platform::Mac => {
                let domain = self.domain()?;
                let target = format!("{domain}/{}", self.name);
                if self.run("launchctl", &["print", &target]).is_ok() {
                    self.run("launchctl", &["kickstart", &target])?;
                } else {
                    self.run(
                        "launchctl",
                        &["bootstrap", &domain, &self.unit.to_string_lossy()],
                    )?;
                }
            }
            Platform::Windows => {
                self.wait_windows_task_idle()?;
                let previous = self.launcher_pid();
                self.run("schtasks.exe", &["/Run", "/TN", &self.name])?;
                confirm_launcher(
                    previous,
                    || self.launcher_pid(),
                    crate::process::is_alive,
                    || std::thread::sleep(Duration::from_millis(100)),
                )?;
            }
        }
        Ok(())
    }
    /// The update-watch job name on Windows and the LaunchAgent label on macOS. Linux
    /// uses the fixed `shina-update-watch` unit pair instead, matching the contract.
    fn watch_name(&self) -> String {
        format!("{}.update-watch", self.name)
    }
    /// macOS: a LaunchAgent that runs recover-update every 60 seconds. No RunAtLoad:
    /// the watch must not fire simply because the user logged in. The job executes the
    /// stable helper copy and names its profile explicitly, so neither a mid-swap
    /// missing binary nor a non-default data location can strand it.
    fn watch_plist(&self, helper: &Path, update_id: &str, profile: &Path) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>service</string><string>recover-update</string><string>{}</string><string>--profile</string><string>{}</string></array>\n<key>StartInterval</key><integer>{}</integer>\n<key>ProcessType</key><string>Background</string>\n</dict></plist>\n",
            xml(&self.watch_name()),
            xml(&helper.to_string_lossy()),
            xml(update_id),
            xml(&profile.to_string_lossy()),
            crate::update::WATCH_INTERVAL_SECS
        )
    }
    /// Linux: a oneshot service plus the 60 second user timer that retriggers it.
    fn watch_units(&self, helper: &Path, update_id: &str, profile: &Path) -> (String, String) {
        let service = format!(
            "[Unit]\nDescription=Shina update recovery after a restart handoff\n\n[Service]\nType=oneshot\nExecStart={} service recover-update {} --profile {}\n",
            unit_quote(&helper.to_string_lossy()),
            unit_quote(update_id),
            unit_quote(&profile.to_string_lossy())
        );
        let timer = format!(
            "[Unit]\nDescription=Shina update recovery watch\n\n[Timer]\nOnActiveSec={}\nOnUnitActiveSec={}\n\n[Install]\nWantedBy=timers.target\n",
            crate::update::WATCH_INTERVAL_SECS,
            crate::update::WATCH_INTERVAL_SECS
        );
        (service, timer)
    }
    /// Windows: a one-time task repeating every minute for 45 minutes.
    fn watch_task_xml(
        &self,
        helper: &Path,
        update_id: &str,
        profile: &Path,
        start_boundary: &str,
    ) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<Task version=\"1.4\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n<RegistrationInfo><Description>Restarts the Shina service if an update helper did not finish</Description></RegistrationInfo>\n<Triggers><TimeTrigger><Repetition><Interval>PT1M</Interval><Duration>PT45M</Duration></Repetition><StartBoundary>{}</StartBoundary><Enabled>true</Enabled></TimeTrigger></Triggers>\n<Principals><Principal id=\"User\"><UserId>{}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>\n<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT10M</ExecutionTimeLimit><Hidden>true</Hidden></Settings>\n<Actions Context=\"User\"><Exec><Command>{}</Command><Arguments>service recover-update \"{}\" --profile \"{}\"</Arguments><WorkingDirectory>{}</WorkingDirectory></Exec></Actions>\n</Task>\n",
            xml(start_boundary),
            xml(self
                .environment
                .get("SHINA_SERVICE_USER")
                .map(String::as_str)
                .unwrap_or("")),
            xml(&helper.to_string_lossy()),
            xml(update_id),
            xml(&profile.to_string_lossy()),
            xml(&self.home.to_string_lossy())
        )
    }
    fn watch_unit_dir(&self) -> PathBuf {
        self.home.join(".config").join("systemd").join("user")
    }
    fn watch_plist_path(&self) -> PathBuf {
        self.home
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{}.plist", self.watch_name()))
    }
    fn watch_task_path(&self) -> PathBuf {
        self.directory.join("update-watch.xml")
    }
    /// Registers the per-user watch: one recover-update run a minute for 45 minutes.
    /// Re-registering replaces the job. The service manager definitions themselves are
    /// unchanged, so crash-loop limits still apply and nothing races a binary swap.
    /// `helper` is a copy of this executable living outside the replaced install;
    /// `profile` is pinned into the job so it never resolves the wrong data directory.
    pub fn register_update_watch(
        &self,
        helper: &Path,
        update_id: &str,
        profile: &Path,
    ) -> Result<(), String> {
        match self.platform {
            Platform::Linux => {
                let dir = self.watch_unit_dir();
                fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let (service, timer) = self.watch_units(helper, update_id, profile);
                crate::store::write_atomic(
                    &dir.join(format!("{}.service", self.watch_name())),
                    service.as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                crate::store::write_atomic(
                    &dir.join(format!("{}.timer", self.watch_name())),
                    timer.as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                self.run("systemctl", &["--user", "daemon-reload"])?;
                self.run(
                    "systemctl",
                    &[
                        "--user",
                        "enable",
                        "--now",
                        &format!("{}.timer", self.watch_name()),
                    ],
                )?;
            }
            Platform::Mac => {
                let plist = self.watch_plist_path();
                if let Some(parent) = plist.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                crate::store::write_atomic(
                    &plist,
                    self.watch_plist(helper, update_id, profile).as_bytes(),
                )
                .map_err(|e| e.to_string())?;
                let domain = self.domain()?;
                let target = format!("{domain}/{}", self.watch_name());
                // A leftover watch for a previous update is replaced, never duplicated.
                let _ = self.run("launchctl", &["bootout", &target]);
                self.run(
                    "launchctl",
                    &["bootstrap", &domain, &plist.to_string_lossy()],
                )?;
            }
            Platform::Windows => {
                let start = (chrono::Local::now() + chrono::Duration::minutes(1))
                    .format("%Y-%m-%dT%H:%M:%S")
                    .to_string();
                let rendered = self.watch_task_xml(helper, update_id, profile, &start);
                let bytes: Vec<u8> = std::iter::once(0xfeffu16)
                    .chain(rendered.encode_utf16())
                    .flat_map(u16::to_le_bytes)
                    .collect();
                crate::store::write_atomic(&self.watch_task_path(), &bytes)
                    .map_err(|e| e.to_string())?;
                self.run(
                    "schtasks.exe",
                    &[
                        "/Create",
                        "/TN",
                        &format!("{}-update-watch", self.name),
                        "/XML",
                        &self.watch_task_path().to_string_lossy(),
                        "/F",
                    ],
                )?;
            }
        }
        Ok(())
    }
    /// Removes the watch. Missing jobs or files are not errors.
    pub fn unregister_update_watch(&self) -> Result<(), String> {
        match self.platform {
            Platform::Linux => {
                let _ = self.run(
                    "systemctl",
                    &[
                        "--user",
                        "disable",
                        "--now",
                        &format!("{}.timer", self.watch_name()),
                    ],
                );
                for name in [
                    format!("{}.timer", self.watch_name()),
                    format!("{}.service", self.watch_name()),
                ] {
                    match fs::remove_file(self.watch_unit_dir().join(name)) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.to_string()),
                    }
                }
                let _ = self.run("systemctl", &["--user", "daemon-reload"]);
            }
            Platform::Mac => {
                let _ = self.run(
                    "launchctl",
                    &[
                        "bootout",
                        &format!("{}/{}", self.domain()?, self.watch_name()),
                    ],
                );
                match fs::remove_file(self.watch_plist_path()) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            Platform::Windows => {
                let _ = self.run(
                    "schtasks.exe",
                    &[
                        "/Delete",
                        "/TN",
                        &format!("{}-update-watch", self.name),
                        "/F",
                    ],
                );
                match fs::remove_file(self.watch_task_path()) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
        }
        Ok(())
    }
    /// Authenticate and ask the runtime to shut down before stopping its manager.
    pub fn stop(&self) -> Result<(), String> {
        if crate::server_lock::Owner::acquire(&self.directory).is_err() {
            crate::server_auth::Discovery::read(&self.directory).map_err(|_| {
                "The profile is owned but its server is not ready. No process was terminated."
                    .to_string()
            })?;
            crate::maintenance::rpc(&self.directory, "server.stop", serde_json::json!({}))?;
            crate::maintenance::wait_unowned(&self.directory, Duration::from_secs(10))?;
        }
        match self.platform {
            Platform::Linux => {
                self.run(
                    "systemctl",
                    &["--user", "stop", &format!("{}.service", self.name)],
                )?;
            }
            Platform::Mac => {
                let _ = self.run(
                    "launchctl",
                    &["bootout", &format!("{}/{}", self.domain()?, self.name)],
                );
            }
            // A normally exited task needs no /End, which could kill a concurrent new run.
            Platform::Windows => {}
        }
        Ok(())
    }
}

/// What one `service recover-update` pass decided. The watch's own logging is the
/// only reader, so the variant names double as its audit trail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoverStep {
    /// The update helper still holds the install lock. The watch waits quietly.
    HelperWorking,
    /// A server answers server.status, so the service came back. Watch removed.
    Healthy,
    /// A server owns the profile but does not answer yet. Wait; never start a second.
    NotReady,
    /// The service was stopped and no helper was running. Started through the manager.
    Started,
    /// The 45 minute watch window ended with no return: a failed outcome was recorded
    /// if the helper left none, and the watch was removed.
    Expired,
    /// The invoked update id or service does not match the registered watch, or no
    /// watch is registered at all. Pure no-op: a newer or different watch must never
    /// be unregistered, started for or written an outcome by a stale invocation.
    Stale,
}

/// The profile is owned and its authenticated server answers. An `Owner::acquire`
/// failure alone is not proof of health. Do not require an OS process handle here:
/// Windows can deny the least-privilege watch access to a server started by an
/// elevated shell even though the same user's authenticated RPC succeeds.
fn service_ready(directory: &Path) -> bool {
    if crate::server_lock::Owner::acquire(directory).is_ok() {
        return false;
    }
    // local_rpc checks the profile's discovery, credentials, server and instance
    // identity. A response proves liveness without crossing a privilege boundary.
    crate::maintenance::rpc(directory, "server.status", serde_json::json!({})).is_ok()
}

/// One pass of the update watch (design contract 9.2). The per-user job that the
/// server registered at the restart handoff runs this every minute until the watch
/// removes itself. `profile` is the data directory the job was registered for; the
/// job always passes it explicitly so environment resolution can never redirect it.
pub fn recover_update(update_id: &str, profile: Option<&Path>) -> Result<RecoverStep, String> {
    let directory = match profile {
        Some(profile) => profile.to_path_buf(),
        None => crate::store::directory().ok_or("No data directory.")?,
    };
    let Some(service) = Service::load_from(&directory)? else {
        return Err("No Shina server service is installed.".into());
    };
    if service
        .directory
        .canonicalize()
        .map_err(|e| e.to_string())?
        != directory.canonicalize().map_err(|e| e.to_string())?
    {
        return Ok(RecoverStep::Stale);
    }
    // Exact identity fence before any side effect: the record's update id must be the
    // invoked id and the record's service must be this service.
    let Some(watch) = crate::update::update_watch(&directory) else {
        // A job without a record has no authority to start or remove anything.
        return Ok(RecoverStep::Stale);
    };
    if watch.update_id != update_id || watch.service != service.name {
        return Ok(RecoverStep::Stale);
    }
    // A held install lock means the helper is alive and owns the handoff, including a
    // deliberate mid-update stop. The watch must not fight it.
    let Some(_lock) = crate::install::InstallLock::try_acquire(&directory.join("updates"))? else {
        return Ok(RecoverStep::HelperWorking);
    };
    // Registration and operator stop use this same lock. Recheck after acquiring it.
    if !crate::update::update_watch(&directory).is_some_and(|current| {
        current.update_id == watch.update_id
            && current.service == watch.service
            && current.registered_at == watch.registered_at
            && current.tag == watch.tag
    }) {
        return Ok(RecoverStep::Stale);
    }
    let mut journal = crate::update::read_update_journal(&directory)?;
    if journal.as_ref().is_some_and(|journal| {
        journal.update_id != watch.update_id
            || journal.service.as_deref() != Some(service.name.as_str())
            || journal.tag != watch.tag
    }) {
        return Ok(RecoverStep::Stale);
    }
    let held = directory.join("update-validation").exists();
    if crate::server_lock::Owner::acquire(&directory).is_err() {
        if held {
            // A server in the dispatch hold owns state but has not been validated.
            // Finish the journal's work instead of mistaking ownership for health.
            return match crate::update::reconcile_held_service(&service, &directory, &watch)? {
                crate::update::HeldRecovery::Validated => {
                    crate::update::unregister_update_watch_for(&service, &directory)?;
                    Ok(RecoverStep::Healthy)
                }
                crate::update::HeldRecovery::RolledBack => {
                    service.start()?;
                    Ok(RecoverStep::Started)
                }
            };
        }
        if service_ready(&directory) {
            let running = crate::server_auth::Discovery::read(&directory)?.binary_version;
            let previous = if let Some(journal) = &journal {
                let previous = crate::update::recovery_install_is_previous(&service, journal)?;
                if previous && running != journal.previous_version {
                    return Ok(RecoverStep::NotReady);
                }
                previous
            } else {
                false
            };
            let outcome = crate::fleet_update::read_outcome(&directory);
            if journal.is_some()
                || outcome
                    .as_ref()
                    .is_none_or(|outcome| outcome.update_id != update_id)
            {
                let success = !previous && running == watch.tag.trim_start_matches('v');
                crate::update::record_recovered_outcome(
                    &directory, update_id, &watch.tag, success,
                    (!success).then(|| "The update helper stopped before the target version was validated. The service is running its previous version.".into()),
                )?;
            }
            crate::update::clear_journal(&directory);
            crate::update::unregister_update_watch_for(&service, &directory)?;
            return Ok(RecoverStep::Healthy);
        }
        return Ok(RecoverStep::NotReady);
    }
    let expired = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|now| now.as_secs())
        .unwrap_or(0)
        >= watch
            .registered_at
            .saturating_add(crate::update::WATCH_EXPIRY.as_secs());
    if held && journal.is_none() {
        return Err("The update dispatch hold has no matching recovery journal. The service remains stopped rather than opening an unverified database.".into());
    }
    if expired {
        // The watch is about to remove itself, so finish any owed rollback first:
        // an unvalidated binary must never be left holding dispatch.
        if let Some(journal) = &mut journal {
            crate::update::restore_interrupted_update(&service, &directory, journal)?;
            // The matching previous build is safe to start even though the update failed.
            service.start()?;
        }
        if crate::fleet_update::read_outcome(&directory)
            .map(|outcome| outcome.update_id)
            .as_deref()
            != Some(update_id)
        {
            crate::fleet_update::save_outcome(
                &directory,
                &crate::fleet_update::Outcome {
                    update_id: update_id.into(),
                    tag: watch.tag.clone(),
                    success: false,
                    reason: Some(
                        "The service did not come back after the update. Start it with `shina service start`."
                            .into(),
                    ),
                },
            )?;
        }
        crate::update::unregister_update_watch_for(&service, &directory)?;
        return Ok(RecoverStep::Expired);
    }
    if let Some(journal) = &mut journal {
        let previous = crate::update::recovery_install_is_previous(&service, journal)?;
        let rollback_in_flight = journal.phase != crate::update::JournalPhase::Replaced;
        // A swap interrupted between the old binary moving out and the new one
        // landing leaves no executable to start. In both cases the matched binary
        // and database go back together before any start.
        if rollback_in_flight || previous || !service.executable.is_file() {
            crate::update::restore_interrupted_update(&service, &directory, journal)?;
            service.start()?;
            return Ok(RecoverStep::Started);
        }
        // Replaced and unowned with an executable in place: start it. If the hold
        // marker exists the server holds dispatch until the next tick validates it.
        if !held {
            crate::store::write_atomic(
                &directory.join("update-validation"),
                b"hold until update recovery validates readiness",
            )
            .map_err(|e| e.to_string())?;
        }
        service.start()?;
        return Ok(RecoverStep::Started);
    }
    if !service.executable.is_file() {
        return Err("The service executable is missing and no matching update journal can prove a safe rollback.".into());
    }
    service.start()?;
    Ok(RecoverStep::Started)
}

pub fn cli(args: &[String]) -> Result<(), String> {
    let action = args.first().map(String::as_str).unwrap_or("status");
    if action == "recover-update" {
        let mut update_id = None;
        let mut profile = None;
        let mut iter = args.iter().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--profile" => {
                    profile = Some(PathBuf::from(
                        iter.next().ok_or("--profile requires a value.")?,
                    ))
                }
                _ if update_id.is_none() => update_id = Some(arg.clone()),
                _ => {
                    return Err(
                        "Usage: shina service recover-update <updateId> [--profile <dir>]".into(),
                    );
                }
            }
        }
        let update_id =
            update_id.ok_or("Usage: shina service recover-update <updateId> [--profile <dir>]")?;
        let step = recover_update(&update_id, profile.as_deref())?;
        println!("{step:?}");
        return Ok(());
    }
    let mut name = None;
    let mut linger = false;
    let mut tailnet = false;
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--enable-linger" if action == "install" => linger = true,
            "--tailnet" if action == "install" => tailnet = true,
            "--name" if action == "install" => name = Some(iter.next().ok_or("--name requires a value.")?.clone()),
            _ => return Err("Usage: shina service install [--tailnet] [--enable-linger] [--name Shina-name] | uninstall | status | start | stop".into()),
        }
    }
    let existing = Service::load()?;
    if action == "install" {
        let mut service =
            Service::current(name.or_else(|| existing.as_ref().map(|old| old.name.clone())))?;
        if tailnet {
            service
                .environment
                .insert("SHINA_SERVICE_TAILNET".into(), "1".into());
        }
        if let Some(old) = &existing {
            if old.name != service.name {
                return Err("Uninstall the existing service before changing its name.".into());
            }
            old.stop()?;
        }
        if matches!(service.platform, Platform::Linux) {
            if linger {
                println!(
                    "Enabling lingering for this user so the server can run without a login. No sudo or password prompt will be used."
                );
                service.run("loginctl", &["enable-linger", "--no-ask-password"])?;
            } else {
                println!(
                    "Lingering was not changed. Without it the user service may stop at logout. Opt in with --enable-linger."
                );
            }
        } else if linger {
            return Err("--enable-linger is Linux-only.".into());
        }
        fs::create_dir_all(&service.directory).map_err(|e| e.to_string())?;
        // launchd opens the log itself and does not create missing folders.
        if let Some(parent) = service.log_path().parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        crate::server_auth::protect(&service.directory)?;
        fs::create_dir_all(service.unit.parent().ok_or("No service directory.")?)
            .map_err(|e| e.to_string())?;
        crate::store::write_atomic(
            &service.descriptor(),
            &serde_json::to_vec_pretty(&service).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let rendered = service.render();
        let bytes = if matches!(service.platform, Platform::Windows) {
            std::iter::once(0xfeffu16)
                .chain(rendered.encode_utf16())
                .flat_map(u16::to_le_bytes)
                .collect()
        } else {
            rendered.into_bytes()
        };
        crate::store::write_atomic(&service.unit, &bytes).map_err(|e| e.to_string())?;
        match service.platform {
            Platform::Linux => {
                service.run("systemctl", &["--user", "daemon-reload"])?;
                service.run(
                    "systemctl",
                    &["--user", "enable", &format!("{}.service", service.name)],
                )?;
            }
            Platform::Mac => {}
            Platform::Windows => {
                service.run(
                    "schtasks.exe",
                    &[
                        "/Create",
                        "/TN",
                        &service.name,
                        "/XML",
                        &service.unit.to_string_lossy(),
                        "/F",
                    ],
                )?;
            }
        }
        service.start()?;
        println!(
            "Installed opt-in server service {}. Desktop login startup remains off.",
            service.name
        );
        return Ok(());
    }
    let Some(service) = existing else {
        if action == "status" || action == "uninstall" {
            println!("Shina server service is not installed.");
            return Ok(());
        }
        return Err("Run shina service install first.".into());
    };
    match action {
        "start" => service.start()?,
        // An explicit operator stop overrides a registered update watch; the helper's
        // own internal Service::stop() mid-update does not come through this path.
        "stop" => {
            let _lock = crate::install::InstallLock::acquire(&service.directory.join("updates"))?;
            service.stop()?;
            crate::update::disarm_update_watch(&service.directory);
        }
        "status" => {
            let state = match service.platform {
                Platform::Linux => service.run(
                    "systemctl",
                    &["--user", "status", &format!("{}.service", service.name)],
                ),
                Platform::Mac => service.run(
                    "launchctl",
                    &["print", &format!("{}/{}", service.domain()?, service.name)],
                ),
                Platform::Windows => service.run(
                    "schtasks.exe",
                    &["/Query", "/TN", &service.name, "/FO", "LIST"],
                ),
            };
            println!(
                "Service {}\nExecutable {}\nProfile {}\n{}",
                service.name,
                service.executable.display(),
                service.directory.display(),
                state.unwrap_or_else(|e| e)
            );
        }
        "uninstall" => {
            let _lock = crate::install::InstallLock::acquire(&service.directory.join("updates"))?;
            // A deleted service must not leave a watch job invoking recover-update.
            service.stop()?;
            crate::update::disarm_update_watch(&service.directory);
            match service.platform {
                Platform::Linux => {
                    service.run(
                        "systemctl",
                        &["--user", "disable", &format!("{}.service", service.name)],
                    )?;
                }
                Platform::Windows => {
                    service.run("schtasks.exe", &["/Delete", "/TN", &service.name, "/F"])?;
                }
                Platform::Mac => {}
            }
            fs::remove_file(&service.unit).map_err(|e| e.to_string())?;
            fs::remove_file(service.descriptor()).map_err(|e| e.to_string())?;
            if matches!(service.platform, Platform::Linux) {
                service.run("systemctl", &["--user", "daemon-reload"])?;
            }
            println!(
                "Removed service {}. Server data and lingering settings were kept.",
                service.name
            );
        }
        _ => {
            return Err(
                "Unknown service command. Use install, uninstall, status, start or stop.".into(),
            );
        }
    }
    Ok(())
}

/// KSG serviceLauncher reads the host's existing service.env at launch, not at installation.
pub fn launch(path: &Path) -> Result<(), String> {
    let bytes = match fs::read(path) {
        Err(error)
            if cfg!(target_os = "macos")
                && error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            // launchd restarts a failed job after 5 seconds, and every start raises the macOS
            // privacy prompt again. Wait so a denied job asks once a minute, not 12 times.
            eprintln!(
                "macOS denied Shina access to {}. Allow Shina in System Settings, Privacy & Security, Files and Folders. Retrying in 60 seconds.",
                path.display()
            );
            std::thread::sleep(Duration::from_secs(60));
            return Err(error.to_string());
        }
        other => other.map_err(|e| e.to_string())?,
    };
    let service: Service = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let log_path = service.log_path();
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    writeln!(log, "Service launcher {} started.", std::process::id()).map_err(|e| e.to_string())?;
    let result = launch_server(&service, &log);
    if let Err(error) = &result {
        let _ = writeln!(log, "Service server failed: {error}");
    }
    result
}

fn launch_server(service: &Service, log: &fs::File) -> Result<(), String> {
    let mut command = Command::new(&service.executable);
    command
        .arg("serve")
        .current_dir(&service.home)
        .envs(&service.environment)
        .env("SHINA_BOOT_SERVICE", "1")
        .env_remove("SHINA_DESKTOP_OWNER_PID")
        .stdin(Stdio::null());
    // On macOS/Linux, `main()` reconciles PATH from the login shell before spawning;
    // keep the reconciled PATH rather than overwriting it with a stale minimal PATH
    // from an older service.json.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if let Some(reconciled_path) = std::env::var_os("PATH") {
        let stale = service.environment.get("PATH").map(String::as_str);
        if let Some(merged) = crate::shell_env::merge_path_entries(reconciled_path.to_str(), stale)
        {
            command.env("PATH", merged);
        }
    }
    if service
        .environment
        .get("SHINA_SERVICE_TAILNET")
        .is_some_and(|value| value == "1")
    {
        command.arg("--tailnet");
    }
    match fs::read_to_string(service.directory.join("service.env")) {
        Ok(text) => {
            for line in text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
            {
                let line = line.strip_prefix("export ").unwrap_or(line);
                let (key, value) = line
                    .split_once('=')
                    .ok_or("Invalid service.env assignment.")?;
                let key = key.trim();
                if key.is_empty()
                    || !key.bytes().enumerate().all(|(i, c)| {
                        c == b'_' || c.is_ascii_alphabetic() || i > 0 && c.is_ascii_digit()
                    })
                {
                    return Err("Invalid service.env key.".into());
                }
                if ![
                    "LOCALAPPDATA",
                    "XDG_DATA_HOME",
                    "SHINA_BOOT_SERVICE",
                    "SHINA_SERVICE_USER",
                ]
                .contains(&key)
                {
                    command.env(key, value.trim().trim_matches(['\'', '"']));
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    command
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log.try_clone().map_err(|e| e.to_string())?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let launcher_pid = service.directory.join("service-launcher.pid");
    crate::store::write_atomic(&launcher_pid, std::process::id().to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    let status = (|| {
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        crate::store::write_atomic(
            &service.directory.join("server-started.pid"),
            child.id().to_string().as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        // A registered update watch becomes stale only once this server proves it
        // answers server.status for its own profile. Removing it earlier would
        // strip the second line if this launch fails or stalls, and a server in
        // the update's dispatch hold never clears it because the journal gate in
        // remove_stale_update_watch keeps the watch armed.
        let directory = service.directory.clone();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(120);
            while std::time::Instant::now() < deadline {
                if service_ready(&directory) {
                    crate::update::remove_stale_update_watch(&directory);
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
        child.wait().map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(launcher_pid);
    let status = status?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Server exited with {status}. See {}.",
            service.log_path().display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owned_profile_without_an_authenticated_server_is_not_ready() {
        let directory = crate::store::directory().unwrap();
        assert!(!service_ready(&directory));
        let _owner = crate::server_lock::Owner::acquire(&directory).unwrap();
        assert!(!service_ready(&directory));
    }

    /// Opt-in native service-manager check. It creates only a uniquely named
    /// temporary watch, never an installed server service.
    #[test]
    #[ignore = "requires SHINA_NATIVE_WATCH_BINARY and a signed-in user service manager"]
    fn native_update_watch_registers_runs_and_cleans_its_explicit_profile() {
        let binary = PathBuf::from(
            std::env::var_os("SHINA_NATIVE_WATCH_BINARY")
                .expect("set SHINA_NATIVE_WATCH_BINARY to this commit's normal Shina executable"),
        );
        assert!(binary.is_file());
        let root = crate::store::directory().unwrap().join("native-watch");
        let profile = root.join("Shina");
        fs::create_dir_all(&profile).unwrap();
        let name = format!("Shina-native-watch-qa-{}", uuid::Uuid::new_v4());
        let mut service = Service::current(Some(name)).unwrap();
        service.directory = profile.clone();
        service.executable = binary.clone();
        fs::write(
            profile.join("service.json"),
            serde_json::to_vec(&service).unwrap(),
        )
        .unwrap();
        let mut command = Command::new(&binary);
        command
            .arg("serve")
            .env("XDG_DATA_HOME", &root)
            .env("LOCALAPPDATA", &root)
            .env("SHINA_PI_CLI", root.join("no-provider"))
            .env("SHINA_CODEX_CLI", root.join("no-provider"))
            .env("SHINA_CLAUDE_CLI", root.join("no-provider"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(root.join("server.log")).unwrap());
        let child = crate::process::Process::spawn(&mut command).unwrap();
        struct Cleanup {
            service: Service,
            profile: PathBuf,
            child: crate::process::Process,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = crate::update::unregister_update_watch_for(&self.service, &self.profile);
                self.child.terminate();
                let _ = self.child.reap(Duration::from_secs(3));
            }
        }
        let cleanup = Cleanup {
            service,
            profile,
            child,
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !service_ready(&cleanup.profile) {
            assert!(
                std::time::Instant::now() < deadline,
                "temporary server did not become ready"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
        let id = uuid::Uuid::new_v4().to_string();
        crate::update::register_update_watch(&cleanup.profile, &id, env!("CARGO_PKG_VERSION"))
            .unwrap();
        match cleanup.service.platform {
            Platform::Mac => {
                let target = format!(
                    "{}/{}",
                    cleanup.service.domain().unwrap(),
                    cleanup.service.watch_name()
                );
                assert!(
                    cleanup
                        .service
                        .run("launchctl", &["print", &target])
                        .is_ok()
                );
                cleanup
                    .service
                    .run("launchctl", &["kickstart", &target])
                    .unwrap();
            }
            Platform::Linux => {
                cleanup
                    .service
                    .run(
                        "systemctl",
                        &[
                            "--user",
                            "is-active",
                            &format!("{}.timer", cleanup.service.watch_name()),
                        ],
                    )
                    .unwrap();
                cleanup
                    .service
                    .run(
                        "systemctl",
                        &[
                            "--user",
                            "start",
                            &format!("{}.service", cleanup.service.watch_name()),
                        ],
                    )
                    .unwrap();
            }
            Platform::Windows => {
                let task = format!("{}-update-watch", cleanup.service.name);
                cleanup
                    .service
                    .run("schtasks.exe", &["/Query", "/TN", &task, "/XML"])
                    .unwrap();
                cleanup
                    .service
                    .run("schtasks.exe", &["/Run", "/TN", &task])
                    .unwrap();
            }
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let outcome = crate::fleet_update::read_outcome(&cleanup.profile);
            if outcome
                .as_ref()
                .is_some_and(|outcome| outcome.update_id == id && outcome.success)
                && crate::update::update_watch(&cleanup.profile).is_none()
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                let direct = Command::new(&binary)
                    .args(["service", "recover-update", &id, "--profile"])
                    .arg(&cleanup.profile)
                    .output();
                let task = format!("{}-update-watch", cleanup.service.name);
                let diagnostics = if cfg!(windows) {
                    cleanup.service.run(
                        "schtasks.exe",
                        &["/Query", "/TN", &task, "/V", "/FO", "LIST"],
                    )
                } else {
                    Ok("See the native service-manager log.".into())
                };
                panic!(
                    "native watch did not run against its pinned profile; direct={direct:?}, helperExists={}, watchPresent={}, outcome={outcome:?}, manager={diagnostics:?}, profile={}",
                    cleanup.profile.join("updates/watch-helper.exe").exists(),
                    crate::update::update_watch(&cleanup.profile).is_some(),
                    cleanup.profile.display()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            service_ready(&cleanup.profile),
            "watch stopped an already healthy server"
        );
    }

    #[cfg(windows)]
    #[test]
    fn scheduled_tasks_use_the_current_token_sid_not_environment_account_names() {
        let sid = current_windows_user_sid().unwrap();
        let service = Service::current(Some("Shina-sid-test".into())).unwrap();
        assert_eq!(service.environment["SHINA_SERVICE_USER"], sid);
        assert!(
            service
                .render()
                .contains(&format!("<UserId>{sid}</UserId>"))
        );
    }

    #[test]
    fn start_confirmation_requires_a_changed_live_launcher() {
        let mut pids = [Some(10), None, Some(20), Some(30)].into_iter();
        confirm_launcher(Some(10), || pids.next().flatten(), |pid| pid == 30, || {}).unwrap();
        for pid in [None, Some(10), Some(20)] {
            let error = confirm_launcher(Some(10), || pid, |pid| pid == 10, || {}).unwrap_err();
            assert!(error.contains("new service launcher never started"));
            assert!(error.contains("ignored"));
        }
    }
    fn fixture(platform: Platform) -> Service {
        Service {
            platform,
            name: "Shina-test".into(),
            executable: "/opt/Shina/shina".into(),
            directory: "/data/Shina".into(),
            unit: "/units/Shina".into(),
            home: "/home/test".into(),
            environment: [("SHINA_SERVICE_USER".into(), "HOST\\tester".into())].into(),
        }
    }
    #[test]
    fn generated_services_match_goldens_without_installing() {
        for (platform, golden) in [
            (
                Platform::Linux,
                include_str!("../tests/fixtures/service/linux.service"),
            ),
            (
                Platform::Mac,
                include_str!("../tests/fixtures/service/macos.plist"),
            ),
            (
                Platform::Windows,
                include_str!("../tests/fixtures/service/windows.xml"),
            ),
        ] {
            let service = fixture(platform);
            // Fixtures describe target-platform paths, independent of the test host.
            let rendered = service
                .render()
                .replace("/data/Shina\\\\", "/data/Shina/")
                .replace("/data/Shina\\", "/data/Shina/")
                .replace(r"\Library\Logs\Shina\", "/Library/Logs/Shina/");
            assert_eq!(rendered.replace("\r\n", "\n"), golden.replace("\r\n", "\n"));
            assert!(!service.render().contains("CPA_API_KEY"));
            assert!(!service.render().contains("service.env"));
        }
    }
    #[test]
    fn escaping_does_not_allow_unit_or_xml_injection() {
        assert_eq!(xml("<>&\""), "&lt;&gt;&amp;&quot;");
        assert_eq!(unit_quote("a%$\"\\b"), "\"a%%$$\\\"\\\\b\"");
    }

    #[test]
    fn update_watch_jobs_match_the_contract_per_platform() {
        let id = "3f4f9d2e-7c2b-4f2d-9a0d-1f3e2c4b5a6d";
        let helper = Path::new("/data/Shina/updates/watch-helper");
        let profile = Path::new("/data/Shina");
        let mac = fixture(Platform::Mac);
        let plist = mac.watch_plist(helper, id, profile);
        assert!(plist.contains("<key>StartInterval</key><integer>60</integer>"));
        assert!(
            !plist.contains("RunAtLoad"),
            "the watch must not fire at login"
        );
        assert!(plist.contains("<string>Shina-test.update-watch</string>"));
        assert!(plist.contains(&format!("<string>{id}</string>")));
        assert!(plist.contains("<string>recover-update</string>"));
        // The job runs the stable helper copy and pins its profile, so a mid-swap
        // missing binary or a non-default data directory cannot strand it.
        assert!(plist.contains("<string>/data/Shina/updates/watch-helper</string>"));
        assert!(plist.contains("<string>--profile</string>"));
        assert!(plist.contains("<string>/data/Shina</string>"));

        let linux = fixture(Platform::Linux);
        let (service, timer) = linux.watch_units(helper, id, profile);
        assert!(service.contains("Type=oneshot"));
        assert!(service.contains(&format!(
            "ExecStart=\"/data/Shina/updates/watch-helper\" service recover-update \"{id}\" --profile \"/data/Shina\""
        )));
        assert!(timer.contains("OnActiveSec=60"));
        assert!(timer.contains("OnUnitActiveSec=60"));
        assert!(timer.contains("WantedBy=timers.target"));

        let windows = fixture(Platform::Windows);
        let task = windows.watch_task_xml(helper, id, profile, "2026-11-16T12:34:56");
        assert!(task.contains("<StartBoundary>2026-11-16T12:34:56</StartBoundary>"));
        assert!(task.contains("<Interval>PT1M</Interval>"));
        assert!(task.contains("<Duration>PT45M</Duration>"));
        assert!(task.contains("IgnoreNew"));
        assert!(task.contains(&format!(
            "recover-update \"{id}\" --profile \"/data/Shina\""
        )));
        assert!(task.contains("<UserId>HOST\\tester</UserId>"));
    }

    #[test]
    fn update_watch_paths_follow_the_service_owner() {
        let service = fixture(Platform::Linux);
        assert_eq!(service.watch_name(), "Shina-test.update-watch");
        assert_eq!(
            service.watch_unit_dir(),
            Path::new("/home/test/.config/systemd/user")
        );
        assert_eq!(
            service.watch_plist_path(),
            Path::new("/home/test/Library/LaunchAgents/Shina-test.update-watch.plist")
        );
        assert_eq!(
            service.watch_task_path(),
            Path::new("/data/Shina/update-watch.xml")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_task_waits_for_launcher_exit_not_its_descendants() {
        use std::os::windows::process::CommandExt;

        let directory =
            std::env::temp_dir().join(format!("shina service wait-'{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let script = directory.join("launcher.ps1");
        let descendant_pid = directory.join("descendant.pid");
        fs::write(
            &script,
            format!(
                "$child = Start-Process powershell.exe -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60' -WindowStyle Hidden -PassThru; [IO.File]::WriteAllText('{}', [string]$child.Id); exit 7",
                descendant_pid.to_string_lossy().replace('\'', "''")
            ),
        )
        .unwrap();
        let arguments = windows_task_arguments(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-File",
                &format!("\\\"{}\\\"", script.display()),
            ],
        );
        let mut command = Command::new("powershell.exe");
        command
            .raw_arg(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Keep the job owner alive until after the assertion, then terminate
        // the test's sleeping descendant even when an assertion fails.
        let mut process = crate::process::Process::spawn(&mut command).unwrap();
        let result = crate::process::wait_child(&mut process, Duration::from_secs(10));
        assert_eq!(
            result
                .expect("the wrapper must not wait for the descendant")
                .code(),
            Some(7),
            "the wrapper must preserve the launcher's exit code"
        );
        let pid: u32 = fs::read_to_string(&descendant_pid)
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            crate::process::is_alive(pid),
            "the descendant outlives the launcher and wrapper"
        );
        process.terminate();
        process.reap(Duration::from_secs(2)).unwrap();
        drop(process);
        fs::remove_dir_all(directory).unwrap();
    }
}
