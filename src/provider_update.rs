//! One-button provider CLI updates, ported from KSG Code's
//! `providerMaintenanceRunner`, `ProviderInstallationGate` and `maintenanceResolver`
//! (design contract `ksg-update-design.md` sections 3 and 4).
//!
//! A provider is the harness CLI Shina launches: `codex` and `claude` only. Pi follows
//! Shina's own release and ACP harnesses are arbitrary commands, so neither is managed.
//!
//! Only three proven strategies execute, always as a fixed argv with no shell:
//! `npm-global` (the real path lives under a proven npm global prefix), Homebrew
//! (the keg's own `brew` reports the same prefix) and `claude-native`
//! (`<resolved> update` for `~/.local/share/claude/versions/` installs). Every other
//! layout reports a `disabled_reason` and, where certain, a `manual_command` that is
//! displayed but never run. A provider update never touches CPA, provider config,
//! auth files or `harnesses.json`.
//!
//! Contract for the root agent:
//! - `ProviderUpdates::new(data_dir)` reconciles `providers/update-state.json`, then
//!   `statuses()` serialises to the section 5 projection. `refresh(force)` drops the
//!   latest-version cache and re-resolves in the background; `poll()` picks the result
//!   up and returns true whenever a projected field changed. Call `poll()` only from
//!   the runtime tick; it never blocks.
//! - `request(driver, target, update_id)` is the `providers.update` backend: the record
//!   is durable before it returns and execution is driven by `poll()`. `admits(driver)`
//!   is the gate turnstile for new harness work, and `set_hooks` wires the busy check
//!   and the pre-run "stop idle harness processes" step. `active()` feeds the server
//!   restart blocker.
//! - RPC names and grant mapping (`providers.refresh`, `providers.update`, MINOR 36)
//!   belong to the parent.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

/// Native and package updaters download large binaries quietly; KSG allows 30 minutes.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const LATEST_TIMEOUT: Duration = Duration::from_secs(4);
const OUTPUT_MAX_BYTES: usize = 10_000;
const PROGRESS_MAX_LEN: usize = 200;
const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);
const LATEST_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_RECORDS_PER_DRIVER: usize = 20;
const WRAPPER_SCRIPT_MAX_BYTES: u64 = 16 * 1024;

// ---------- projected types (section 5 JSON) ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advisory {
    Current,
    Behind,
    Unknown,
}
impl Advisory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Behind => "behind",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRecord {
    pub update_id: String,
    /// queued | running | succeeded | unchanged | failed
    pub status: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
    /// Never projected by `statuses()`; only `refresh()` output carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub driver: String,
    pub name: String,
    pub installed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    pub advisory: String,
    /// "npm-global" | "homebrew-formula" | "homebrew-cask" | "claude-native" | null
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_command: Option<String>,
    /// Shown to the client with the home directory collapsed to `~`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdateRecord>,
}

// ---------- durable types (providers/update-state.json) ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRecord {
    update_id: String,
    driver: String,
    status: String,
    message: String,
    #[serde(default)]
    started_at: Option<u64>,
    #[serde(default)]
    finished_at: Option<u64>,
    #[serde(default)]
    from_version: Option<String>,
    #[serde(default)]
    target_version: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    lock_key: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ledger {
    version: u32,
    #[serde(default)]
    records: Vec<StoredRecord>,
}

// ---------- external effects, faked in tests ----------

#[derive(Clone, Debug, Default)]
pub struct ProbeResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Bounded, noninteractive probes: `<exe> --version`, `npm prefix -g`,
/// `brew --prefix`, `npm outdated`/`brew info`, and the claude-native registry GET.
pub trait Prober: Send + Sync {
    fn run(&self, argv: &[String], timeout: Duration) -> ProbeResult;
    fn http_get(&self, url: &str, timeout: Duration) -> Option<String>;
}

/// The running update command. `poll()` owns it; it never blocks the caller.
pub(crate) trait RunningUpdate: Send {
    /// Latest meaningful output line, sampled by `poll`.
    fn progress(&mut self) -> Option<String>;
    /// Finished result, or None while still running.
    fn result(&mut self) -> Option<CommandResult>;
    fn kill(&mut self);
}

#[derive(Clone, Debug, Default)]
pub struct CommandResult {
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

pub(crate) trait Runner: Send + Sync {
    fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String>;
}

// ---------- system implementations ----------

/// A bounded byte sink shared between a pump thread and the poller. The child
/// never blocks on output: the pump drains continuously and only the last
/// `OUTPUT_MAX_BYTES * 2` bytes are retained, so a 30-minute install cannot grow
/// memory unboundedly.
#[derive(Default)]
struct ByteTail {
    bytes: std::collections::VecDeque<u8>,
}
impl ByteTail {
    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend(chunk.iter().copied());
        while self.bytes.len() > OUTPUT_MAX_BYTES * 2 {
            self.bytes.pop_front();
        }
    }
    fn take(&mut self) -> Vec<u8> {
        self.bytes.drain(..).collect()
    }
}
type SharedTail = Arc<std::sync::Mutex<ByteTail>>;

fn pump(mut reader: impl std::io::Read + Send + 'static) -> SharedTail {
    let tail: SharedTail = Arc::new(std::sync::Mutex::new(ByteTail::default()));
    let writer = Arc::clone(&tail);
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Ok(mut tail) = writer.lock() {
                        tail.push(&buffer[..n]);
                    } else {
                        break;
                    }
                }
            }
        }
    });
    tail
}

/// Append raw bytes to a bounded UTF-8 tail without ever splitting a codepoint
/// at the trim boundary.
fn push_tail(tail: &mut String, bytes: &[u8]) {
    tail.push_str(&String::from_utf8_lossy(bytes));
    if tail.len() > OUTPUT_MAX_BYTES {
        let mut cut = tail.len() - OUTPUT_MAX_BYTES;
        while !tail.is_char_boundary(cut) {
            cut += 1;
        }
        tail.drain(..cut);
    }
}

fn run_bounded(argv: &[String], timeout: Duration) -> ProbeResult {
    let Some((program, args)) = argv.split_first() else {
        return ProbeResult::default();
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match crate::process::Process::spawn(&mut command) {
        Ok(child) => child,
        Err(error) => {
            return ProbeResult {
                stderr: format!("{program}: {error}"),
                ..Default::default()
            };
        }
    };
    let (out_rx, err_rx) = (
        pump(child.stdout.take().expect("piped stdout")),
        pump(child.stderr.take().expect("piped stderr")),
    );
    let deadline = Instant::now() + timeout;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() >= deadline => {
                child.terminate();
                let _ = child.reap(Duration::from_secs(2));
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let collect = |rx: &SharedTail| {
        let bytes = rx.lock().map(|mut t| t.take()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    thread::sleep(Duration::from_millis(30));
    ProbeResult {
        exit_code: code,
        stdout: strip_ansi(&collect(&out_rx)),
        stderr: strip_ansi(&collect(&err_rx)),
    }
}

struct SystemProber;
impl Prober for SystemProber {
    fn run(&self, argv: &[String], timeout: Duration) -> ProbeResult {
        run_bounded(argv, timeout)
    }
    fn http_get(&self, url: &str, timeout: Duration) -> Option<String> {
        // reqwest honours HTTPS_PROXY/HTTP_PROXY like KSG's server-side fetch.
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .ok()?;
        let body = client
            .get(url)
            .header("accept", "application/json")
            .send()
            .ok()?
            .text()
            .ok()?;
        serde_json::from_str::<serde_json::Value>(&body).ok()?["version"]
            .as_str()
            .map(str::to_owned)
    }
}

struct CommandUpdate {
    child: crate::process::Process,
    out: SharedTail,
    err: SharedTail,
    out_tail: String,
    err_tail: String,
}
impl CommandUpdate {
    fn drain(&mut self) {
        for (rx, tail) in [
            (&self.out, &mut self.out_tail),
            (&self.err, &mut self.err_tail),
        ] {
            let bytes = rx.lock().map(|mut t| t.take()).unwrap_or_default();
            push_tail(tail, &bytes);
        }
    }
}
impl RunningUpdate for CommandUpdate {
    fn progress(&mut self) -> Option<String> {
        self.drain();
        [&self.err_tail, &self.out_tail]
            .into_iter()
            .find_map(|tail| {
                tail.rsplit(['\r', '\n'])
                    .find_map(|l| progress_line_raw(l).map(|p| redact(&p)))
            })
    }
    fn result(&mut self) -> Option<CommandResult> {
        self.drain();
        let status = self.child.try_wait().ok()??;
        self.drain();
        Some(CommandResult {
            exit_code: status.code(),
            stdout_tail: strip_ansi(&self.out_tail),
            stderr_tail: strip_ansi(&self.err_tail),
        })
    }
    fn kill(&mut self) {
        self.child.terminate();
        let _ = self.child.reap(Duration::from_secs(2));
    }
}

struct SystemRunner;
impl Runner for SystemRunner {
    fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
        let Some((program, args)) = argv.split_first() else {
            return Err("Empty update command.".into());
        };
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = crate::process::Process::spawn(&mut command)
            .map_err(|error| format!("Could not start {program}: {error}"))?;
        Ok(Box::new(CommandUpdate {
            out: pump(child.stdout.take().expect("piped stdout")),
            err: pump(child.stderr.take().expect("piped stderr")),
            child,
            out_tail: String::new(),
            err_tail: String::new(),
        }))
    }
}

// ---------- detection (contract 4.1, 4.2) ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Driver {
    Codex,
    Claude,
}
impl Driver {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            _ => None,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }
    fn env_override(self) -> &'static str {
        match self {
            Self::Codex => "SHINA_CODEX_CLI",
            Self::Claude => "SHINA_CLAUDE_CLI",
        }
    }
    fn package(self) -> &'static str {
        match self {
            Self::Codex => "@openai/codex",
            Self::Claude => "@anthropic-ai/claude-code",
        }
    }
}

/// A proven, executable strategy. The argv it yields is fixed; no remote input
/// and no shell ever enters it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Strategy {
    /// `npm install -g --prefix <prefix> <pkg>@<target>`; prefix proven by `npm prefix -g`.
    /// `npm` is an argv prefix: `[npm]` natively, `[node, npm-cli.js]` for `.cmd` shims.
    NpmGlobal { prefix: PathBuf, npm: Vec<String> },
    /// `brew upgrade [--cask] <name>`; brew proven by `brew --prefix` == keg prefix.
    Homebrew {
        brew: Vec<String>,
        name: String,
        cask: bool,
    },
    /// `<resolvedPath> update`; real path under `~/.local/share/claude/versions/`.
    ClaudeNative { resolved: PathBuf },
}
impl Strategy {
    fn kind(&self) -> &'static str {
        match self {
            Self::NpmGlobal { .. } => "npm-global",
            Self::Homebrew { cask: false, .. } => "homebrew-formula",
            Self::Homebrew { cask: true, .. } => "homebrew-cask",
            Self::ClaudeNative { .. } => "claude-native",
        }
    }
    fn lock_key(&self) -> String {
        match self {
            Self::NpmGlobal { prefix, .. } => {
                format!("npm-global:{}", slash_path(prefix))
            }
            Self::Homebrew { .. } => "homebrew".into(),
            Self::ClaudeNative { .. } => "claude-native".into(),
        }
    }
    fn argv(&self, target: &str, package: &str) -> Vec<String> {
        match self {
            Self::NpmGlobal { prefix, npm } => {
                let mut argv = npm.clone();
                argv.extend([
                    "install".into(),
                    "-g".into(),
                    "--prefix".into(),
                    prefix.to_string_lossy().into_owned(),
                    format!("{package}@{target}"),
                ]);
                argv
            }
            Self::Homebrew { brew, name, cask } => {
                let mut argv = brew.clone();
                argv.push("upgrade".into());
                if *cask {
                    argv.push("--cask".into());
                }
                argv.push(name.clone());
                argv
            }
            Self::ClaudeNative { resolved } => {
                vec![resolved.to_string_lossy().into_owned(), "update".into()]
            }
        }
    }
    /// Display string for "Running ..." messages and manual hints.
    fn describe(&self, target: &str, package: &str) -> String {
        self.argv(target, package)
            .iter()
            .map(|word| {
                // Display only; the executed argv is the vector itself.
                if word
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/:.@-_\\".contains(&b))
                {
                    word.clone()
                } else {
                    format!("'{word}'")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Filesystem proof result for one driver.
#[derive(Clone, Debug)]
struct Detected {
    installed: bool,
    resolved: Option<PathBuf>,
    real: Option<PathBuf>,
    /// Extra argv prefix for `--version` when the launcher is `node <script>`.
    script_prefix: Option<Vec<String>>,
    strategy: Option<Strategy>,
    disabled_reason: Option<String>,
    manual_command: Option<String>,
}
impl Detected {
    fn missing() -> Self {
        Self {
            installed: false,
            resolved: None,
            real: None,
            script_prefix: None,
            strategy: None,
            disabled_reason: None,
            manual_command: None,
        }
    }
}

/// `/`-joined path with its original case: used for argv and install identity.
fn slash_path(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        if let Some(unc) = path.strip_prefix("//?/UNC/") {
            return format!("//{unc}");
        }
        if let Some(drive) = path.strip_prefix("//?/")
            && drive.as_bytes().get(1) == Some(&b':')
        {
            return drive.into();
        }
    }
    path
}

fn receive_worker<T>(receiver: &mpsc::Receiver<T>) -> Result<Option<T>, ()> {
    match receiver.try_recv() {
        Ok(value) => Ok(Some(value)),
        Err(mpsc::TryRecvError::Empty) => Ok(None),
        Err(mpsc::TryRecvError::Disconnected) => Err(()),
    }
}

fn same_updated_install(strategy: &Strategy, previous_real: &Path, fresh: &Detected) -> bool {
    if fresh.strategy.as_ref() != Some(strategy) {
        return false;
    }
    let Some(real) = &fresh.real else {
        return false;
    };
    match strategy {
        Strategy::NpmGlobal { .. } => real == previous_real,
        // Reproof already checked this brew's prefix and the same formula/cask.
        // Its canonical executable is expected to move to the new version's keg.
        Strategy::Homebrew { .. } => true,
        Strategy::ClaudeNative { .. } => {
            let root = |path: &Path| {
                slash_path(path)
                    .split_once("/.local/share/claude/versions/")
                    .map(|(root, _)| root.to_owned())
            };
            root(previous_real)
                .zip(root(real))
                .is_some_and(|(old, new)| old == new)
        }
    }
}

/// Case-folded form for pattern matching only. Never build a path from it:
/// case-sensitive installs would point at the wrong location.
fn match_key(text: &str) -> String {
    text.to_lowercase()
}

/// ASCII case-insensitive `find`; the index still slices `haystack` because the
/// needles here are pure ASCII and a non-matching region never verifies.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let index = haystack.to_lowercase().find(needle)?;
    haystack
        .get(index..index + needle.len())
        .filter(|m| m.eq_ignore_ascii_case(needle))
        .map(|_| index)
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn candidates(driver: Driver) -> Vec<String> {
    // Same order as `codex.rs::process` / `claude.rs::process`, minus env overrides,
    // which the caller checks first.
    match driver {
        Driver::Codex => {
            let mut out = vec!["codex".to_owned()];
            if cfg!(windows)
                && let Some(local) = std::env::var_os("LOCALAPPDATA")
            {
                out.push(
                    PathBuf::from(&local)
                        .join("Programs/OpenAI/Codex/bin/codex.exe")
                        .to_string_lossy()
                        .into_owned(),
                );
                if let Some(profile) = std::env::var_os("USERPROFILE") {
                    out.push(
                        PathBuf::from(profile)
                            .join("AppData/Local/Programs/OpenAI/Codex/bin/codex.exe")
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
            out
        }
        Driver::Claude => {
            let mut out = vec!["claude".to_owned()];
            if let Some(home) = home() {
                for relative in if cfg!(windows) {
                    vec![
                        ".local/bin/claude.exe",
                        ".claude/local/claude.exe",
                        "AppData/Roaming/npm/claude.cmd",
                    ]
                } else {
                    vec![
                        ".local/bin/claude",
                        ".claude/local/claude",
                        ".npm-global/bin/claude",
                    ]
                } {
                    out.push(home.join(relative).to_string_lossy().into_owned());
                }
            }
            out
        }
    }
}

fn is_mise(p: &str) -> bool {
    p.contains("/mise/installs/")
        || p.contains("/mise/shims/")
        || p.contains("/mise/command-wrappers/")
}
fn is_pnpm(p: &str) -> bool {
    p.contains("/.local/share/pnpm/")
        || p.contains("/library/pnpm/")
        || p.contains("/appdata/local/pnpm/")
        || p.contains("/pnpm/global/")
}
fn is_bun(p: &str) -> bool {
    p.contains("/.bun/")
}
fn is_volta(p: &str) -> bool {
    p.contains("/volta/tools/") || p.contains("/volta/bin/")
}
fn is_vite_plus(p: &str) -> bool {
    p.contains("/.vite-plus/") || p.contains("/vite-plus/bin/")
}
fn is_nix(p: &str) -> bool {
    p.starts_with("/nix/store/")
}
fn is_macports(p: &str) -> bool {
    p.starts_with("/opt/local/")
}
fn is_windows_managed(p: &str) -> Option<&'static str> {
    if p.contains("/winget/") {
        Some("winget")
    } else if p.contains("/chocolatey/") {
        Some("Chocolatey")
    } else if p.contains("/program files/") {
        Some("an administrator install")
    } else {
        None
    }
}
fn is_codex_desktop_bundle(p: &str) -> bool {
    p.contains("/openai/codex/") || p.contains("/codex.app/")
}

fn homebrew_keg(p: &str) -> Option<(String, String, bool)> {
    // (prefix, name, cask) from `<p>/Cellar/<name>/<v>/...` or `<p>/Caskroom/<name>/<v>/...`
    for (marker, cask) in [("/cellar/", false), ("/caskroom/", true)] {
        if let Some(index) = find_ci(p, marker) {
            let prefix = p[..index].to_owned();
            let name = p[index + marker.len()..]
                .split('/')
                .next()
                .unwrap_or_default()
                .to_owned();
            if !name.is_empty() {
                return Some((prefix, name, cask));
            }
        }
    }
    None
}

/// The npm global prefix owning `real`, per KSG `npmGlobalPrefixFromCommandPath`:
/// `<prefix>/lib/node_modules/<pkg>/...`, never nested under another package.
fn npm_prefix(real: &str, package: &str) -> Option<String> {
    npm_prefix_for(real, package, cfg!(windows))
}

fn npm_prefix_for(real: &str, package: &str, windows: bool) -> Option<String> {
    let posix = format!("/lib/node_modules/{}/", package.to_lowercase());
    let native = format!("/node_modules/{}/", package.to_lowercase());
    let lowered = real.to_lowercase();
    let segment = if lowered.contains(&posix) {
        posix
    } else if windows {
        native
    } else {
        return None;
    };
    let index = lowered.rfind(&segment)?;
    if !real.is_char_boundary(index)
        || !real
            .get(index..index + segment.len())
            .is_some_and(|m| m.eq_ignore_ascii_case(&segment))
    {
        return None;
    }
    if match_key(&real[..index]).contains("/node_modules/") {
        return None;
    }
    // Mise tool trees mimic npm globals; only globals under mise's Node count.
    let before = match_key(&real[..index]);
    if let Some(tool) = before
        .rsplit("/mise/installs/")
        .next()
        .and_then(|t| t.split('/').next())
        .filter(|_| before.contains("/mise/installs/"))
        && tool != "node"
    {
        return None;
    }
    Some(real[..index].to_owned())
}

fn is_wrapper_script(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() > WRAPPER_SCRIPT_MAX_BYTES {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 2];
    use std::io::Read;
    file.read_exact(&mut head).is_ok() && &head == b"#!"
}

/// Test-seedable overrides: candidate lists, tool paths (npm, brew), and an
/// injected durable-write failure for fail-closed coverage.
#[derive(Clone, Default)]
struct Overrides {
    candidates: HashMap<Driver, Vec<String>>,
    tools: HashMap<String, PathBuf>,
    fail_writes: Arc<std::sync::atomic::AtomicBool>,
}

/// A tool's argv prefix: `[exe]` for native binaries, `[node, entry.js]` when
/// the PATH hit is a `.cmd` shim or script (Windows npm is `npm.cmd`).
fn find_tool(name: &str, overrides: Option<&Overrides>) -> Option<Vec<String>> {
    let (exe, script) = overrides
        .and_then(|o| o.tools.get(name))
        .and_then(|p| crate::harness::executable(&p.to_string_lossy()).ok())
        .or_else(|| crate::harness::executable(name).ok())?;
    let mut argv = vec![exe.to_string_lossy().into_owned()];
    if let Some(script) = script {
        argv.push(script.to_string_lossy().into_owned());
    }
    Some(argv)
}

/// Full detection with prover access (contract 4.2). Runs `npm prefix -g`/
/// `brew --prefix` proofs, so it only ever runs on a background thread or in
/// tests, never inside `poll()` on the reducer thread.
fn detect(driver: Driver, prober: &dyn Prober, overrides: Option<&Overrides>) -> Detected {
    if let Some(command) = std::env::var_os(driver.env_override()) {
        let launch = overrides
            .is_none()
            .then(|| crate::harness::executable(&command.to_string_lossy()).ok())
            .flatten();
        let resolved = launch
            .as_ref()
            .map(|(exe, script)| script.as_ref().unwrap_or(exe).clone());
        let real = resolved.as_ref().and_then(|path| path.canonicalize().ok());
        let script_prefix = launch.and_then(|(exe, script)| {
            script.map(|script| {
                vec![
                    exe.to_string_lossy().into_owned(),
                    script.to_string_lossy().into_owned(),
                ]
            })
        });
        return Detected {
            installed: resolved.is_some(),
            resolved,
            real,
            script_prefix,
            disabled_reason: Some(format!(
                "Set by {}. Update it where it was installed.",
                driver.env_override()
            )),
            ..Detected::missing()
        };
    }
    // When overrides are present they are authoritative: a driver absent from
    // the map has no candidates, so tests never touch the real PATH.
    let owned;
    let list: &[String] = match overrides {
        Some(overrides) => overrides
            .candidates
            .get(&driver)
            .map(|v| v.as_slice())
            .unwrap_or(&[]),
        None => {
            owned = candidates(driver);
            &owned
        }
    };
    let mut found = None;
    for candidate in list {
        if let Ok((executable, script)) = crate::harness::executable(&candidate) {
            found = Some((executable, script));
            break;
        }
    }
    let Some((executable, script)) = found else {
        return Detected {
            disabled_reason: Some(format!("{} is not installed.", driver.name())),
            ..Detected::missing()
        };
    };
    // Ownership is judged on what an updater would replace: the entry point.
    let resolved = script.clone().unwrap_or_else(|| executable.clone());
    let script_prefix = script.map(|entry| {
        vec![
            executable.to_string_lossy().into_owned(),
            entry.to_string_lossy().into_owned(),
        ]
    });
    let Ok(real) = std::fs::canonicalize(&resolved) else {
        return Detected {
            installed: true,
            resolved: Some(resolved.clone()),
            real: None,
            script_prefix,
            strategy: None,
            disabled_reason: Some(format!(
                "Could not resolve {}. Update it where it was installed.",
                driver.name()
            )),
            manual_command: None,
        };
    };
    classify_resolved(driver, resolved, real, script_prefix, prober, overrides)
}

// ---------- versions (contract 4.3) ----------

/// Strict semver: `x.y.z` numeric parts plus optional `-prerelease`/`+build`.
/// Anything looser ("1.2.garbage", "v2", dates) is not an installable version.
fn valid_semver(value: &str) -> bool {
    let without_build = value.split('+').next().unwrap_or("");
    let (main, prerelease) = match without_build.split_once('-') {
        Some((m, p)) => (m, Some(p)),
        None => (without_build, None),
    };
    let parts: Vec<&str> = main.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    prerelease.is_none_or(|p| {
        !p.is_empty()
            && p.split('.')
                .all(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    })
}

/// First strict semver in version output, or None. Eligibility and post-update
/// verification both refuse unverifiable output rather than guessing.
fn parse_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() {
            continue;
        }
        let end = bytes[start..]
            .iter()
            .position(|b| !(b.is_ascii_alphanumeric() || b".-_".contains(b)))
            .map_or(bytes.len(), |n| start + n);
        let candidate = text[start..end].trim_end_matches(['.', '-', '_']);
        if valid_semver(candidate) {
            return Some(candidate.to_owned());
        }
    }
    None
}

/// KSG's `compareSemverVersions` (shared parser in `model_picker`).
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    crate::model_picker::compare_semver_versions(a, b)
}

/// Removes URL credentials and token-shaped words from captured installer or
/// probe output before it reaches a record, message, or the projection.
fn redact(text: &str) -> String {
    crate::update::redact(&crate::projects::redact(text))
}

fn installed_version(detected: &Detected, prober: &dyn Prober) -> Option<String> {
    let resolved = detected.resolved.as_ref()?;
    let mut argv = detected
        .script_prefix
        .clone()
        .unwrap_or_else(|| vec![resolved.to_string_lossy().into_owned()]);
    argv.push("--version".into());
    let result = prober.run(&argv, PROBE_TIMEOUT);
    if result.exit_code != Some(0) {
        return None;
    }
    parse_version(&result.stdout).or_else(|| parse_version(&result.stderr))
}

fn latest_version(driver: Driver, strategy: &Strategy, prober: &dyn Prober) -> Option<String> {
    match strategy {
        Strategy::NpmGlobal { prefix, npm } => {
            // `outdated` respects the user's npm config (min-release-age, registry).
            let argv: Vec<String> = npm
                .iter()
                .cloned()
                .chain([
                    "outdated".into(),
                    "-g".into(),
                    "--prefix".into(),
                    prefix.to_string_lossy().into_owned(),
                    "--json".into(),
                    driver.package().into(),
                ])
                .collect();
            let result = prober.run(&argv, PROBE_TIMEOUT);
            // npm outdated exits 1 when updates exist, and may print {"error":...}.
            let value: serde_json::Value = serde_json::from_str(result.stdout.trim()).ok()?;
            value.get(driver.package())?["latest"]
                .as_str()
                .map(str::to_owned)
        }
        Strategy::Homebrew { brew, name, cask } => {
            let argv: Vec<String> = brew
                .iter()
                .cloned()
                .chain(["info".into(), "--json=v2".into(), name.clone()])
                .collect();
            let result = prober.run(&argv, PROBE_TIMEOUT);
            let value: serde_json::Value = serde_json::from_str(result.stdout.trim()).ok()?;
            let raw = if *cask {
                value["casks"][0]["version"]
                    .as_str()
                    .map(|v| v.split(',').next().unwrap_or(v).to_owned())
            } else {
                value["formulae"][0]["versions"]["stable"]
                    .as_str()
                    .map(str::to_owned)
            };
            raw.filter(|v| v != "latest")
        }
        Strategy::ClaudeNative { .. } => prober.http_get(
            "https://registry.npmjs.org/@anthropic-ai/claude-code/latest",
            LATEST_TIMEOUT,
        ),
    }
}

// ---------- gate hooks owned by the root ----------

/// Root-wired admission control (contract 4.6). `busy(driver)` reports in-flight
/// harness work for that driver; `before_run(driver)` stops idle harness processes
/// immediately before the update command spawns. The permissive defaults exist for
/// standalone tests only: the parent MUST refuse new harness work while
/// `!admits(driver)` and must have stopped idle harness processes before calling
/// `request`, because a provider update replaces the binary those processes run.
#[derive(Clone)]
pub struct Hooks {
    pub busy: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    pub before_run: Arc<dyn Fn(&str) + Send + Sync>,
}
impl Default for Hooks {
    fn default() -> Self {
        Self {
            busy: Arc::new(|_| false),
            before_run: Arc::new(|_| {}),
        }
    }
}

// ---------- worker state ----------

enum Job {
    /// Under-lock re-detection with full ownership proofs before the command.
    Reprove(mpsc::Receiver<(Detected, Option<String>)>),
    /// `npm outdated`/`brew info`/registry GET for the pre-update check.
    Latest(mpsc::Receiver<Option<String>>),
    /// The update command itself.
    Command {
        handle: Box<dyn RunningUpdate>,
        started: Instant,
        last_progress: Instant,
    },
    /// Post-update re-detection plus `<exe> --version`.
    Verify(mpsc::Receiver<(Detected, Option<String>)>),
}

struct DetectionOutcome {
    detected: Detected,
    version: Option<String>,
    latest: Option<String>,
    checked_at: u64,
}

struct Active {
    record: UpdateRecord,
    lock_key: String,
    /// Held cross-process lock under providers/locks/; released on drop.
    lock_file: Option<std::fs::File>,
    phase: Phase,
    strategy: Strategy,
    real_path: PathBuf,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Queued,
    Reproving,
    CheckingLatest,
    WaitBusy,
    Running,
    Verifying,
}

// ---------- the service ----------

pub struct ProviderUpdates {
    data_dir: PathBuf,
    ledger: Ledger,
    /// Latest detection per driver, swapped in by the detect job.
    detected: HashMap<Driver, DetectionOutcome>,
    /// Refresh/startup detection jobs; kept separate from `jobs` so a refresh
    /// can never clobber an in-flight update command, reprove, or verify.
    detect_jobs: HashMap<Driver, mpsc::Receiver<DetectionOutcome>>,
    jobs: HashMap<Driver, Job>,
    latest_cache: HashMap<String, (Instant, Option<String>)>,
    active: HashMap<Driver, Active>,
    hooks: Hooks,
    prober: Arc<dyn Prober>,
    runner: Arc<dyn Runner>,
    /// Test hook: replaces the PATH-based candidate list.
    overrides: Option<Overrides>,
    update_timeout: Duration,
    /// Durable store failure: corrupt input or a failed atomic write. Fails
    /// closed - no request is accepted and nothing installs while set.
    failure: Option<String>,
}

impl ProviderUpdates {
    /// Production constructor. Reconciles `providers/update-state.json`, then
    /// detection; statuses land once `poll()` picks the detection job up.
    ///
    /// Under `cfg(test)` detection is hermetic: empty candidate overrides mean
    /// no real CLI is ever resolved and no probe or network call runs, so unit
    /// fixtures that construct the service stay offline. The module's own tests
    /// use `build` with fake probers/runners and seeded candidates.
    pub fn new(data_dir: &Path) -> Self {
        #[cfg(not(test))]
        let overrides = None;
        #[cfg(test)]
        let overrides = Some(Overrides::default());
        Self::build(
            data_dir,
            Hooks::default(),
            Arc::new(SystemProber),
            Arc::new(SystemRunner),
            overrides,
            UPDATE_TIMEOUT,
        )
    }

    fn build(
        data_dir: &Path,
        hooks: Hooks,
        prober: Arc<dyn Prober>,
        runner: Arc<dyn Runner>,
        overrides: Option<Overrides>,
        update_timeout: Duration,
    ) -> Self {
        let (ledger, failure) = load_ledger(&data_dir.join("providers/update-state.json"));
        let mut service = Self {
            data_dir: data_dir.to_path_buf(),
            ledger,
            detected: HashMap::new(),
            detect_jobs: HashMap::new(),
            jobs: HashMap::new(),
            latest_cache: HashMap::new(),
            active: HashMap::new(),
            hooks,
            prober,
            runner,
            overrides,
            update_timeout,
            failure,
        };
        // A queued/running record whose lock is free died with its process: the
        // OS releases the file lock when the owner exits, so an acquirable lock
        // proves the previous run is gone. A lock still held by another live
        // process leaves the record alone rather than re-dispatching it.
        if service.failure.is_none() {
            let now = crate::lifecycle::now();
            let mut recovered = false;
            for record in &mut service.ledger.records {
                if matches!(record.status.as_str(), "succeeded" | "unchanged" | "failed") {
                    continue;
                }
                let free = record
                    .lock_key
                    .as_deref()
                    .is_none_or(|key| try_lock_file(&service.data_dir, key).is_some());
                if !free {
                    continue;
                }
                record.status = "failed".into();
                record.finished_at = Some(now);
                let name = Driver::from_id(&record.driver)
                    .map_or(record.driver.clone(), |d| d.name().to_owned());
                record.message = format!(
                    "Shina stopped while the {name} update was running. Check {name} and retry."
                );
                recovered = true;
            }
            if recovered {
                let _ = service.persist();
            }
        }
        service.start_detect();
        service
    }

    /// Wire the root's gate hooks (contract 4.6).
    #[cfg(all(test, unix))]
    pub fn set_hooks(&mut self, hooks: Hooks) {
        self.hooks = hooks;
    }

    fn state_path(&self) -> PathBuf {
        self.data_dir.join("providers/update-state.json")
    }

    /// Atomic ledger write. Fails closed: a failed write marks the service so
    /// no request is accepted and no installer runs on an out-of-date ledger.
    fn persist(&mut self) -> Result<(), String> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let write = if self
            .overrides
            .as_ref()
            .is_some_and(|o| o.fail_writes.load(std::sync::atomic::Ordering::Relaxed))
        {
            Err("injected write failure".to_owned())
        } else {
            serde_json::to_vec(&self.ledger)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    crate::store::write_atomic(&self.state_path(), &bytes)
                        .map_err(|e| e.to_string())
                })
        };
        write.map_err(|error| {
            let failure = format!("Could not persist provider update state: {error}");
            self.failure = Some(failure.clone());
            failure
        })
    }

    /// Durable-store failure the root can quarantine on. While Some, every
    /// mutation API refuses and no install can start.
    pub fn failure(&self) -> Option<String> {
        self.failure.clone()
    }

    fn start_detect(&mut self) {
        for driver in [Driver::Codex, Driver::Claude] {
            if self.detect_jobs.contains_key(&driver) {
                continue;
            }
            let prober = Arc::clone(&self.prober);
            let overrides = self.overrides.clone();
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = tx.send(resolve_driver(driver, &*prober, overrides.as_ref()));
            });
            self.detect_jobs.insert(driver, rx);
        }
    }

    /// Section 5 projection. `output` is stripped: it is returned by `refresh` only.
    pub fn statuses(&self) -> Vec<ProviderStatus> {
        self.statuses_inner(false)
    }

    fn statuses_inner(&self, with_output: bool) -> Vec<ProviderStatus> {
        [Driver::Codex, Driver::Claude]
            .into_iter()
            .map(|driver| {
                let detection = self.detected.get(&driver);
                let record = self
                    .active
                    .get(&driver)
                    .map(|a| a.record.clone())
                    .or_else(|| {
                        self.ledger
                            .records
                            .iter()
                            .rev()
                            .find(|r| r.driver == driver.id())
                            .map(|r| UpdateRecord {
                                update_id: r.update_id.clone(),
                                status: r.status.clone(),
                                message: r.message.clone(),
                                started_at: r.started_at,
                                finished_at: r.finished_at,
                                from_version: r.from_version.clone(),
                                target_version: r.target_version.clone(),
                                output: if with_output { r.output.clone() } else { None },
                            })
                    });
                let mut status = match detection {
                    Some(outcome) => {
                        let detected = &outcome.detected;
                        let advisory = match (&outcome.version, &outcome.latest) {
                            (Some(version), Some(latest))
                                if compare_versions(version, latest)
                                    == std::cmp::Ordering::Less =>
                            {
                                Advisory::Behind
                            }
                            (Some(_), Some(_)) => Advisory::Current,
                            _ => Advisory::Unknown,
                        };
                        ProviderStatus {
                            driver: driver.id().into(),
                            name: driver.name().into(),
                            installed: detected.installed,
                            version: outcome.version.clone(),
                            latest_version: outcome.latest.clone(),
                            advisory: advisory.as_str().into(),
                            strategy: detected
                                .strategy
                                .as_ref()
                                .map(Strategy::kind)
                                .map(str::to_owned),
                            disabled_reason: detected.disabled_reason.clone(),
                            manual_command: detected.manual_command.clone(),
                            install_path: detected.real.as_ref().map(|p| collapse_home(p)),
                            checked_at: Some(outcome.checked_at),
                            update: record.map(|mut r| {
                                if !with_output {
                                    r.output = None;
                                }
                                r
                            }),
                        }
                    }
                    None => ProviderStatus {
                        driver: driver.id().into(),
                        name: driver.name().into(),
                        installed: false,
                        version: None,
                        latest_version: None,
                        advisory: Advisory::Unknown.as_str().into(),
                        strategy: None,
                        disabled_reason: None,
                        manual_command: None,
                        install_path: None,
                        checked_at: None,
                        update: record,
                    },
                };
                if let Some(failure) = &self.failure {
                    status.strategy = None;
                    status.disabled_reason = Some(failure.clone());
                }
                status
            })
            .collect()
    }

    /// `providers.refresh`. Drops the latest-version cache and re-resolves in the
    /// background; returns the current view immediately, with `output` attached.
    pub fn refresh(&mut self, force: bool) -> Vec<ProviderStatus> {
        if force {
            self.latest_cache.clear();
        }
        self.start_detect();
        self.statuses_inner(true)
    }

    /// Pre-commit validation for `providers.update`: pure, no mutation, no
    /// probes, no waits. The runtime calls this before accepting a receipt, then
    /// calls `request` with the accepted update id. Same section 5 rejections.
    pub fn validate_request(&self, driver: &str, target: &str) -> Result<(), String> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let driver =
            Driver::from_id(driver).ok_or_else(|| format!("Unknown provider {driver}."))?;
        let Some(outcome) = self.detected.get(&driver) else {
            return Err("Refresh and try again.".into());
        };
        if self.active.contains_key(&driver) {
            return Err(format!(
                "An update is already running for {}.",
                driver.name()
            ));
        }
        if !outcome.detected.installed {
            return Err(format!("{} is not installed.", driver.name()));
        }
        if outcome.detected.strategy.is_none() {
            return Err(outcome
                .detected
                .disabled_reason
                .clone()
                .unwrap_or_else(|| format!("{} cannot be updated here.", driver.name())));
        }
        if outcome.latest.as_deref() != Some(target) {
            return Err("Refresh and try again.".into());
        }
        Ok(())
    }

    /// `providers.update` post-acceptance entry. The same update id with the same
    /// payload replays its stored outcome (never re-runs); a different payload is
    /// refused. The durable record exists before `poll()` may spawn anything.
    pub fn request(&mut self, driver: &str, target: &str, update_id: String) -> Result<(), String> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let driver =
            Driver::from_id(driver).ok_or_else(|| format!("Unknown provider {driver}."))?;
        // Idempotency: only the same updateId with the same payload replays its
        // stored receipt. A different payload under it is refused, and a new id
        // while an update is active is rejected by `validate_request`.
        if let Some(existing) = self
            .ledger
            .records
            .iter()
            .find(|r| r.update_id == update_id)
        {
            let same = existing.driver == driver.id()
                && existing.target_version.as_deref() == Some(target);
            return if same {
                Ok(())
            } else {
                Err("A different payload under that update id is refused.".into())
            };
        }
        self.validate_request(driver.id(), target)?;
        let outcome = self.detected.get(&driver).expect("validated");
        let strategy = outcome.detected.strategy.clone().expect("validated");
        let real = outcome.detected.real.clone().expect("validated");
        let record = UpdateRecord {
            update_id: update_id.clone(),
            status: "queued".into(),
            message: "Waiting for another provider update to finish.".into(),
            started_at: Some(crate::lifecycle::now()),
            finished_at: None,
            from_version: outcome.version.clone(),
            target_version: Some(target.to_owned()),
            output: None,
        };
        let lock_key = strategy.lock_key();
        self.ledger.records.push(stored(&record, driver, &lock_key));
        trim_records(&mut self.ledger, driver.id());
        // Fail closed: the record must be durable before any install side
        // effect; a failed write drops the intent and marks the service.
        if let Err(error) = self.persist() {
            self.ledger.records.retain(|r| r.update_id != update_id);
            return Err(error);
        }
        self.active.insert(
            driver,
            Active {
                record,
                lock_key,
                lock_file: None,
                phase: Phase::Queued,
                strategy,
                real_path: real,
            },
        );
        Ok(())
    }

    /// Marks an accepted update id as terminally failed without running anything,
    /// so the projection preserves the accepted receipt's outcome when the root
    /// learns the update cannot proceed after acceptance (e.g. a refused restart
    /// gate or a lost worker). Releases the driver lock when the id is active.
    /// Returns the durable-write error, if any.
    pub fn record_failure(
        &mut self,
        driver: &str,
        target: &str,
        update_id: String,
        message: String,
    ) -> Result<(), String> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let Some(driver) = Driver::from_id(driver) else {
            return Err("Unknown provider.".into());
        };
        // Same-id terminal failure for the accepted payload only.
        if self
            .ledger
            .records
            .iter()
            .any(|r| r.update_id == update_id && r.target_version.as_deref() != Some(target))
        {
            return Err("A different payload under that update id is refused.".into());
        }
        // If the accepted id is still active, release it (its lock drops).
        if self
            .active
            .get(&driver)
            .is_some_and(|a| a.record.update_id == update_id)
        {
            self.active.remove(&driver);
        } else if self.active.contains_key(&driver) {
            return Err(format!(
                "An update is already running for {}.",
                driver.name()
            ));
        }
        let now = crate::lifecycle::now();
        let stored_record = StoredRecord {
            update_id: update_id.clone(),
            driver: driver.id().into(),
            status: "failed".into(),
            message: redact(&message),
            started_at: Some(now),
            finished_at: Some(now),
            from_version: None,
            target_version: Some(target.to_owned()),
            output: None,
            lock_key: None,
        };
        if let Some(existing) = self
            .ledger
            .records
            .iter_mut()
            .find(|r| r.update_id == update_id)
        {
            *existing = stored_record;
        } else {
            self.ledger.records.push(stored_record);
        }
        trim_records(&mut self.ledger, driver.id());
        self.persist()
    }

    /// True while any record is queued or running. Feeds the server restart
    /// blocker (contract 5: "A provider update is running.").
    pub fn active(&self) -> bool {
        !self.active.is_empty()
    }

    /// Gate turnstile (contract 4.6): the root refuses or queues new harness work
    /// for a driver while its update is queued, waiting, or running.
    pub fn admits(&self, driver: &str) -> bool {
        Driver::from_id(driver).is_none_or(|d| !self.active.contains_key(&d))
    }

    /// Runtime tick. Advances every queued/running record without blocking and
    /// picks up finished background jobs. True when a projected field changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        // Detection jobs.
        for driver in [Driver::Codex, Driver::Claude] {
            if let Some(rx) = self.detect_jobs.get(&driver)
                && let Ok(outcome) = rx.try_recv()
            {
                self.detected.insert(driver, outcome);
                self.detect_jobs.remove(&driver);
                changed = true;
            }
        }
        for driver in [Driver::Codex, Driver::Claude] {
            if self.step(driver) {
                changed = true;
            }
        }
        changed
    }

    /// One nonblocking step of a driver's state machine (contract 4.5).
    fn step(&mut self, driver: Driver) -> bool {
        let Some((phase, lock_key, strategy, real_path)) = self.active.get(&driver).map(|a| {
            (
                a.phase,
                a.lock_key.clone(),
                a.strategy.clone(),
                a.real_path.clone(),
            )
        }) else {
            return false;
        };
        match phase {
            Phase::Queued => {
                // Cross-process lock on the install location, held until settle.
                let Some(lock) = try_lock_file(&self.data_dir, &lock_key) else {
                    return false;
                };
                let prober = Arc::clone(&self.prober);
                let overrides = self.overrides.clone();
                let (tx, rx) = mpsc::channel();
                // Ownership is re-proven under the lock on a worker thread with
                // the full npm/brew prover calls, never on the reducer.
                thread::spawn(move || {
                    let detected = detect(driver, &*prober, overrides.as_ref());
                    let version = installed_version(&detected, &*prober);
                    let _ = tx.send((detected, version));
                });
                if let Some(active) = self.active.get_mut(&driver) {
                    active.lock_file = Some(lock);
                    active.phase = Phase::Reproving;
                    active.record.status = "running".into();
                    active.record.message = "Checking the installation".into();
                }
                self.jobs.insert(driver, Job::Reprove(rx));
                self.sync_record(driver);
                true
            }
            Phase::Reproving => {
                let done = if let Some(Job::Reprove(rx)) = self.jobs.get(&driver) {
                    receive_worker(rx)
                } else {
                    Err(())
                };
                let (fresh, version) = match done {
                    Ok(Some(fresh)) => fresh,
                    Ok(None) => return false,
                    Err(()) => return self.worker_stopped(driver),
                };
                self.jobs.remove(&driver);
                // Same strategy and same canonical install, or nothing runs.
                let same = fresh.strategy.as_ref().map(Strategy::lock_key)
                    == Some(lock_key.clone())
                    && fresh.real.as_ref() == Some(&real_path);
                if !same {
                    self.settle(
                        driver,
                        "failed",
                        "Provider installation changed. Refresh and try again.".into(),
                        None,
                    );
                    return true;
                }
                if let Some(active) = self.active.get_mut(&driver) {
                    active.record.from_version = version;
                    active.phase = Phase::CheckingLatest;
                    active.record.status = "running".into();
                    active.record.message = "Checking for the latest version".into();
                }
                self.sync_record(driver);
                // Latest version on a bounded background probe.
                let cached = self
                    .latest_cache
                    .get(&lock_key)
                    .filter(|(at, _)| at.elapsed() < LATEST_CACHE_TTL)
                    .and_then(|(_, v)| v.clone());
                if let Some(latest) = cached {
                    self.finish_latest_check(driver, Some(latest));
                } else {
                    let prober = Arc::clone(&self.prober);
                    let (tx, rx) = mpsc::channel();
                    thread::spawn(move || {
                        let _ = tx.send(latest_version(driver, &strategy, &*prober));
                    });
                    self.jobs.insert(driver, Job::Latest(rx));
                }
                true
            }
            Phase::CheckingLatest => {
                let done = if let Some(Job::Latest(rx)) = self.jobs.get(&driver) {
                    receive_worker(rx)
                } else {
                    Err(())
                };
                let done = match done {
                    Ok(Some(done)) => done,
                    Ok(None) => return false,
                    Err(()) => return self.worker_stopped(driver),
                };
                self.jobs.remove(&driver);
                self.latest_cache.insert(
                    self.active[&driver].lock_key.clone(),
                    (Instant::now(), done.clone()),
                );
                self.finish_latest_check(driver, done);
                true
            }
            Phase::WaitBusy => {
                if (self.hooks.busy)(driver.id()) {
                    return false;
                }
                let target = self.active[&driver]
                    .record
                    .target_version
                    .clone()
                    .unwrap_or_default();
                let argv = self.active[&driver]
                    .strategy
                    .argv(&target, driver.package());
                let describe = self.active[&driver]
                    .strategy
                    .describe(&target, driver.package());
                (self.hooks.before_run)(driver.id());
                match self.runner.start(&argv) {
                    Ok(handle) => {
                        let active = self.active.get_mut(&driver).expect("active");
                        active.phase = Phase::Running;
                        active.record.status = "running".into();
                        active.record.message = format!("Running {describe}");
                        self.jobs.insert(
                            driver,
                            Job::Command {
                                handle,
                                started: Instant::now(),
                                last_progress: Instant::now() - PROGRESS_INTERVAL,
                            },
                        );
                        self.sync_record(driver);
                    }
                    Err(error) => {
                        self.settle(driver, "failed", error, None);
                    }
                }
                true
            }
            Phase::Running => {
                let mut line = None;
                let mut finished = None;
                let timeout = self.update_timeout;
                if let Some(Job::Command {
                    handle,
                    started,
                    last_progress,
                }) = self.jobs.get_mut(&driver)
                {
                    if let Some(progress) = handle.progress()
                        && last_progress.elapsed() >= PROGRESS_INTERVAL
                    {
                        *last_progress = Instant::now();
                        line = Some(progress);
                    }
                    if let Some(result) = handle.result() {
                        finished = Some(Ok(result));
                    } else if started.elapsed() >= timeout {
                        handle.kill();
                        finished = Some(Err(format!(
                            "Update timed out after {} minutes.",
                            timeout.as_secs().max(59) / 60
                        )));
                    }
                }
                let progressed = line.is_some();
                if let Some(line) = line {
                    self.message(driver, &line);
                }
                let Some(finished) = finished else {
                    return progressed;
                };
                self.jobs.remove(&driver);
                match finished {
                    Err(message) => {
                        self.settle(driver, "failed", message, None);
                    }
                    Ok(result) if result.exit_code != Some(0) => {
                        let message = failure_message(&result);
                        self.settle(driver, "failed", message, command_output(&result));
                    }
                    Ok(_) => {
                        self.message(driver, "Verifying the installed version");
                        if let Some(active) = self.active.get_mut(&driver) {
                            active.phase = Phase::Verifying;
                        }
                        // Re-resolve the owned location and probe its version on
                        // a worker, so a swapped install cannot verify a wrapper.
                        let prober = Arc::clone(&self.prober);
                        let overrides = self.overrides.clone();
                        let (tx, rx) = mpsc::channel();
                        thread::spawn(move || {
                            let detected = detect(driver, &*prober, overrides.as_ref());
                            let version = installed_version(&detected, &*prober);
                            let _ = tx.send((detected, version));
                        });
                        self.jobs.insert(driver, Job::Verify(rx));
                    }
                }
                true
            }
            Phase::Verifying => {
                let done = if let Some(Job::Verify(rx)) = self.jobs.get(&driver) {
                    receive_worker(rx)
                } else {
                    Err(())
                };
                let (fresh, version) = match done {
                    Ok(Some(done)) => done,
                    Ok(None) => return false,
                    Err(()) => return self.worker_stopped(driver),
                };
                self.jobs.remove(&driver);
                let lock_key = self.active[&driver].lock_key.clone();
                let real_path = self.active[&driver].real_path.clone();
                let name = driver.name();
                let target = self.active[&driver].record.target_version.clone();
                // The update must have landed in the same owned location it was
                // proven for; a different path means something else answered.
                let same = same_updated_install(&strategy, &real_path, &fresh);
                if !same {
                    self.settle(
                        driver,
                        "failed",
                        "Provider installation changed. Refresh and try again.".into(),
                        None,
                    );
                    return true;
                }
                match (version, target) {
                    (None, _) => self.settle(
                        driver,
                        "failed",
                        "Could not verify the installed version.".into(),
                        None,
                    ),
                    (Some(version), target) => {
                        let reached = target.as_deref().is_none_or(|target| {
                            compare_versions(&version, target) != std::cmp::Ordering::Less
                        });
                        if reached {
                            self.settle(
                                driver,
                                "succeeded",
                                format!("{name} updated: v{version}"),
                                None,
                            );
                        } else {
                            self.settle(
                                driver,
                                "unchanged",
                                format!("{name} still needs an update"),
                                None,
                            );
                        }
                    }
                }
                self.latest_cache.remove(&lock_key);
                true
            }
        }
    }

    fn worker_stopped(&mut self, driver: Driver) -> bool {
        self.jobs.remove(&driver);
        self.settle(
            driver,
            "failed",
            "The provider update worker stopped unexpectedly. Refresh and retry explicitly.".into(),
            None,
        );
        true
    }

    fn finish_latest_check(&mut self, driver: Driver, _latest: Option<String>) {
        let Some(active) = self.active.get_mut(&driver) else {
            return;
        };
        // Precheck (contract 4.5 step 3): already current needs no command.
        let current = matches!(
            (&active.record.from_version, &active.record.target_version),
            (Some(from), Some(target))
                if compare_versions(from, target) != std::cmp::Ordering::Less
        );
        if current {
            self.settle(driver, "succeeded", "Already up to date.".into(), None);
            return;
        }
        active.phase = Phase::WaitBusy;
        active.record.status = "running".into();
        active.record.message = format!("Waiting for running {} work to finish.", driver.name());
        self.sync_record(driver);
    }

    /// Mirror the active record into the durable ledger on every transition.
    fn sync_record(&mut self, driver: Driver) {
        let Some(active) = self.active.get(&driver) else {
            return;
        };
        let stored = stored(&active.record, driver, &active.lock_key);
        if let Some(existing) = self
            .ledger
            .records
            .iter_mut()
            .find(|r| r.update_id == stored.update_id)
        {
            *existing = stored;
        } else {
            self.ledger.records.push(stored);
        }
        let _ = self.persist();
    }

    fn message(&mut self, driver: Driver, message: &str) {
        if let Some(active) = self.active.get_mut(&driver) {
            active.record.message = message.to_owned();
            self.sync_record(driver);
        }
    }

    fn settle(&mut self, driver: Driver, status: &str, message: String, output: Option<String>) {
        if let Some(mut active) = self.active.remove(&driver) {
            active.record.status = status.into();
            active.record.message = message;
            active.record.finished_at = Some(crate::lifecycle::now());
            if output.is_some() {
                active.record.output = output;
            }
            let stored = stored(&active.record, driver, &active.lock_key);
            if let Some(existing) = self
                .ledger
                .records
                .iter_mut()
                .find(|r| r.update_id == stored.update_id)
            {
                *existing = stored;
            } else {
                self.ledger.records.push(stored);
            }
            // Dropping the File releases the OS lock; the lock file stays on
            // disk harmlessly and is never unlinked while held.
            drop(active.lock_file.take());
            trim_records(&mut self.ledger, driver.id());
            let _ = self.persist();
            // A settled update changes the projected advisory; redetect lazily.
            self.start_detect();
        }
    }
}

/// Cross-process advisory lock on an install location. The lock is held by the
/// open File; the OS releases it automatically if the process dies, so a stale
/// lock can never strand retries. Some means this process now owns it.
fn try_lock_file(data_dir: &Path, lock_key: &str) -> Option<std::fs::File> {
    let path = lock_path(data_dir, lock_key);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .ok()?;
    file.try_lock().ok()?;
    Some(file)
}

fn stored(record: &UpdateRecord, driver: Driver, lock_key: &str) -> StoredRecord {
    StoredRecord {
        update_id: record.update_id.clone(),
        driver: driver.id().into(),
        status: record.status.clone(),
        message: record.message.clone(),
        started_at: record.started_at,
        finished_at: record.finished_at,
        from_version: record.from_version.clone(),
        target_version: record.target_version.clone(),
        output: record.output.clone(),
        lock_key: Some(lock_key.to_owned()),
    }
}

fn lock_path(data_dir: &Path, lock_key: &str) -> PathBuf {
    let digest = sha2::Sha256::digest(lock_key.as_bytes());
    let name = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .take(8)
        .collect::<String>();
    data_dir
        .join("providers/locks")
        .join(format!("{name}.lock"))
}

/// Durable ledger load. Missing is a fresh ledger; unreadable, malformed, or a
/// foreign version is a service failure that preserves the file and disables
/// updates rather than overwriting whatever is there.
fn load_ledger(path: &Path) -> (Ledger, Option<String>) {
    let empty = || Ledger {
        version: 1,
        records: vec![],
    };
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Ledger>(&bytes) {
            Ok(ledger) if ledger.version == 1 => (ledger, None),
            Ok(_) => (
                empty(),
                Some("Provider update state has an unknown version; updates are disabled.".into()),
            ),
            Err(_) => (
                empty(),
                Some("Provider update state is unreadable; updates are disabled.".into()),
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (empty(), None),
        Err(error) => (
            empty(),
            Some(format!("Could not read provider update state: {error}")),
        ),
    }
}

fn trim_records(ledger: &mut Ledger, driver: &str) {
    let finished: Vec<usize> = ledger
        .records
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.driver == driver && matches!(r.status.as_str(), "succeeded" | "unchanged" | "failed")
        })
        .map(|(i, _)| i)
        .collect();
    for index in finished
        .iter()
        .take(finished.len().saturating_sub(MAX_RECORDS_PER_DRIVER))
        .rev()
    {
        ledger.records.remove(*index);
    }
}

fn collapse_home(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    home()
        .and_then(|home| {
            let home = home.to_string_lossy().into_owned();
            text.strip_prefix(&home).map(|rest| format!("~{rest}"))
        })
        .unwrap_or(text)
}

/// Detection plus installed/latest versions for one driver.
fn resolve_driver(
    driver: Driver,
    prober: &dyn Prober,
    overrides: Option<&Overrides>,
) -> DetectionOutcome {
    let detected = detect(driver, prober, overrides);
    let version = if detected.installed {
        installed_version(&detected, prober)
    } else {
        None
    };
    let latest = detected
        .strategy
        .as_ref()
        .and_then(|strategy| latest_version(driver, strategy, prober));
    DetectionOutcome {
        detected,
        version,
        latest,
        checked_at: crate::lifecycle::now(),
    }
}

/// Shared tail of `detect` once a resolved/real pair exists.
fn classify_resolved(
    driver: Driver,
    resolved: PathBuf,
    real: PathBuf,
    script_prefix: Option<Vec<String>>,
    prober: &dyn Prober,
    overrides: Option<&Overrides>,
) -> Detected {
    let mut detected = Detected {
        installed: true,
        resolved: Some(resolved),
        real: Some(real),
        script_prefix,
        strategy: None,
        disabled_reason: None,
        manual_command: None,
    };
    classify_inner(&mut detected, driver, prober, overrides);
    detected
}

fn classify_inner(
    detected: &mut Detected,
    driver: Driver,
    prober: &dyn Prober,
    overrides: Option<&Overrides>,
) {
    let real = detected.real.clone().expect("real");
    let resolved = detected.resolved.clone().expect("resolved");
    // Original-case slash paths feed extraction and argv; the folded copies are
    // for pattern matching only and never become paths again.
    let real_s = slash_path(&real);
    let resolved_s = slash_path(&resolved);
    let real_low = match_key(&real_s);
    let resolved_low = match_key(&resolved_s);
    let paths = [real_low.as_str(), resolved_low.as_str()];
    let base = detected.clone();
    let manual = |reason: String, command: Option<String>| Detected {
        disabled_reason: Some(reason),
        manual_command: command,
        ..base.clone()
    };
    let managed = |manager: &str, command: Option<String>| {
        manual(
            format!(
                "{} is installed by {manager}. Update it there.",
                driver.name()
            ),
            command,
        )
    };
    let prefix = npm_prefix(&real_s, driver.package()).or_else(|| {
        if !cfg!(windows) {
            return None;
        }
        let dir = resolved.parent()?;
        let manifest = dir
            .join("node_modules")
            .join(driver.package())
            .join("package.json");
        manifest.is_file().then(|| slash_path(dir))
    });
    if let Some(prefix) = prefix
        && !paths.iter().any(|p| {
            is_nix(p)
                || is_mise(p)
                || is_volta(p)
                || is_pnpm(p)
                || is_bun(p)
                || is_vite_plus(p)
                || is_macports(p)
        })
    {
        if let Some(npm) = find_tool("npm", overrides) {
            let argv: Vec<String> = npm
                .iter()
                .cloned()
                .chain(["prefix".into(), "-g".into()])
                .collect();
            let probe = prober.run(&argv, PROBE_TIMEOUT);
            let reported = probe.stdout.lines().next().unwrap_or("").trim();
            let canonical = std::fs::canonicalize(reported)
                .map(|p| match_key(&slash_path(&p)))
                .unwrap_or_else(|_| match_key(&slash_path(Path::new(reported))));
            let expected = std::fs::canonicalize(&prefix)
                .map(|p| match_key(&slash_path(&p)))
                .unwrap_or_else(|_| match_key(&prefix));
            if probe.exit_code == Some(0) && canonical == expected {
                detected.strategy = Some(Strategy::NpmGlobal {
                    prefix: PathBuf::from(&prefix),
                    npm,
                });
                return;
            }
        }
        *detected = managed(
            "npm",
            Some(format!(
                "npm install -g --prefix {prefix} {}@latest",
                driver.package()
            )),
        );
        return;
    }
    let keg = [&real_s, resolved_s.as_str()]
        .iter()
        .find_map(|p| homebrew_keg(p));
    if let Some((prefix, name, cask)) = keg {
        let keg_brew = PathBuf::from(&prefix).join("bin/brew");
        let brew = if keg_brew.is_file() {
            Some(vec![keg_brew.to_string_lossy().into_owned()])
        } else {
            find_tool("brew", overrides)
        };
        if let Some(brew) = brew {
            let argv: Vec<String> = brew.iter().cloned().chain(["--prefix".into()]).collect();
            let probe = prober.run(&argv, PROBE_TIMEOUT);
            let reported = probe.stdout.lines().next().unwrap_or("").trim();
            let canonical = std::fs::canonicalize(reported)
                .map(|p| match_key(&slash_path(&p)))
                .unwrap_or_else(|_| match_key(&slash_path(Path::new(reported))));
            let expected = std::fs::canonicalize(&prefix)
                .map(|p| match_key(&slash_path(&p)))
                .unwrap_or_else(|_| match_key(&prefix));
            if probe.exit_code == Some(0) && canonical == expected {
                detected.strategy = Some(Strategy::Homebrew { brew, name, cask });
                return;
            }
        }
        *detected = managed(
            "Homebrew",
            Some(format!(
                "brew upgrade {}{name}",
                if cask { "--cask " } else { "" }
            )),
        );
        return;
    }
    if driver == Driver::Claude && real_low.contains("/.local/share/claude/versions/") {
        detected.strategy = Some(Strategy::ClaudeNative {
            resolved: resolved.clone(),
        });
        return;
    }
    *detected = manual_reason(
        detected, driver, &paths, &resolved, &real_low, managed, manual,
    );
}

fn manual_reason(
    detected: &Detected,
    driver: Driver,
    paths: &[&str],
    resolved: &Path,
    real_s: &str,
    managed: impl Fn(&str, Option<String>) -> Detected,
    manual: impl Fn(String, Option<String>) -> Detected,
) -> Detected {
    if paths.iter().any(|p| is_nix(p)) {
        return managed("Nix", None);
    }
    if paths.iter().any(|p| is_macports(p)) {
        return managed("MacPorts", None);
    }
    if let Some(manager) = paths.iter().find_map(|p| is_windows_managed(p)) {
        return managed(manager, None);
    }
    if paths.iter().any(|p| is_mise(p)) {
        return managed("mise", Some("mise upgrade".into()));
    }
    if paths.iter().any(|p| is_volta(p)) {
        return managed(
            "Volta",
            Some(format!("volta install {}@latest", driver.package())),
        );
    }
    if paths.iter().any(|p| is_pnpm(p)) {
        return managed(
            "pnpm",
            Some(format!("pnpm add -g {}@latest", driver.package())),
        );
    }
    if paths.iter().any(|p| is_bun(p)) {
        return managed(
            "bun",
            Some(format!("bun install -g {}@latest", driver.package())),
        );
    }
    if paths.iter().any(|p| is_vite_plus(p)) {
        return managed("Vite+", Some(format!("vp i -g {}", driver.package())));
    }
    if driver == Driver::Codex && paths.iter().any(|p| is_codex_desktop_bundle(p)) {
        return manual(
            "Codex is bundled with the Codex app. Update the app.".into(),
            None,
        );
    }
    if real_s.contains("/node_modules/") {
        return manual(
            format!(
                "{} is installed into a project. Update it where it was installed.",
                driver.name()
            ),
            None,
        );
    }
    let resolved_is_link = std::fs::symlink_metadata(resolved)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false);
    if !resolved_is_link && is_wrapper_script(resolved) {
        return manual(
            format!(
                "A wrapper script launches {}. Update it where it was installed.",
                driver.name()
            ),
            None,
        );
    }
    let _ = detected;
    manual(
        format!(
            "{} is installed by an installer Shina cannot prove. Update it where it was installed.",
            driver.name()
        ),
        None,
    )
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// KSG `toProgressLine`: the last meaningful line, collapsed whitespace, 200 chars.
fn progress_line_raw(line: &str) -> Option<String> {
    let text = strip_ansi(line);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then(|| text.chars().take(PROGRESS_MAX_LEN).collect())
}

fn command_output(result: &CommandResult) -> Option<String> {
    let text = [result.stderr_tail.trim(), result.stdout_tail.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty()).then(|| redact(&text))
}

/// KSG `failureMessage`: exit summary plus the last line naming an error.
fn failure_message(result: &CommandResult) -> String {
    let summary = match result.exit_code {
        Some(code) => format!("Update command exited with code {code}"),
        None => "Update command failed".to_owned(),
    };
    let reason = [result.stderr_tail.as_str(), result.stdout_tail.as_str()]
        .into_iter()
        .flat_map(|s| {
            s.rsplit(['\r', '\n'])
                .filter_map(progress_line_raw)
                .collect::<Vec<_>>()
        })
        .find(|line| {
            let lower = line.to_lowercase();
            lower.contains("error") || lower.contains("fail")
        })
        .or_else(|| {
            [result.stderr_tail.as_str(), result.stdout_tail.as_str()]
                .into_iter()
                .find_map(|s| s.rsplit(['\r', '\n']).find_map(progress_line_raw))
        });
    reason.map_or(format!("{summary}."), |r| {
        format!("{summary}: {}", redact(&r))
    })
}

use sha2::Digest;

#[cfg(test)]
#[path = "provider_update_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) mod testkit {
    //! Fakes for `provider_update_tests.rs`.
    use super::*;
    use std::sync::Mutex;

    pub struct FakeProber {
        pub calls: Mutex<Vec<Vec<String>>>,
        pub run: Box<dyn Fn(&[String]) -> ProbeResult + Send + Sync>,
        pub http: Option<String>,
    }
    impl FakeProber {
        pub fn new(run: impl Fn(&[String]) -> ProbeResult + Send + Sync + 'static) -> Self {
            Self {
                calls: Mutex::new(vec![]),
                run: Box::new(run),
                http: None,
            }
        }
    }
    impl Prober for FakeProber {
        fn run(&self, argv: &[String], _timeout: Duration) -> ProbeResult {
            self.calls.lock().unwrap().push(argv.to_vec());
            (self.run)(argv)
        }
        fn http_get(&self, _url: &str, _timeout: Duration) -> Option<String> {
            self.http.clone()
        }
    }

    #[derive(Clone)]
    pub struct QueuedRun {
        pub result: CommandResult,
        pub delay: Duration,
    }
    pub struct FakeRunner {
        pub calls: Mutex<Vec<Vec<String>>>,
        pub runs: Mutex<std::collections::VecDeque<QueuedRun>>,
    }
    impl FakeRunner {
        pub fn script(runs: Vec<QueuedRun>) -> Self {
            Self {
                calls: Mutex::new(vec![]),
                runs: Mutex::new(runs.into()),
            }
        }
    }
    struct FakeRunning {
        result: CommandResult,
        ready: Instant,
    }
    impl RunningUpdate for FakeRunning {
        fn progress(&mut self) -> Option<String> {
            None
        }
        fn result(&mut self) -> Option<CommandResult> {
            (Instant::now() >= self.ready).then(|| self.result.clone())
        }
        fn kill(&mut self) {}
    }
    impl Runner for FakeRunner {
        fn start(&self, argv: &[String]) -> Result<Box<dyn RunningUpdate>, String> {
            self.calls.lock().unwrap().push(argv.to_vec());
            let run = self
                .runs
                .lock()
                .unwrap()
                .pop_front()
                .expect("fake runner script exhausted");
            Ok(Box::new(FakeRunning {
                ready: Instant::now() + run.delay,
                result: run.result,
            }))
        }
    }

    pub struct Fixture {
        pub dir: PathBuf,
    }
    impl Fixture {
        pub fn new() -> Self {
            Self {
                dir: crate::store::directory().expect("test profile"),
            }
        }
        /// A fake npm-global install: `<prefix>/bin/<name>` -> `../lib/node_modules/<pkg>/cli.js`.
        pub fn npm_global(&self, prefix: &str, name: &str, package: &str) -> String {
            let prefix = self.dir.join(prefix);
            let lib = prefix.join("lib/node_modules").join(package);
            std::fs::create_dir_all(&lib).unwrap();
            let entry = lib.join("cli.js");
            std::fs::write(&entry, "// cli\n").unwrap();
            let bin = prefix.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let link = bin.join(if cfg!(windows) {
                format!("{name}.cmd")
            } else {
                name.into()
            });
            #[cfg(unix)]
            std::os::unix::fs::symlink(&entry, &link).unwrap();
            #[cfg(not(unix))]
            std::fs::write(
                &link,
                format!(
                    "@echo off\r\nnode \"%dp0%..\\lib\\node_modules\\{}\\cli.js\" %*\r\n",
                    package.replace('/', "\\")
                ),
            )
            .unwrap();
            link.to_string_lossy().into_owned()
        }
        pub fn native(&self, path: &str) -> String {
            let path = self.dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"\x7fELF fake").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&path).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&path, perms).unwrap();
            }
            path.to_string_lossy().into_owned()
        }
    }

    pub fn service(
        dir: &Path,
        candidates: HashMap<Driver, Vec<String>>,
        prober: FakeProber,
        runner: impl Runner + Send + Sync + 'static,
    ) -> ProviderUpdates {
        // Hermetic: a driver with no override must not fall back to the real
        // PATH, which could find a real CLI on the host.
        let mut candidates = candidates;
        for driver in [Driver::Codex, Driver::Claude] {
            candidates.entry(driver).or_default();
        }
        ProviderUpdates::build(
            dir,
            Hooks::default(),
            Arc::new(prober),
            Arc::new(runner),
            Some(Overrides {
                candidates,
                ..Default::default()
            }),
            Duration::from_secs(30),
        )
    }

    /// Drive `poll()` until nothing is queued/running and no jobs are in flight.
    pub fn settle(service: &mut ProviderUpdates, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            service.poll();
            if !service.active() && service.jobs.is_empty() && service.detect_jobs.is_empty() {
                return;
            }
            assert!(Instant::now() < deadline, "provider update did not settle");
            thread::sleep(Duration::from_millis(5));
        }
    }
}
