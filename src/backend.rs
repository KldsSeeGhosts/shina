//! Pi is an RPC subprocess, never a terminal. All pipes and process waits live off the UI thread.
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
    for worker in workers {
        let _ = worker.join();
    }
}

/// How a Pi process reaches the Shina MCP: the endpoint, this thread's bearer token, and the
/// bridge extension that registers Shina's tools inside Pi.
#[derive(Clone, Debug)]
pub struct HostLink {
    pub url: String,
    pub token: String,
    pub extension: PathBuf,
}

pub struct Pi {
    commands: SyncSender<Value>,
    pub events: Receiver<Event>,
}

/// Machine dispatch lives here, not in the UI or orchestration reducer. A remote host
/// can implement the same session transport without changing project/thread ownership.
pub enum BackendHost {
    Local,
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
        session: Option<String>,
        wake: Waker,
        instructions: Option<String>,
        link: Option<HostLink>,
    ) -> Pi {
        match self {
            Self::Local => {
                Pi::start_with_instructions(directory, session, wake, instructions, link)
            }
        }
    }
}

struct Launch {
    workspace: PathBuf,
    session: Option<String>,
    instructions: Option<String>,
    host: Option<HostLink>,
    ephemeral: bool,
    #[cfg(test)]
    session_directory: PathBuf,
}

impl Pi {
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
        (Self { commands, events }, input, output)
    }

    pub fn start_with_instructions(
        workspace: PathBuf,
        session: Option<String>,
        wake: Waker,
        instructions: Option<String>,
        host: Option<HostLink>,
    ) -> Self {
        Self::start(workspace, session, wake, instructions, host, false)
    }

    /// Read provider metadata without creating a saved conversation or exposing Shina tools.
    pub fn discover(workspace: PathBuf, wake: Waker) -> Self {
        Self::start(workspace, None, wake, None, None, true)
    }

    fn start(
        workspace: PathBuf,
        session: Option<String>,
        wake: Waker,
        instructions: Option<String>,
        host: Option<HostLink>,
        ephemeral: bool,
    ) -> Self {
        let (commands, input) = mpsc::sync_channel::<Value>(32);
        let (events, output) = mpsc::sync_channel(256);
        #[cfg(test)]
        let session_directory = crate::store::directory()
            .expect("test profile")
            .join("pi-sessions");
        let worker = thread::spawn(move || {
            let launch = Launch {
                workspace,
                session,
                instructions,
                host,
                ephemeral,
                #[cfg(test)]
                session_directory,
            };
            if let Err(error) = run(&launch, input, &events, &wake) {
                publish(&events, &wake, Event::Error(error));
            }
            publish(&events, &wake, Event::Exited);
        });
        let mut workers = WORKERS.lock().expect("worker registry");
        workers.retain(|worker| !worker.is_finished());
        workers.push(worker);
        Self {
            commands,
            events: output,
        }
    }

    pub fn send(&self, record: Value) -> Result<(), String> {
        self.commands
            .try_send(record)
            .map_err(|e| format!("Pi did not accept the command: {e}"))
    }
}

fn publish(output: &SyncSender<Event>, wake: &Waker, event: Event) -> bool {
    // A full queue applies backpressure instead of growing memory without a limit.
    let sent = output.send(event).is_ok();
    wake();
    sent
}

/// Resolve npm's JavaScript entry directly on Windows, avoiding cmd.exe quoting and console windows.
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

fn run(
    launch: &Launch,
    input: Receiver<Value>,
    output: &SyncSender<Event>,
    wake: &Waker,
) -> Result<(), String> {
    let mut command = command()?;
    command
        .args([
            "--mode",
            "rpc",
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-prompt-templates",
        ])
        .current_dir(&launch.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Explicit extensions still load with --no-extensions. Load only the user's
    // catalog-backed provider, not unrelated packages, project extensions or MCP.
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
        .env_remove(crate::mcp::TOKEN_ENV);
    if let Some(host) = &launch.host {
        command
            .arg("--extension")
            .arg(&host.extension)
            .env(crate::mcp::URL_ENV, &host.url)
            .env(crate::mcp::TOKEN_ENV, &host.token);
    }
    if let Some(instructions) = &launch.instructions {
        command.arg("--append-system-prompt").arg(instructions);
    }
    if let Some(session) = &launch.session {
        command.args(["--session", session]);
    } else if launch.ephemeral {
        command.args(["--no-session", "--offline"]);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not start Pi: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let events = output.clone();
    let waker = wake.clone();
    let ready = Arc::new(AtomicBool::new(false));
    let received_state = ready.clone();
    let stdout_worker = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            match read_record(&mut reader) {
                Ok(Some(record)) => {
                    if record["type"] == "response" && record["command"] == "get_state" {
                        received_state.store(true, Ordering::Relaxed);
                    }
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
    let result = (|| {
        let started = Instant::now();
        write_record(&mut stdin, &json!({"id":"state", "type":"get_state"}))?;
        write_record(&mut stdin, &json!({"id":"history", "type":"get_messages"}))?;
        write_record(
            &mut stdin,
            &json!({"id":"models", "type":"get_available_models"}),
        )?;
        write_record(
            &mut stdin,
            &json!({"id":"levels", "type":"get_available_thinking_levels"}),
        )?;
        loop {
            if !ready.load(Ordering::Relaxed) && started.elapsed() > Duration::from_secs(15) {
                return Err("Pi did not answer within 15 seconds. Check its CLI configuration and workspace trust, then retry.".into());
            }
            match input.recv_timeout(Duration::from_millis(100)) {
                Ok(record) => write_record(&mut stdin, &record)?,
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
    // Dropping Pi disconnects the command channel. Close stdin, allow graceful shutdown, then reap.
    drop(stdin);
    let deadline = Instant::now() + Duration::from_millis(700);
    let mut exited = false;
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            exited = true;
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    if !exited {
        let _ = child.kill();
        let _ = child.wait();
    }
    // Deliver all buffered records before Event::Exited makes the UI drop its receiver.
    let _ = stdout_worker.join();
    let _ = stderr_worker.join();
    result
}

fn write_record(writer: &mut impl Write, record: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, record).map_err(|e| e.to_string())?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|e| format!("Could not write to Pi: {e}"))
}

fn read_record(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    // Pi echoes user images in message events and saved history. A single accepted
    // 20 MiB image prompt expands to almost 28 MiB in base64, before JSON framing.
    const LIMIT: u64 = 64 * 1024 * 1024;
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
            "Pi sent a protocol record larger than 64 MiB. Image-heavy history can exceed this limit. Inspect the saved session with Pi or start a new Shina thread."
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
