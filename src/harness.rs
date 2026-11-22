//! Harness selection and the small Pi-shaped command contract shared by both transports.
//! ACP is adapted at the transport boundary, not in the transcript reducer.
use crate::backend::{BackendHost, HostLink, Pi, SessionOwner, SessionSource};
use rune::Waker;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

pub use crate::backend::Pi as Session;

/// Native compaction uses the shared `compact` command and compaction_start/end
/// events. ACP has no standard equivalent. Acknowledgement is not completion.
pub fn supports_compaction(id: &str) -> bool {
    matches!(id, "pi" | "codex" | "claude")
}

/// Providers for which KSG Code declares `freshContext` support (server-wide in KSG's
/// `ServerEnvironment`): Pi (`SessionSource::New`), Codex (`thread/start`), Claude
/// (`--session-id`), and ACP (`session/new`).
pub fn supports_fresh_context(id: &str) -> bool {
    matches!(id, "pi" | "codex" | "claude") || valid_id(id)
}

/// Prompt suggestions are obtained only for providers that expose a native prediction signal
/// or sampling call in KSG Code (`claude` and `codex`).
pub fn supports_prompt_suggestions(id: &str) -> bool {
    matches!(id, "claude" | "codex")
}

impl Start {
    /// Discovery can open a temporary session for catalog data, but never registers MCP
    /// or sends a prompt.
    pub(crate) fn discovery(id: &str, workspace: PathBuf) -> Self {
        Self {
            id: id.into(),
            workspace,
            source: SessionSource::New,
            instructions: None,
            shina: crate::mcp::INSTRUCTIONS,
            link: None,
            owner: Arc::default(),
            context_sent: false,
            prompt_suggestions: false,
            metadata_only: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Names only. Values are inherited at launch and never written to Shina storage.
    #[serde(default)]
    pub env: Vec<String>,
}

#[derive(Default, PartialEq, Eq)]
pub struct Registry {
    pub agents: Vec<Agent>,
    pub error: Option<String>,
}

#[derive(Default, PartialEq, Eq)]
pub(crate) struct Availability {
    pub claude: bool,
    pub antigravity: bool,
    pub cursor: bool,
}

#[derive(Default)]
pub(crate) struct AvailabilityProbe {
    pub last: Option<Availability>,
    pending: Option<mpsc::Receiver<(Registry, Availability)>>,
    started: Option<Instant>,
}

impl AvailabilityProbe {
    pub(crate) fn loading(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn refresh(&mut self, wake: Waker, force: bool) {
        // Capture the profile before crossing threads. Tests keep an isolated profile
        // per test thread; the worker must read the launching profile.
        let directory = crate::store::directory();
        self.refresh_with(Instant::now(), wake, force, move || {
            let registry = Registry::read_at(directory.as_deref());
            let availability = Availability {
                claude: crate::claude::process().is_ok(),
                antigravity: crate::antigravity::cli_available(),
                cursor: crate::cursor::cli_available(),
            };
            (registry, availability)
        });
    }

    fn refresh_with(
        &mut self,
        now: Instant,
        wake: Waker,
        force: bool,
        probe: impl FnOnce() -> (Registry, Availability) + Send + 'static,
    ) {
        if self.pending.is_some()
            || (!force
                && self.started.is_some_and(|at| {
                    now.saturating_duration_since(at) < Duration::from_secs(5 * 60)
                }))
        {
            return;
        }
        let (send, receive) = mpsc::channel();
        self.pending = Some(receive);
        self.started = Some(now);
        std::thread::spawn(move || {
            let _ = send.send(probe());
            wake();
        });
    }

    pub(crate) fn poll(&mut self, registry: &mut Registry) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        match pending.try_recv() {
            Ok((next_registry, next)) => {
                self.pending = None;
                let changed = self.last.as_ref() != Some(&next) || *registry != next_registry;
                if changed {
                    *registry = next_registry;
                    self.last = Some(next);
                }
                changed
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                false
            }
        }
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn availability_probes_coalesce_throttle_and_publish_only_changes() {
        let now = Instant::now();
        let mut probe = AvailabilityProbe::default();
        let mut registry = Registry::default();
        let (release, wait) = mpsc::channel();
        let (done, wakeups) = mpsc::channel();
        let wake: Waker = Arc::new(move || done.send(()).unwrap());
        let attempts = Arc::new(AtomicUsize::new(0));
        let caller = std::thread::current().id();
        let first = attempts.clone();
        probe.refresh_with(now, wake.clone(), false, move || {
            assert_ne!(std::thread::current().id(), caller);
            first.fetch_add(1, Ordering::Relaxed);
            wait.recv().unwrap();
            (
                Registry::default(),
                Availability {
                    claude: true,
                    ..Default::default()
                },
            )
        });
        for _ in 0..16 {
            probe.refresh_with(now, wake.clone(), false, || {
                unreachable!("probe still running")
            });
            probe.refresh_with(now, wake.clone(), true, || {
                unreachable!("probe still running")
            });
            assert!(!probe.poll(&mut registry));
        }
        release.send(()).unwrap();
        wakeups.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(probe.poll(&mut registry));
        assert!(probe.last.as_ref().unwrap().claude);
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        probe.refresh_with(now, wake.clone(), false, || unreachable!("warm cache"));
        assert!(!probe.loading());
        probe.refresh_with(now, wake.clone(), true, || {
            (
                Registry::default(),
                Availability {
                    claude: true,
                    ..Default::default()
                },
            )
        });
        wakeups.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            !probe.poll(&mut registry),
            "identical results do not cause a redraw"
        );
        probe.refresh_with(now + Duration::from_secs(5 * 60), wake, false, || {
            (
                Registry {
                    error: Some("fixture registry error".into()),
                    ..Default::default()
                },
                Availability {
                    cursor: true,
                    ..Default::default()
                },
            )
        });
        wakeups.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(probe.poll(&mut registry));
        assert!(probe.last.as_ref().unwrap().cursor);
        assert!(!probe.last.as_ref().unwrap().claude);
        assert_eq!(registry.error.as_deref(), Some("fixture registry error"));
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.bytes().enumerate().all(|(i, c)| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || (i > 0 && b"._-".contains(&c))
        })
}

impl Registry {
    pub fn read() -> Self {
        Self::read_at(crate::store::directory().as_deref())
    }

    fn read_at(directory: Option<&Path>) -> Self {
        let Some(directory) = directory else {
            return Self {
                error: Some("Could not locate harnesses.json.".into()),
                ..Self::default()
            };
        };
        Self::read_in(directory)
    }

    fn read_in(directory: &Path) -> Self {
        let mut registry = Self::default();
        match std::fs::read(directory.join("harnesses.json")) {
            Ok(bytes) => match serde_json::from_slice::<Vec<Agent>>(&bytes) {
                Ok(agents) => {
                    let mut ids = HashSet::new();
                    for agent in agents {
                        if !valid_id(&agent.id)
                            || matches!(agent.id.as_str(), "pi" | "codex" | "claude")
                            || !ids.insert(agent.id.clone())
                            || agent.command.trim().is_empty()
                            || agent.name.trim().is_empty()
                            || agent
                                .env
                                .iter()
                                .any(|name| name.is_empty() || name.contains(['=', '\0']))
                        {
                            registry.error = Some("Invalid harnesses.json. Use unique lowercase ACP ids, nonempty names and commands, and environment variable names only.".into());
                            registry.agents.clear();
                            return registry;
                        }
                        registry.agents.push(agent);
                    }
                }
                Err(error) => {
                    registry.error = Some(format!("Could not read harnesses.json: {error}"));
                    return registry;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                registry.error = Some(format!("Could not read harnesses.json: {error}"));
                return registry;
            }
        }
        // Like KSG's ACP registry, discovery never installs or authenticates an agent.
        for (id, name, command, args, env) in [
            (
                "claude-code-acp",
                "Claude Code ACP",
                "claude-code-acp",
                vec![],
                vec![],
            ),
            ("codex-acp", "Codex ACP", "codex-acp", vec![], vec![]),
            (
                "gemini",
                "Gemini ACP",
                "gemini",
                vec!["--experimental-acp".into()],
                // Gemini CLI reads its key and base-url overrides from the
                // environment; pass them through by name only.
                vec![
                    "GEMINI_API_KEY".into(),
                    "GOOGLE_API_KEY".into(),
                    "GOOGLE_GEMINI_BASE_URL".into(),
                ],
            ),
            (
                "opencode",
                "OpenCode ACP",
                "opencode",
                vec!["acp".into()],
                vec![],
            ),
        ] {
            if !registry.agents.iter().any(|a| a.id == id) && executable(command).is_ok() {
                registry.agents.push(Agent {
                    id: id.into(),
                    name: name.into(),
                    command: command.into(),
                    args,
                    env,
                });
            }
        }
        // KSG's Devin ACP Registry entry: `devin acp` from PATH or the T3-managed
        // install cache, with its credential environment passed through by name.
        if !registry.agents.iter().any(|a| a.id == "devin")
            && let Some(command) = crate::devin::discover_command()
        {
            registry.agents.push(crate::devin::agent(command));
        }
        registry
    }

    pub fn find(&self, id: &str) -> Result<Agent, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        self.agents.iter().find(|agent| agent.id == id).cloned()
            .ok_or_else(|| format!("ACP harness {id} is unavailable. Configure it in harnesses.json or put its executable on PATH."))
    }
}

/// Blocking filesystem lookup. Resolve npm Windows shims without running a shell.
/// Background discovery publishes availability separately from launch-time resolution.
pub(crate) fn executable(command: &str) -> Result<(PathBuf, Option<PathBuf>), String> {
    let path = Path::new(command);
    let candidates = if path.is_absolute() || path.components().count() > 1 {
        vec![path.to_path_buf()]
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .flat_map(|dir| {
                let path = dir.join(command);
                if cfg!(windows) && path.extension().is_none() {
                    vec![
                        path.with_extension("exe"),
                        path.with_extension("com"),
                        path.with_extension("cmd"),
                    ]
                } else {
                    vec![path]
                }
            })
            .collect()
    };
    for path in candidates.into_iter().filter(|path| path.is_file()) {
        if path.extension().is_some_and(|ext| ext == "cmd") {
            if let Ok(shim) = std::fs::read_to_string(&path) {
                for token in shim.split('"') {
                    if let Some(relative) = token.strip_prefix("%dp0%")
                        && relative.ends_with(".js")
                    {
                        let entry = path
                            .parent()
                            .unwrap()
                            .join(relative.trim_start_matches(['\\', '/']));
                        if entry.is_file() {
                            return Ok((PathBuf::from("node"), Some(entry)));
                        }
                    }
                }
            }
            continue;
        }
        if path
            .extension()
            .is_some_and(|ext| ext == "js" || ext == "mjs")
        {
            return Ok((PathBuf::from("node"), Some(path)));
        }
        if !path
            .extension()
            .is_some_and(|ext| ext == "ps1" || ext == "bat")
        {
            return Ok((path, None));
        }
    }
    Err(format!(
        "ACP executable {command} was not found. Use a native executable or a JavaScript entry point, not a shell command."
    ))
}

impl Agent {
    pub fn process(&self) -> Result<Command, String> {
        let (executable, script) = executable(&self.command)?;
        let mut command = Command::new(executable);
        if let Some(script) = script {
            command.arg(script);
        }
        command.args(&self.args).env_clear();
        // Keep ordinary OS/provider config discovery, but pass secrets only by explicit name.
        for name in [
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
        ]
        .into_iter()
        .chain(self.env.iter().map(String::as_str))
        {
            if name.starts_with("SHINA_") {
                continue;
            }
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        Ok(command)
    }
}

pub struct Start {
    pub id: String,
    pub workspace: PathBuf,
    pub source: SessionSource,
    pub instructions: Option<String>,
    /// The role-matched Shina awareness text from the caller's effective record.
    pub shina: &'static str,
    pub link: Option<HostLink>,
    pub owner: Arc<SessionOwner>,
    pub context_sent: bool,
    pub prompt_suggestions: bool,
    pub(crate) metadata_only: bool,
}

pub fn start(
    host: BackendHost,
    start: Start,
    wake: Waker,
    models: &[crate::session::AgentModel],
) -> Result<Session, String> {
    if start.id == "pi" {
        // All real Pi paths, including forks and resumes, pass through this boundary.
        let start = pi_start(start);
        Ok(host.start(
            start.workspace,
            start.source,
            wake,
            start.instructions,
            start.link,
            start.owner,
        ))
    } else {
        if start.id == "codex" {
            return crate::codex::start(start, wake);
        }
        if start.id == "claude" {
            return crate::claude::start(start, wake);
        }
        // Antigravity is built in when `agy` is on PATH unless harnesses.json overrides it.
        if start.id == "antigravity" {
            let registry = Registry::read();
            if registry.agents.iter().any(|a| a.id == "antigravity") {
                return crate::acp::start_with_catalog(
                    registry.find("antigravity")?,
                    start,
                    wake,
                    models,
                );
            }
            return crate::antigravity::start(start, wake);
        }
        // Cursor is built in when `cursor-agent`/`agent` resolves, with the same override.
        if start.id == "cursor" {
            let registry = Registry::read();
            if registry.agents.iter().any(|a| a.id == "cursor") {
                return crate::acp::start_with_catalog(
                    registry.find("cursor")?,
                    start,
                    wake,
                    models,
                );
            }
            return crate::cursor::start(start, wake, models);
        }
        let agent = Registry::read().find(&start.id)?;
        crate::acp::start_with_catalog(agent, start, wake, models)
    }
}

fn pi_start(mut start: Start) -> Start {
    start.instructions = Some(format!(
        "{}\n\n{}",
        start.shina,
        start.instructions.as_deref().unwrap_or("")
    ));
    start
}

/// Prompt, steer-or-queue, cancel, respond, load-session and set-model all retain the
/// existing typed-by-`type` command envelope. ACP translates it to JSON-RPC.
pub fn send(session: &Pi, record: Value) -> Result<(), String> {
    session.send(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_environment_has_names_only_and_never_inherits_shina_bridge_credentials() {
        let agent = Agent {
            id: "fixture".into(),
            name: "Fixture".into(),
            command: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args: vec![],
            env: vec!["SHINA_MCP_TOKEN".into()],
        };
        let command = agent.process().unwrap();
        assert!(
            !command
                .get_envs()
                .any(|(name, _)| name.to_string_lossy().starts_with("SHINA_"))
        );
        assert_eq!(
            command.get_program(),
            std::env::current_exe().unwrap().as_os_str()
        );
    }

    #[test]
    fn npm_shims_resolve_without_invoking_a_shell() {
        let directory = crate::store::directory().unwrap();
        let script = directory.join("agent.js");
        std::fs::write(&script, "// fixture only").unwrap();
        let shim = directory.join("agent.cmd");
        std::fs::write(&shim, r#"@"%dp0%\node.exe" "%dp0%\agent.js" %*"#).unwrap();
        let (command, entry) = executable(&shim.to_string_lossy()).unwrap();
        assert_eq!(command, PathBuf::from("node"));
        assert_eq!(entry, Some(script));
    }

    #[test]
    fn every_pi_start_path_includes_shina_awareness_and_its_persona() {
        for (path, source, persona) in [
            ("lone", SessionSource::New, None),
            (
                "captain home",
                SessionSource::New,
                Some("Captain's standing instructions"),
            ),
            ("delegated", SessionSource::New, Some("Crew instructions")),
            (
                "forked",
                SessionSource::Fork("original.jsonl".into()),
                Some("Inherited captain"),
            ),
            ("resumed", SessionSource::Resume("saved.jsonl".into()), None),
        ] {
            let start = pi_start(Start {
                id: "pi".into(),
                workspace: PathBuf::new(),
                source,
                instructions: persona.map(str::to_owned),
                shina: crate::mcp::INSTRUCTIONS,
                link: None,
                owner: Arc::default(),
                context_sent: false,
                prompt_suggestions: false,
                metadata_only: false,
            });
            let instructions = start.instructions.as_deref().unwrap();
            assert!(instructions.contains(crate::mcp::INSTRUCTIONS), "{path}");
            if let Some(persona) = persona {
                assert!(instructions.contains(persona), "{path}");
            }
        }
    }

    #[test]
    fn registry_defaults_and_validation_do_not_write_configuration() {
        let dir = crate::store::directory().unwrap();
        let registry = Registry::read_in(&dir);
        assert!(registry.error.is_none());
        assert!(!dir.join("harnesses.json").exists());
        for invalid in [
            r#"[{"id":"pi","name":"Other","command":"agent"}]"#,
            r#"[{"id":"claude","name":"Other","command":"agent"}]"#,
            r#"[{"id":"bad id","name":"Other","command":"agent"}]"#,
            r#"[{"id":"ok","name":"Other","command":"agent","env":{"TOKEN":"secret"}}]"#,
        ] {
            std::fs::write(dir.join("harnesses.json"), invalid).unwrap();
            assert!(Registry::read_in(&dir).error.is_some());
        }
        std::fs::write(dir.join("harnesses.json"), r#"[{"id":"fixture","name":"Fixture","command":"node","args":["agent.js"],"env":["FIXTURE_TOKEN"]}]"#).unwrap();
        let agent = Registry::read_in(&dir).find("fixture").unwrap();
        assert_eq!(agent.env, ["FIXTURE_TOKEN"]);
    }

    #[test]
    fn a_devin_registry_entry_is_honored_with_credential_names() {
        let dir = crate::store::directory().unwrap();
        std::fs::write(
            dir.join("harnesses.json"),
            r#"[{"id":"devin","name":"Devin","command":"devin","args":["acp"],"env":["DEVIN_TOKEN","DEVIN_API_KEY"]}]"#,
        )
        .unwrap();
        let agent = Registry::read_in(&dir).find("devin").unwrap();
        assert_eq!(agent.args, ["acp"]);
        assert_eq!(agent.env, ["DEVIN_TOKEN", "DEVIN_API_KEY"]);
    }
}
