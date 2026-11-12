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

impl Service {
    fn launcher_pid(&self) -> Option<u32> {
        fs::read_to_string(self.directory.join("service-launcher.pid"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    /// IgnoreNew can discard /Run until the PowerShell task action has exited, even
    /// after both Shina processes have gone. COM state numbers do not depend on locale.
    fn wait_windows_task_idle(&self) -> Result<(), String> {
        let script = format!(
            "$ErrorActionPreference='Stop'; $s=New-Object -ComObject Schedule.Service; $s.Connect(); $t=$s.GetFolder('\\').GetTask('{}'); $w=[Diagnostics.Stopwatch]::StartNew(); while ($t.State -eq 4 -or $t.State -eq 2) {{ if ($w.Elapsed.TotalSeconds -ge 15) {{ throw 'The previous service task did not exit within 15 seconds. No new start was requested.' }}; Start-Sleep -Milliseconds 100 }}; if ($t.State -ne 3) {{ throw 'The service task is not ready to start.' }}",
            self.name.replace('\'', "''")
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
        let windows_arguments = format!(
            "-NoProfile -NonInteractive -WindowStyle Hidden -Command \"$p = Start-Process -FilePath '{}' -ArgumentList '--service-run', '\\\"{}\\\"' -WindowStyle Hidden -PassThru -Wait; exit $p.ExitCode\"",
            exe.replace('\'', "''"),
            descriptor.replace('\'', "''")
        );
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
        if cfg!(windows) {
            environment.insert(
                "SHINA_SERVICE_USER".into(),
                format!(
                    "{}\\{}",
                    std::env::var("USERDOMAIN").unwrap_or_default(),
                    std::env::var("USERNAME").map_err(|e| e.to_string())?
                ),
            );
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
        let path = crate::store::directory()
            .ok_or("No data directory.")?
            .join("service.json");
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let output = crate::process::output(
            Command::new(program).args(args).stdin(Stdio::null()),
            Duration::from_secs(30),
            64 * 1024,
        )
        .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "{program} failed with {}. No elevation was requested.\n{}",
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

pub fn cli(args: &[String]) -> Result<(), String> {
    let action = args.first().map(String::as_str).unwrap_or("status");
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
        "stop" => service.stop()?,
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
            service.stop()?;
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
    let service: Service = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
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
}
