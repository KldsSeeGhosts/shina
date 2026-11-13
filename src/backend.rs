//! Harnesses are RPC subprocesses, never terminals. Pipes and process waits stay off the UI thread.
use rune::Waker;
use serde_json::{Value, json};
use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

pub enum Event {
    Record(Value),
    Error(String),
    Diagnostic(String),
    Exited,
}

static WORKERS: Mutex<Vec<thread::JoinHandle<()>>> = Mutex::new(Vec::new());

/// Only called after Rune's window closes. Releasing a session never waits on the UI thread.
pub fn shutdown_all() {
    let workers = std::mem::take(&mut *WORKERS.lock().expect("worker registry"));
    let deadline = Instant::now() + Duration::from_secs(3);
    for worker in workers {
        crate::process::join_for(worker, deadline.saturating_duration_since(Instant::now()));
    }
}

/// A thread's Shina MCP endpoint and bearer token. Pi additionally uses a bridge extension.
#[derive(Clone, Debug)]
pub struct HostLink {
    pub url: String,
    pub token: String,
    pub extension: PathBuf,
    pub access_mode: String,
}

pub struct Pi {
    commands: SyncSender<Value>,
    pub events: Receiver<Event>,
    stopped: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct SessionOwner {
    lock: Mutex<()>,
    pub(crate) quarantined: AtomicBool,
}

impl Drop for Pi {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

// A reconnect must wait off the UI thread until the previous owner has reaped Pi.
type SessionLocks = std::collections::HashMap<PathBuf, std::sync::Weak<Mutex<()>>>;
static SESSIONS: std::sync::LazyLock<Mutex<SessionLocks>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

fn acquire_session<'a>(
    lock: &'a Mutex<()>,
    stopped: &AtomicBool,
) -> Result<Option<std::sync::MutexGuard<'a, ()>>, String> {
    loop {
        if stopped.load(Ordering::Acquire) {
            return Ok(None);
        }
        match lock.try_lock() {
            Ok(guard) => return Ok(Some(guard)),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(
                    "The previous session worker panicked. Restart Shina before resuming.".into(),
                );
            }
            Err(std::sync::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// A server dispatches only its own subprocesses. Multi-server clients send commands to
/// the owning server, whose records use `local`. Never fall back to this host for a peer.
pub enum BackendHost {
    Local,
}

pub enum SessionSource {
    New,
    Resume(String),
    Fork(String),
}

impl BackendHost {
    pub fn resolve(machine: &str) -> Result<Self, String> {
        match machine {
            "local" => Ok(Self::Local),
            _ => Err(format!(
                "Machine {machine} is unavailable. Only local hosts are supported."
            )),
        }
    }

    pub fn start(
        &self,
        directory: PathBuf,
        source: SessionSource,
        wake: Waker,
        instructions: Option<String>,
        link: Option<HostLink>,
        owner: Arc<SessionOwner>,
    ) -> Pi {
        match self {
            Self::Local => Pi::start(directory, source, wake, instructions, link, false, owner),
        }
    }
}

struct Launch {
    workspace: PathBuf,
    session: Option<String>,
    fork_source: Option<String>,
    instructions: Option<String>,
    host: Option<HostLink>,
    ephemeral: bool,
    #[cfg(test)]
    session_directory: PathBuf,
}

impl Pi {
    /// ACP, Codex and Claude share Pi's bounded connection, owner and shutdown registry.
    pub(crate) fn worker(
        wake: Waker,
        owner: Arc<SessionOwner>,
        session_key: Option<PathBuf>,
        run: impl FnOnce(
            Receiver<Value>,
            &SyncSender<Event>,
            &Waker,
            &AtomicBool,
            &SessionOwner,
        ) -> Result<(), String>
        + Send
        + 'static,
    ) -> Self {
        let (commands, input) = mpsc::sync_channel(32);
        let (events, output) = mpsc::sync_channel(256);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let session_lock = session_key.map(|key| {
            let mut sessions = SESSIONS.lock().expect("session owners");
            sessions.retain(|_, owner| owner.strong_count() > 0);
            let lock = sessions
                .get(&key)
                .and_then(std::sync::Weak::upgrade)
                .unwrap_or_else(|| Arc::new(Mutex::new(())));
            sessions.insert(key, Arc::downgrade(&lock));
            lock
        });
        let worker = thread::spawn(move || {
            let result = (|| {
                let Some(_guard) = acquire_session(&owner.lock, &stop)? else {
                    return Ok(());
                };
                if owner.quarantined.load(Ordering::Acquire) {
                    return Err(
                        "The previous agent could not be reaped. Restart Shina before resuming."
                            .into(),
                    );
                }
                let _session = if let Some(lock) = &session_lock {
                    let Some(guard) = acquire_session(lock, &stop)? else {
                        return Ok(());
                    };
                    Some(guard)
                } else {
                    None
                };
                run(input, &events, &wake, &stop, &owner)
            })();
            publish(
                &events,
                &wake,
                match result {
                    Ok(()) => Event::Exited,
                    Err(error) => Event::Error(error),
                },
            );
        });
        let mut workers = WORKERS.lock().expect("worker registry");
        workers.retain(|worker| !worker.is_finished());
        workers.push(worker);
        Self {
            commands,
            events: output,
            stopped,
        }
    }
    #[cfg(test)]
    pub fn test_connection() -> (Self, Receiver<Value>, SyncSender<Event>) {
        Self::test_connection_with_capacity(256)
    }

    #[cfg(test)]
    pub fn test_connection_with_capacity(
        capacity: usize,
    ) -> (Self, Receiver<Value>, SyncSender<Event>) {
        let (commands, input) = mpsc::sync_channel(32);
        let (output, events) = mpsc::sync_channel(capacity);
        (
            Self {
                commands,
                events,
                stopped: Arc::new(AtomicBool::new(false)),
            },
            input,
            output,
        )
    }

    #[cfg(test)]
    pub fn start_with_instructions(
        workspace: PathBuf,
        session: Option<String>,
        wake: Waker,
        instructions: Option<String>,
        host: Option<HostLink>,
    ) -> Self {
        Self::start(
            workspace,
            session.map_or(SessionSource::New, SessionSource::Resume),
            wake,
            instructions,
            host,
            false,
            Arc::default(),
        )
    }

    /// Read provider metadata without creating a saved conversation or exposing Shina tools.
    pub fn discover(workspace: PathBuf, wake: Waker) -> Self {
        Self::start(
            workspace,
            SessionSource::New,
            wake,
            None,
            None,
            true,
            Arc::default(),
        )
    }

    fn start(
        workspace: PathBuf,
        source: SessionSource,
        wake: Waker,
        instructions: Option<String>,
        host: Option<HostLink>,
        ephemeral: bool,
        owner: Arc<SessionOwner>,
    ) -> Self {
        let (session, fork_source) = match source {
            SessionSource::New => (None, None),
            SessionSource::Resume(path) => (Some(path), None),
            SessionSource::Fork(path) => (None, Some(path)),
        };
        let (commands, input) = mpsc::sync_channel::<Value>(32);
        let (events, output) = mpsc::sync_channel(256);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let session_lock = session.as_ref().map(|session| {
            let path = Path::new(session);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace.join(path)
            };
            let path = crate::projects::normalize_path(&path);
            let mut sessions = SESSIONS.lock().expect("session owners");
            sessions.retain(|_, owner| owner.strong_count() > 0);
            let owner = sessions
                .get(&path)
                .and_then(std::sync::Weak::upgrade)
                .unwrap_or_else(|| Arc::new(Mutex::new(())));
            sessions.insert(path, Arc::downgrade(&owner));
            owner
        });
        #[cfg(test)]
        let session_directory = crate::store::directory()
            .expect("test profile")
            .join("pi-sessions");
        let worker = thread::spawn(move || {
            let _connection_guard = match acquire_session(&owner.lock, &stop) {
                Ok(Some(guard)) => guard,
                Ok(None) => return,
                Err(error) => {
                    publish(&events, &wake, Event::Error(error));
                    return;
                }
            };
            if owner.quarantined.load(Ordering::Acquire) {
                publish(&events, &wake, Event::Error("The previous Pi process could not be reaped. Restart Shina before resuming this session.".into()));
                return;
            }
            let _session_guard = if let Some(owner) = &session_lock {
                match acquire_session(owner, &stop) {
                    Ok(Some(guard)) => Some(guard),
                    Ok(None) => return,
                    Err(error) => {
                        publish(&events, &wake, Event::Error(error));
                        return;
                    }
                }
            } else {
                None
            };
            let launch = Launch {
                workspace,
                session,
                fork_source,
                instructions,
                host,
                ephemeral,
                #[cfg(test)]
                session_directory,
            };
            match run(&launch, input, &events, &wake, &stop, &owner) {
                Err(error) => {
                    publish(&events, &wake, Event::Error(error));
                }
                Ok(()) => {
                    publish(&events, &wake, Event::Exited);
                }
            }
        });
        let mut workers = WORKERS.lock().expect("worker registry");
        workers.retain(|worker| !worker.is_finished());
        workers.push(worker);
        Self {
            commands,
            events: output,
            stopped,
        }
    }

    pub fn send(&self, record: Value) -> Result<(), String> {
        crate::outbox::send(&self.commands, record)
    }
}

pub(crate) fn publish(output: &SyncSender<Event>, wake: &Waker, event: Event) -> bool {
    // A full queue applies backpressure instead of growing memory without a limit.
    let sent = output.send(event).is_ok();
    wake();
    sent
}

/// Resolve npm's JavaScript entry directly on Windows, avoiding cmd.exe quoting and console windows.
pub(crate) fn discovery_command() -> Result<Command, String> {
    command()
}

fn command() -> Result<Command, String> {
    if let Some(path) = env::var_os("SHINA_PI_CLI").or_else(|| env::var_os("STILL_PI_CLI")) {
        return command_for(&PathBuf::from(path));
    }
    #[cfg(windows)]
    {
        for directory in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
            let executable = directory.join("pi.exe");
            if executable.is_file() {
                return Ok(Command::new(executable));
            }
            for package in [
                "@earendil-works/pi-coding-agent",
                "@mariozechner/pi-coding-agent",
            ] {
                let root = directory.join("node_modules").join(package);
                for entry in ["dist/bundle/cli.js", "dist/cli.js"] {
                    let path = root.join(entry);
                    if path.is_file() {
                        return command_for(&path);
                    }
                }
            }
        }
        Err("Pi was not found. Install Pi, or set SHINA_PI_CLI to its CLI .js or .exe path, then retry.".into())
    }
    #[cfg(not(windows))]
    Ok(Command::new("pi"))
}

fn command_for(path: &Path) -> Result<Command, String> {
    if !path.is_file() {
        return Err(format!("The Pi CLI does not exist at {}.", path.display()));
    }
    if path
        .extension()
        .is_some_and(|ext| ext == "js" || ext == "mjs")
    {
        let mut command = Command::new("node");
        command.arg(path);
        Ok(command)
    } else if path
        .extension()
        .is_some_and(|ext| ext == "cmd" || ext == "ps1")
    {
        Err("SHINA_PI_CLI must name Pi's CLI .js or native executable, not a shell wrapper.".into())
    } else {
        Ok(Command::new(path))
    }
}

fn cpa_extension() -> Result<Option<PathBuf>, String> {
    let agent_directory = env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".pi/agent"))
        })
        .ok_or("Could not find Pi's agent directory. Set PI_CODING_AGENT_DIR.")?;
    cpa_extension_in(&agent_directory)
}

fn cpa_extension_in(agent_directory: &Path) -> Result<Option<PathBuf>, String> {
    let extension = agent_directory.join("extensions/cpa-provider.ts");
    if extension.is_file() {
        return Ok(Some(extension));
    }
    // Some enrolled hosts use Pi's generated models.json instead of an extension.
    // Read only enrollment metadata. Pi still owns the routes and credentials, and
    // Shina's catalog and prompt guards continue to reject every non-CPA provider.
    let registry = std::fs::read(agent_directory.join("models.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    if registry.as_ref().is_some_and(|registry| {
        let provider = &registry["providers"]["cpa"];
        provider["baseUrl"]
            .as_str()
            .is_some_and(|url| !url.trim().is_empty())
            && provider["models"].as_array().is_some_and(|models| {
                models
                    .iter()
                    .any(|model| model["id"].as_str().is_some_and(|id| !id.trim().is_empty()))
            })
    }) {
        return Ok(None);
    }
    Err(format!(
        "Pi has no CPA provider extension at {} and no configured CPA models in models.json. Enroll Pi in cpa-catalog, then retry. Shina will not fall back to a direct provider.",
        extension.display()
    ))
}

fn resource_args(ephemeral: bool) -> &'static [&'static str] {
    if ephemeral {
        &[
            "--mode",
            "rpc",
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-prompt-templates",
        ]
    } else {
        &["--mode", "rpc"]
    }
}

fn run(
    launch: &Launch,
    input: Receiver<Value>,
    output: &SyncSender<Event>,
    wake: &Waker,
    stopped: &AtomicBool,
    owner: &SessionOwner,
) -> Result<(), String> {
    let mut command = command()?;
    // Catalog probes stay offline and isolated. Real sessions use Pi's normal resource discovery
    // and trust flow, like KSG Code's buildPiRpcLaunch, without rewriting the user's settings.
    command.args(resource_args(launch.ephemeral));
    command
        .current_dir(&launch.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The explicit CPA provider also works for restricted catalog probes. Pi deduplicates
    // a path that normal discovery has already loaded in a live session.
    if let Some(extension) = cpa_extension()? {
        command.arg("--extension").arg(extension);
    }
    #[cfg(test)]
    {
        std::fs::create_dir_all(&launch.session_directory).map_err(|e| e.to_string())?;
        command.arg("--session-dir").arg(&launch.session_directory);
    }
    // Shina's credentials belong to this thread only; never pass along inherited ones.
    command
        .env_remove(crate::mcp::URL_ENV)
        .env_remove(crate::mcp::TOKEN_ENV)
        .env_remove("SHINA_RUNTIME_MODE")
        .env_remove("SHINA_PI_BRIDGE_PATH");
    if let Some(host) = &launch.host {
        command
            .arg("--extension")
            .arg(&host.extension)
            .env(crate::mcp::URL_ENV, &host.url)
            .env(crate::mcp::TOKEN_ENV, &host.token)
            .env("SHINA_RUNTIME_MODE", &host.access_mode)
            .env("SHINA_PI_BRIDGE_PATH", &host.extension);
    }
    if let Some(instructions) = &launch.instructions {
        command.arg("--append-system-prompt").arg(instructions);
    }
    if let Some(source) = &launch.fork_source {
        command.args(["--fork", source]);
    } else if let Some(session) = &launch.session {
        command.args(["--session", session]);
    } else if launch.ephemeral {
        command.args(["--no-session", "--offline"]);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = crate::process::Process::spawn(&mut command)
        .map_err(|e| format!("Could not start Pi: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let events = output.clone();
    let waker = wake.clone();
    let ready = Arc::new(AtomicBool::new(false));
    let received_state = ready.clone();
    #[cfg(test)]
    let import_directory = launch.session_directory.parent().unwrap().to_path_buf();
    let stdout_worker = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            match read_record(&mut reader) {
                Ok(Some(mut record)) => {
                    if record["type"] == "response" && record["command"] == "get_state"
                        || record["type"] == "extension_ui_request"
                            && matches!(
                                record["method"].as_str(),
                                Some("confirm" | "select" | "input" | "editor")
                            )
                    {
                        // A trust or extension dialog is a live Pi, not a failed startup.
                        received_state.store(true, Ordering::Relaxed);
                    }
                    #[cfg(test)]
                    crate::store::with_directory(import_directory.clone(), || {
                        crate::attachments::import_history(&mut record);
                    });
                    #[cfg(not(test))]
                    crate::attachments::import_history(&mut record);
                    crate::model::session_history(&mut record);
                    if !publish(&events, &waker, Event::Record(record)) {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    publish(&events, &waker, Event::Error(error));
                    break;
                }
            }
        }
    });
    let events = output.clone();
    let waker = wake.clone();
    let stderr_worker = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        // Keep draining diagnostics, but send a bounded amount to the UI.
        let mut sent = 0;
        loop {
            let mut bytes = Vec::new();
            let result = Read::by_ref(&mut reader)
                .take(8192)
                .read_until(b'\n', &mut bytes);
            match result {
                Ok(0) | Err(_) => break,
                _ if sent < 12 => {
                    sent += 1;
                    if !publish(
                        &events,
                        &waker,
                        Event::Diagnostic(String::from_utf8_lossy(&bytes).trim().to_owned()),
                    ) {
                        break;
                    }
                }
                _ => {}
            }
        }
    });
    let (writes, records) = mpsc::sync_channel(32);
    let (write_result, written) = mpsc::channel();
    let writing = Arc::new(Mutex::new(None::<Instant>));
    let writer_state = writing.clone();
    let writer = thread::spawn(move || {
        let result = (|| {
            for record in [
                json!({"id":"state", "type":"get_state"}),
                json!({"id":"history", "type":"get_entries"}),
                json!({"id":"models", "type":"get_available_models"}),
                json!({"id":"levels", "type":"get_available_thinking_levels"}),
                json!({"id":"commands", "type":"get_commands"}),
            ]
            .into_iter()
            .chain(records)
            {
                *writer_state.lock().expect("write deadline") = Some(Instant::now());
                write_record(&mut stdin, &record)?;
                *writer_state.lock().expect("write deadline") = None;
            }
            Ok::<_, String>(())
        })();
        let _ = write_result.send(result);
    });
    let result = (|| {
        let started = Instant::now();
        loop {
            if stopped.load(Ordering::Acquire) {
                break;
            }
            if let Ok(result) = written.try_recv() {
                return result;
            }
            if writing
                .lock()
                .expect("write deadline")
                .is_some_and(|at| at.elapsed() > Duration::from_secs(30))
            {
                return Err("Pi stopped reading commands. Retry to resume the session.".into());
            }
            if !ready.load(Ordering::Relaxed) && started.elapsed() > Duration::from_secs(15) {
                return Err("Pi did not answer within 15 seconds. Check its CLI configuration and workspace trust, then retry.".into());
            }
            match input.recv_timeout(Duration::from_millis(100)) {
                Ok(record) => writes
                    .try_send(record)
                    .map_err(|_| "Pi stopped accepting commands.".to_owned())?,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "Pi exited with {status}. Retry to resume the saved conversation."
                    ))
                };
            }
        }
        Ok(())
    })();
    drop(writes);
    child.terminate();
    if let Err(error) = child.reap(Duration::from_secs(1)) {
        owner.quarantined.store(true, Ordering::Release);
        return Err(format!(
            "Could not reap Pi. Reconnect is disabled until restart: {error}"
        ));
    }
    // Deliver all buffered records before Event::Exited makes the UI drop its receiver.
    crate::process::join_for(writer, Duration::from_millis(200));
    crate::process::join_for(stdout_worker, Duration::from_millis(200));
    crate::process::join_for(stderr_worker, Duration::from_millis(200));
    result
}

pub(crate) fn write_record(writer: &mut impl Write, record: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, record).map_err(|e| e.to_string())?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|e| format!("Could not write to Pi: {e}"))
}

pub(crate) fn read_record(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    // KSG provider-pi/server/rpc.ts budgets duplicate MCP image content plus 8 MiB.
    // This also admits an echoed 80 MiB user image payload after base64 expansion.
    const LIMIT: u64 =
        2 * 8 * crate::attachments::MAX_IMAGE_BYTES.div_ceil(3) * 4 + 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    let count = Read::by_ref(reader)
        .take(LIMIT + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| format!("Could not read Pi output: {e}"))?;
    if count == 0 {
        return Ok(None);
    }
    if count as u64 > LIMIT {
        return Err(
            "The provider sent a protocol record larger than the attachment transport limit. Inspect the saved session or start a new Shina thread."
                .into(),
        );
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| format!("Pi sent an invalid RPC record: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_discovery_is_enabled_only_in_real_sessions() {
        assert_eq!(resource_args(false), &["--mode", "rpc"]);
        for flag in [
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-prompt-templates",
        ] {
            assert!(resource_args(true).contains(&flag));
            assert!(!resource_args(false).contains(&flag));
        }
    }

    #[test]
    fn waiting_for_the_previous_session_is_cancellable_without_starting_pi() {
        let owner = Arc::new(Mutex::new(()));
        let guard = owner.lock().unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let (started, ready) = mpsc::channel();
        let (done, result) = mpsc::channel();
        let worker_owner = owner.clone();
        let stop = stopped.clone();
        let worker = thread::spawn(move || {
            started.send(()).unwrap();
            done.send(acquire_session(&worker_owner, &stop).unwrap().is_none())
                .unwrap();
        });
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(result.try_recv().is_err());
        stopped.store(true, Ordering::Release);
        assert!(result.recv_timeout(Duration::from_secs(1)).unwrap());
        drop(guard);
        worker.join().unwrap();
    }
    #[test]
    fn cpa_enrollment_accepts_generated_models_without_loading_other_extensions() {
        let directory = crate::store::directory().unwrap();
        for registry in [
            json!({}),
            json!({"providers":{"direct":{"baseUrl":"https://example.invalid","models":[{"id":"x"}]}}}),
            json!({"providers":{"cpa":{"baseUrl":"http://127.0.0.1:8317/v1","models":[]}}}),
        ] {
            std::fs::write(directory.join("models.json"), registry.to_string()).unwrap();
            assert!(cpa_extension_in(&directory).is_err());
        }
        std::fs::write(
            directory.join("models.json"),
            json!({
                "providers":{"cpa":{"baseUrl":"http://127.0.0.1:8317/v1","models":[{"id":"proxy"}]}}
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(cpa_extension_in(&directory).unwrap(), None);
        let extensions = directory.join("extensions");
        std::fs::create_dir(&extensions).unwrap();
        let provider = extensions.join("cpa-provider.ts");
        std::fs::write(&provider, "// Existing owner-managed provider").unwrap();
        assert_eq!(cpa_extension_in(&directory).unwrap(), Some(provider));
    }

    #[test]
    fn framing_preserves_unicode_and_accepts_crlf() {
        let input = "{\"type\":\"test\",\"text\":\"a\u{2028}b\u{2029}c\"}\r\n{\"type\":\"next\"}\n";
        let mut reader = std::io::Cursor::new(input.as_bytes());
        assert_eq!(
            read_record(&mut reader).unwrap().unwrap()["text"],
            "a\u{2028}b\u{2029}c"
        );
        assert_eq!(read_record(&mut reader).unwrap().unwrap()["type"], "next");
        assert!(read_record(&mut reader).unwrap().is_none());
    }

    #[test]
    fn command_encoding_is_one_jsonl_record() {
        let mut output = Vec::new();
        write_record(
            &mut output,
            &json!({"type":"prompt","message":"quoted \"text\"\nand newline"}),
        )
        .unwrap();
        assert_eq!(output.iter().filter(|&&b| b == b'\n').count(), 1);
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap()["message"],
            "quoted \"text\"\nand newline"
        );
    }

    #[test]
    fn image_history_records_can_exceed_the_old_eight_mib_limit() {
        let data = "A".repeat(9 * 1024 * 1024);
        let record = json!({"type":"response","command":"get_messages","data":{"messages":[
            {"role":"user","content":[{"type":"image","data":data,"mimeType":"image/png"}]}
        ]}});
        let encoded = serde_json::to_vec(&record).unwrap();
        let mut input = std::io::Cursor::new(encoded);
        let decoded = read_record(&mut input).unwrap().unwrap();
        assert_eq!(
            decoded["data"]["messages"][0]["content"][0]["data"]
                .as_str()
                .unwrap()
                .len(),
            9 * 1024 * 1024
        );
    }

    #[test]
    #[ignore = "Starts the installed Pi, but sends no model prompt"]
    fn installed_pi_answers_state_and_history_without_a_prompt() {
        crate::shell_env::reconcile_path();
        for ephemeral in [false, true] {
            let pi = if ephemeral {
                Pi::discover(std::env::temp_dir(), Arc::new(|| {}))
            } else {
                Pi::start_with_instructions(std::env::temp_dir(), None, Arc::new(|| {}), None, None)
            };
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut state = false;
            let mut history = false;
            let mut models = false;
            let mut levels = false;
            while Instant::now() < deadline && !(state && history && models && levels) {
                match pi.events.recv_timeout(Duration::from_secs(1)) {
                    Ok(Event::Record(record)) if record["type"] == "response" => {
                        assert_ne!(
                            record["success"], false,
                            "Pi rejected the read-only handshake"
                        );
                        if record["command"] == "get_state" {
                            if ephemeral {
                                assert!(
                                    record["data"]["sessionFile"].is_null(),
                                    "discovery must not save a session"
                                );
                            }
                            assert_eq!(
                                record["data"]["model"]["provider"], "cpa",
                                "Pi must load the CPA provider instead of falling back to a direct provider"
                            );
                            state = true;
                        }
                        if record["command"] == "get_messages" {
                            history = true;
                        }
                        if record["command"] == "get_available_models" {
                            assert!(
                                record["data"]["models"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .any(|model| model["provider"] == "cpa"),
                                "CPA catalog must populate model discovery"
                            );
                            models = true;
                        }
                        if record["command"] == "get_available_thinking_levels" {
                            assert!(!record["data"]["levels"].as_array().unwrap().is_empty());
                            levels = true;
                        }
                    }
                    Ok(Event::Error(error)) => panic!("Pi handshake failed: {error}"),
                    Ok(Event::Exited) => panic!("Pi exited before answering the handshake"),
                    _ => {}
                }
            }
            drop(pi);
            shutdown_all();
            assert!(
                state && history && models && levels,
                "Pi did not return state, history, models and reasoning levels"
            );
        }
    }
}
