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
    sync::Arc,
};

pub use crate::backend::Pi as Session;

/// Native compaction uses the shared `compact` command and compaction_start/end
/// events. ACP has no standard equivalent. Acknowledgement is not completion.
pub fn supports_compaction(id: &str) -> bool {
    matches!(id, "pi" | "codex" | "claude")
}

#[derive(Clone, Debug, Deserialize)]
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

#[derive(Default)]
pub struct Registry {
    pub agents: Vec<Agent>,
    pub error: Option<String>,
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
        let Some(directory) = crate::store::directory() else {
            return Self {
                error: Some("Could not locate harnesses.json.".into()),
                ..Self::default()
            };
        };
        Self::read_in(&directory)
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
        for (id, name, command, args) in [
            (
                "claude-code-acp",
                "Claude Code ACP",
                "claude-code-acp",
                vec![],
            ),
            ("codex-acp", "Codex ACP", "codex-acp", vec![]),
            (
                "gemini",
                "Gemini ACP",
                "gemini",
                vec!["--experimental-acp".into()],
            ),
            ("opencode", "OpenCode ACP", "opencode", vec!["acp".into()]),
        ] {
            if !registry.agents.iter().any(|a| a.id == id) && executable(command).is_ok() {
                registry.agents.push(Agent {
                    id: id.into(),
                    name: name.into(),
                    command: command.into(),
                    args,
                    env: vec![],
                });
            }
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

/// No shell is used. Resolve npm Windows shims to their JavaScript entry points.
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
    pub link: Option<HostLink>,
    pub owner: Arc<SessionOwner>,
    pub context_sent: bool,
}

pub fn start(host: BackendHost, start: Start, wake: Waker) -> Result<Session, String> {
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
        let agent = Registry::read().find(&start.id)?;
        crate::acp::start(agent, start, wake)
    }
}

fn pi_start(mut start: Start) -> Start {
    start.instructions = Some(format!(
        "{}\n\n{}",
        crate::mcp::INSTRUCTIONS,
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
                link: None,
                owner: Arc::default(),
                context_sent: false,
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
}
