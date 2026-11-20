//! Devin (`devin acp`) normalization and client terminals, ported from KSG Code's
//! `provider-acp-registry/src/server/devinAcp.ts`, its `clientTerminals.ts`
//! (`shellCommands: true`), and the managed registry cache in `AcpRegistrySupport.ts`.
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

/// KSG `clientCapabilitiesMeta`: Devin tags subagent work and streamed message
/// grouping in `_meta` only after the client advertises support.
pub fn client_capabilities_meta() -> Value {
    json!({"cognition.ai/subagentSupport":true,"cognition.ai/messageGrouping":true})
}

/// KSG `normalizeDevinSessionUpdate`. Child-session updates are rerouted from the
/// root wire session to `cognition.ai/subagent_context.parentAgentId`, and streamed
/// chunks gain `cognition.ai/streamingMessageId` as `messageId`.
pub fn normalize_session_update(notification: &Value) -> Value {
    let update = &notification["update"];
    let meta = &update["_meta"];
    let mut routed = notification.clone();
    if let Some(parent) = meta["cognition.ai/subagent_context"]["parentAgentId"]
        .as_str()
        .filter(|parent| *parent != "root")
    {
        routed["sessionId"] = json!(parent);
    }
    if matches!(
        update["sessionUpdate"].as_str(),
        Some("agent_message_chunk" | "agent_thought_chunk")
    ) && let Some(message_id) = meta["cognition.ai/streamingMessageId"].as_str()
    {
        routed["update"]["messageId"] = json!(message_id);
    }
    routed
}

/// KSG `normalizeDevinToolCall`: Devin names its calls in `_meta`, sometimes after a
/// generic `Tool` title arrives first.
pub fn normalize_tool_call(tool: &Value) -> Value {
    let Some(name) = tool["_meta"]["cognition.ai/inferenceToolName"].as_str() else {
        return tool.clone();
    };
    if tool["title"]
        .as_str()
        .is_none_or(|title| title.is_empty() || title == "Tool")
    {
        let mut tool = tool.clone();
        tool["title"] = json!(name);
        tool
    } else {
        tool.clone()
    }
}

/// KSG `AcpAdapterV2SubagentUpdate` reduced to the fields Shina's transcript uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubagentUpdate {
    pub native_task_id: String,
    pub child_session_id: String,
    pub prompt: String,
    pub title: Option<String>,
    pub model: Option<String>,
    /// "running" | "completed" | "failed", like KSG's subagent status strings.
    pub status: &'static str,
    pub result: Option<String>,
}

/// KSG `extractDevinSubagentUpdate`. Completion metadata wins over started metadata.
pub fn extract_subagent_update(tool: &Value) -> Option<SubagentUpdate> {
    let meta = &tool["_meta"];
    let completed = meta["cognition.ai/subagent_completed"].as_object();
    let started = meta["cognition.ai/subagent_started"].as_object();
    let data = completed.or(started)?;
    let agent_id = data.get("agentId")?.as_str()?;
    Some(SubagentUpdate {
        native_task_id: agent_id.to_owned(),
        child_session_id: agent_id.to_owned(),
        prompt: started
            .and_then(|started| started.get("task")?.as_str())
            .unwrap_or("")
            .to_owned(),
        title: started
            .and_then(|started| started.get("title")?.as_str())
            .map(str::to_owned),
        model: started
            .and_then(|started| started.get("model")?.as_str())
            .map(str::to_owned),
        status: match completed {
            Some(completed) if completed.get("success") == Some(&Value::Bool(false)) => "failed",
            Some(_) => "completed",
            None => "running",
        },
        result: completed
            .and_then(|completed| completed.get("summary")?.as_str())
            .map(str::to_owned),
    })
}

/// KSG `devinAnswerablePermissionOptions`: Devin's plan exit offers several
/// `allow_once` options in no promised order; `plan_bypass` is never an answer.
pub fn answerable_permission_options(request: &Value) -> Vec<Value> {
    request["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|option| option["optionId"].as_str().map(str::trim) != Some("plan_bypass"))
        .cloned()
        .collect()
}

/// The built-in registry entry (`KSG DevinEnv` passthrough names; values stay in the
/// environment only, like every other Agent secret).
pub fn agent(command: PathBuf) -> crate::harness::Agent {
    crate::harness::Agent {
        id: "devin".into(),
        name: "Devin".into(),
        command: command.to_string_lossy().into_owned(),
        args: vec!["acp".into()],
        env: vec!["DEVIN_TOKEN".into(), "DEVIN_API_KEY".into()],
    }
}

/// KSG `readCacheCommand`: prefer `devin` on PATH, then the newest T3-managed
/// install under `tools/devin/<version>/<platform-target>`.
#[cfg(not(test))]
pub fn discover_command() -> Option<PathBuf> {
    if crate::harness::executable("devin").is_ok() {
        return Some(PathBuf::from("devin"));
    }
    cached_command()
}

/// Hermetic like `antigravity::cli_available`: the host's Devin install and the T3
/// tools cache must not change test snapshots. `cached_command_in` is covered
/// directly with fixture trees.
#[cfg(test)]
pub fn discover_command() -> Option<PathBuf> {
    None
}

#[cfg(not(test))]
fn cached_command() -> Option<PathBuf> {
    let home = std::env::var_os("T3CODE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|home| PathBuf::from(home).join(".t3"))
        })?;
    cached_command_in(&home.join("tools"))
}

/// Scan `tools/devin/<version>/<platform-target>` newest-first, like KSG's
/// `installRoot` walk. The registry ships the binary under `bin/`.
fn cached_command_in(tools_root: &Path) -> Option<PathBuf> {
    let root = tools_root.join("devin");
    let mut versions: Vec<String> = std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();
    versions.sort_by(|a, b| natural_order(b, a));
    let executable = if cfg!(windows) { "devin.exe" } else { "devin" };
    for version in versions {
        for target in platform_targets() {
            let install = root.join(&version).join(&target);
            if !install.is_dir() {
                continue;
            }
            for candidate in [
                install.join("bin").join(executable),
                install.join(executable),
            ] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn platform_targets() -> Vec<String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "windows",
        "linux" => "linux",
        _ => return vec![],
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        _ => return vec![],
    };
    let target = format!("{os}-{arch}");
    // KSG candidateTargets: Windows ARM also accepts the x64 emulation install.
    if target == "windows-aarch64" {
        return vec![target, "windows-x86_64".into()];
    }
    vec![target]
}

/// localeCompare `{numeric:true}` for registry version directory names.
fn natural_order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    loop {
        match (a.get(i), b.get(j)) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let digits = |s: &[u8]| s.iter().take_while(|c| c.is_ascii_digit()).count();
                let (an, bn) = (digits(&a[i..]), digits(&b[j..]));
                let strip = |s: &[u8]| {
                    s.iter()
                        .skip_while(|&&c| c == b'0')
                        .copied()
                        .collect::<Vec<u8>>()
                };
                let (aa, bb) = (strip(&a[i..i + an]), strip(&b[j..j + bn]));
                let order = aa.len().cmp(&bb.len()).then_with(|| aa.cmp(&bb));
                if order != Ordering::Equal {
                    return order;
                }
                i += an;
                j += bn;
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(y);
                }
                i += 1;
                j += 1;
            }
        }
    }
}

// KSG clientTerminals.ts limits.
const MAX_LIVE_TERMINALS: usize = 16;
const MAX_UNRELEASED_TERMINALS: usize = 64;
const MAX_RETAINED_TERMINALS: usize = 32;
const MAX_RETAINED_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_OUTPUT_BYTE_LIMIT: usize = 4 * 1024 * 1024;
const MAX_OUTPUT_BYTE_LIMIT: usize = 8 * 1024 * 1024;
/// A backgrounded descendant can inherit the output pipes; report exit without them.
const OUTPUT_DRAIN_GRACE: Duration = Duration::from_secs(1);

/// ACP `terminal/*` sessions for Devin. Terminals bound to released or other
/// sessions answer "unknown terminal ID", exactly like KSG's `requireTerminal`.
#[derive(Clone, Default)]
pub struct Terminals {
    shared: Arc<Mutex<TerminalShared>>,
}

#[derive(Default)]
struct TerminalShared {
    terminals: HashMap<String, Terminal>,
    /// Insertion order is KSG's Map order: the oldest terminal trims first.
    order: Vec<String>,
    serial: usize,
}

struct Terminal {
    session: String,
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    truncated: bool,
    limit: usize,
    exit: Option<Value>,
    released: bool,
    process: Arc<Mutex<crate::process::Process>>,
}

impl Terminal {
    fn append(&mut self, chunk: &[u8]) {
        self.chunks.push_back(chunk.to_vec());
        self.bytes += chunk.len();
        self.truncate(self.bytes.saturating_sub(self.limit));
    }

    /// Trim from the front like KSG's tail buffer, then report `truncated`.
    fn truncate(&mut self, mut excess: usize) -> usize {
        let mut removed = 0;
        while excess > 0 {
            let Some(front) = self.chunks.front_mut() else {
                break;
            };
            let take = front.len().min(excess);
            front.drain(..take);
            if front.is_empty() {
                self.chunks.pop_front();
            }
            removed += take;
            excess -= take;
        }
        self.bytes -= removed;
        self.truncated |= removed > 0;
        removed
    }

    /// `flattenTerminalChunks`: whole retained buffer; a byte split mid-character
    /// decodes as a replacement character, as KSG's Buffer.toString does.
    fn output(&self) -> (String, bool) {
        let size = self.chunks.iter().map(Vec::len).sum();
        let mut bytes = Vec::with_capacity(size);
        for chunk in &self.chunks {
            bytes.extend_from_slice(chunk);
        }
        (String::from_utf8_lossy(&bytes).into_owned(), self.truncated)
    }
}

impl TerminalShared {
    /// KSG `enforceTotalOutputLimit`: the oldest terminal loses output first.
    fn enforce_total_limit(&mut self) {
        let mut excess = self
            .terminals
            .values()
            .map(|terminal| terminal.bytes)
            .sum::<usize>()
            .saturating_sub(MAX_RETAINED_OUTPUT_BYTES);
        for id in &self.order {
            if excess == 0 {
                return;
            }
            if let Some(terminal) = self.terminals.get_mut(id) {
                excess = excess.saturating_sub(terminal.truncate(excess));
            }
        }
    }

    /// KSG `pruneReleasedTerminals`: released terminals are dropped oldest-first when
    /// either retained bound is exceeded.
    fn prune_released(&mut self) {
        let mut released: Vec<String> = self
            .order
            .iter()
            .filter(|id| self.terminals.get(*id).is_some_and(|t| t.released))
            .cloned()
            .collect();
        let mut retained: usize = released
            .iter()
            .filter_map(|id| self.terminals.get(id))
            .map(|terminal| terminal.bytes)
            .sum();
        while released.len() > MAX_RETAINED_TERMINALS || retained > MAX_RETAINED_OUTPUT_BYTES {
            let id = released.remove(0);
            if let Some(terminal) = self.terminals.remove(&id) {
                retained = retained.saturating_sub(terminal.bytes);
            }
            self.order.retain(|kept| kept != &id);
        }
    }
}

/// KSG `acpTerminalCommand` under `shellCommands: true`: Devin sends shell source in
/// `command` rather than an executable plus args.
fn terminal_command(command: &str, args: &[String]) -> Command {
    if args.is_empty() {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let mut shell =
                Command::new(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()));
            shell
                .args(["/d", "/s", "/c"])
                .raw_arg(format!("\"{command}\""));
            shell
        }
        #[cfg(not(windows))]
        {
            let mut shell = Command::new("sh");
            shell.args(["-c", command]);
            shell
        }
    } else {
        let mut direct = Command::new(command);
        direct.args(args);
        direct
    }
}

/// KSG `mergeEnvironments`: allowlisted base environment, then the request's `env` entries;
/// on Windows a request `Path` replaces the inherited `PATH`.
fn terminal_environment(request_env: &[Value]) -> Vec<(OsString, OsString)> {
    const ALLOWED_BASE_ENV: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "LANG",
        "LC_ALL",
        "TERM",
        "DEVIN_TOKEN",
        "DEVIN_API_KEY",
    ];
    let inherited = ALLOWED_BASE_ENV
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(*name), value)));
    let requested = request_env.iter().filter_map(|entry| {
        Some((
            OsString::from(entry["name"].as_str()?),
            OsString::from(entry["value"].as_str()?),
        ))
    });
    let mut merged: Vec<(OsString, OsString)> = vec![];
    for (name, value) in inherited.chain(requested) {
        let position = merged.iter().position(|(existing, _)| {
            if cfg!(windows) {
                existing
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&name.to_string_lossy())
            } else {
                *existing == name
            }
        });
        match position {
            Some(index) => merged[index] = (name, value),
            None => merged.push((name, value)),
        }
    }
    merged
}

fn pump(
    shared: &Arc<Mutex<TerminalShared>>,
    id: &str,
    mut pipe: impl Read + Send + 'static,
) -> thread::JoinHandle<()> {
    let shared = shared.clone();
    let id = id.to_owned();
    thread::spawn(move || {
        let mut buffer = [0u8; 32 * 1024];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut shared = shared.lock().expect("ACP terminals");
                    if let Some(terminal) = shared.terminals.get_mut(&id) {
                        terminal.append(&buffer[..n]);
                        shared.enforce_total_limit();
                    }
                }
            }
        }
    })
}

fn exit_status(status: Option<std::process::ExitStatus>) -> Value {
    let code = status
        .and_then(|status| status.code())
        .map(|code| json!(code));
    #[cfg(unix)]
    let signal = status.and_then(|status| {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(signal_name)
    });
    #[cfg(not(unix))]
    let signal = None::<String>;
    json!({"exitCode":code,"signal":signal})
}

#[cfg(unix)]
fn signal_name(signal: i32) -> String {
    let name = match signal {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        6 => "SIGABRT",
        9 => "SIGKILL",
        11 => "SIGSEGV",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        19 => "SIGSTOP",
        20 => "SIGTSTP",
        24 => "SIGXCPU",
        25 => "SIGXFSZ",
        _ if cfg!(target_os = "macos") => match signal {
            10 => "SIGBUS",
            12 => "SIGSYS",
            30 => "SIGUSR1",
            31 => "SIGUSR2",
            _ => "",
        },
        _ => match signal {
            10 => "SIGUSR1",
            12 => "SIGUSR2",
            17 => "SIGCHLD",
            _ => "",
        },
    };
    if name.is_empty() {
        format!("signal {signal}")
    } else {
        name.into()
    }
}

impl Terminals {
    fn require<'a>(
        shared: &'a TerminalShared,
        params: &Value,
        operation: &str,
    ) -> Result<&'a Terminal, String> {
        let id = params["terminalId"]
            .as_str()
            .ok_or_else(|| format!("ACP {operation} requires a terminalId."))?;
        match shared.terminals.get(id) {
            Some(terminal)
                if !terminal.released
                    && Some(terminal.session.as_str()) == params["sessionId"].as_str() =>
            {
                Ok(terminal)
            }
            _ => Err(format!(
                "ACP {operation} received an unknown terminal ID '{id}'."
            )),
        }
    }

    /// `terminal/create`. Returns the new `terminalId`; the session only receives
    /// the id it later scopes every other call to.
    pub fn create(&self, params: &Value, default_cwd: &Path) -> Result<String, String> {
        let session = params["sessionId"]
            .as_str()
            .ok_or("ACP terminal/create requires a sessionId.")?;
        let command_line = params["command"]
            .as_str()
            .ok_or("ACP terminal/create requires a command.")?;
        let args: Vec<String> = params["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|arg| arg.as_str().map(str::to_owned))
            .collect();
        let limit = params["outputByteLimit"]
            .as_u64()
            .map(|limit| (limit as usize).min(MAX_OUTPUT_BYTE_LIMIT))
            .unwrap_or(DEFAULT_OUTPUT_BYTE_LIMIT);
        let mut shared = self.shared.lock().expect("ACP terminals");
        if shared
            .terminals
            .values()
            .filter(|terminal| !terminal.released)
            .count()
            >= MAX_UNRELEASED_TERMINALS
        {
            return Err(format!(
                "ACP terminal/create exceeded the limit of {MAX_UNRELEASED_TERMINALS} unreleased terminals."
            ));
        }
        if shared
            .terminals
            .values()
            .filter(|terminal| !terminal.released && terminal.exit.is_none())
            .count()
            >= MAX_LIVE_TERMINALS
        {
            return Err(format!(
                "ACP terminal/create exceeded the limit of {MAX_LIVE_TERMINALS} concurrent terminals."
            ));
        }
        let cwd = params["cwd"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| default_cwd.to_path_buf());
        let mut command = terminal_command(command_line, &args);
        command
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.env_clear();
        for (name, value) in
            terminal_environment(params["env"].as_array().map(Vec::as_slice).unwrap_or(&[]))
        {
            command.env(name, value);
        }
        let mut process = crate::process::Process::spawn(&mut command).map_err(|error| {
            format!("Could not start terminal command '{command_line}'. {error}")
        })?;
        let stdout = process.stdout.take().expect("piped stdout");
        let stderr = process.stderr.take().expect("piped stderr");
        shared.serial += 1;
        let id = format!("shina-term-{}", shared.serial);
        shared.order.push(id.clone());
        shared.terminals.insert(
            id.clone(),
            Terminal {
                session: session.to_owned(),
                chunks: VecDeque::new(),
                bytes: 0,
                truncated: false,
                limit,
                exit: None,
                released: false,
                process: Arc::new(Mutex::new(process)),
            },
        );
        let process = shared.terminals[&id].process.clone();
        drop(shared);
        let out = pump(&self.shared, &id, stdout);
        let err = pump(&self.shared, &id, stderr);
        let shared = self.shared.clone();
        let terminal_id = id.clone();
        thread::spawn(move || {
            let status = loop {
                match process.lock().expect("ACP terminal").try_wait() {
                    Ok(Some(status)) => break Some(status),
                    Ok(None) => thread::sleep(Duration::from_millis(10)),
                    Err(_) => break None,
                }
            };
            let deadline = Instant::now() + OUTPUT_DRAIN_GRACE;
            crate::process::join_for(out, deadline.saturating_duration_since(Instant::now()));
            crate::process::join_for(err, deadline.saturating_duration_since(Instant::now()));
            if let Some(terminal) = shared
                .lock()
                .expect("ACP terminals")
                .terminals
                .get_mut(&terminal_id)
            {
                terminal.exit = Some(exit_status(status));
            }
        });
        Ok(id)
    }

    /// `terminal/output` → `{output, truncated, exitStatus?}`.
    pub fn output(&self, params: &Value) -> Result<Value, String> {
        let shared = self.shared.lock().expect("ACP terminals");
        let terminal = Self::require(&shared, params, "terminal/output")?;
        let (output, truncated) = terminal.output();
        let mut result = json!({"output":output,"truncated":truncated});
        if let Some(exit) = &terminal.exit {
            result["exitStatus"] = exit.clone();
        }
        Ok(result)
    }

    /// `terminal/wait_for_exit`: `Some(status)` once exited; `None` while running.
    /// The caller registers a waiter for the `None` case.
    pub fn exit_status(&self, params: &Value) -> Result<Option<Value>, String> {
        let shared = self.shared.lock().expect("ACP terminals");
        Ok(Self::require(&shared, params, "terminal/wait_for_exit")?
            .exit
            .clone())
    }

    /// Waiter flushing only needs the id; released terminals never resolve.
    pub fn exit_status_of(&self, terminal: &str) -> Option<Value> {
        self.shared
            .lock()
            .expect("ACP terminals")
            .terminals
            .get(terminal)
            .and_then(|terminal| terminal.exit.clone())
    }

    /// `terminal/kill`: KSG answers as soon as the signal is sent, not after exit.
    pub fn kill(&self, params: &Value) -> Result<(), String> {
        let process = {
            let shared = self.shared.lock().expect("ACP terminals");
            Self::require(&shared, params, "terminal/kill")?
                .process
                .clone()
        };
        process.lock().expect("ACP terminal").terminate();
        Ok(())
    }

    /// `terminal/release`: mark released, stop the child, then prune the retained
    /// set. KSG waits for exit before answering.
    pub fn release(&self, params: &Value) -> Result<(), String> {
        let process = {
            let mut shared = self.shared.lock().expect("ACP terminals");
            let id = params["terminalId"]
                .as_str()
                .ok_or("ACP terminal/release requires a terminalId.")?;
            let valid = shared.terminals.get(id).is_some_and(|terminal| {
                !terminal.released
                    && Some(terminal.session.as_str()) == params["sessionId"].as_str()
            });
            if !valid {
                return Err(format!(
                    "ACP terminal/release received an unknown terminal ID '{id}'."
                ));
            }
            let terminal = shared.terminals.get_mut(id).expect("checked terminal");
            terminal.released = true;
            terminal.process.clone()
        };
        process.lock().expect("ACP terminal").terminate();
        let id = params["terminalId"].as_str().unwrap_or_default().to_owned();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let done = self
                .shared
                .lock()
                .expect("ACP terminals")
                .terminals
                .get(&id)
                .is_none_or(|terminal| terminal.exit.is_some());
            if done {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.shared.lock().expect("ACP terminals").prune_released();
        Ok(())
    }

    /// KSG `readOutputSnapshot`: embedded tool-call terminal content reads stay
    /// available after release but must match the update's session.
    pub fn output_snapshot(&self, session: &str, terminal: &str) -> Option<String> {
        let shared = self.shared.lock().expect("ACP terminals");
        let terminal = shared.terminals.get(terminal)?;
        (terminal.session == session).then(|| terminal.output().0)
    }

    /// Session teardown stops every child; watcher threads then exit on their own.
    pub fn dispose(&self) {
        let processes: Vec<Arc<Mutex<crate::process::Process>>> = {
            let mut shared = self.shared.lock().expect("ACP terminals");
            let processes = shared
                .terminals
                .values()
                .map(|terminal| terminal.process.clone())
                .collect();
            shared.terminals.clear();
            shared.order.clear();
            processes
        };
        for process in processes {
            process.lock().expect("ACP terminal").terminate();
        }
    }
}

impl Drop for Terminals {
    fn drop(&mut self) {
        self.dispose();
    }
}

#[cfg(test)]
#[path = "devin_tests.rs"]
mod tests;
