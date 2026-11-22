//! Cursor `agent acp` provider layer. Ported from KSG Code's
//! `packages/provider-cursor/src/server` (driver.ts, adapter.ts,
//! CursorAgentSdk.ts, CursorSdk.ts, CursorSdkCatalog.ts, apiKey.ts).
//!
//! KSG's V2 driver wraps the bundled `@cursor/sdk` in-process runtime. Shina
//! drives the documented `agent acp` transport the Cursor CLI exposes for
//! custom ACP clients through the shared ACP session layer, and discovers
//! models without a conversation: the same public `api.cursor.com/v1/models`
//! route KSG probes when `CURSOR_API_KEY` is set, otherwise the CLI's own
//! `agent models` list, which honours `agent login` credentials.
use crate::{
    backend::{Event, Pi},
    descriptors::{
        ModelCapabilities, ProviderOptionChoice, ProviderOptionDescriptor, ProviderOptionRole,
    },
    harness::{Agent, Start},
};
use rune::Waker;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::Duration,
};

/// KSG `cursorSettings.binaryPath` plus the PATH fallbacks the CLI installs to.
pub const MISSING: &str = "Cursor Agent is unavailable. Shina checks SHINA_CURSOR_CLI, CURSOR_CLI and PATH for `cursor-agent` or `agent`; install Cursor Agent or set SHINA_CURSOR_CLI to its executable.";

/// KSG `cursorSettings.binaryPath` is `SHINA_CURSOR_CLI`/`CURSOR_CLI` here; the
/// values win over PATH in order.
const CLI_ENV: [&str; 2] = ["SHINA_CURSOR_CLI", "CURSOR_CLI"];
/// `cursor-agent` is the documented binary; `agent` is the alias its installer
/// registers on PATH and in `%LOCALAPPDATA%\cursor-agent`.
const CLI_NAMES: &[&str] = &["cursor-agent", "agent"];
/// KSG `CURSOR_BACKEND_URL` default (`CursorSdkCatalog.ts`).
const DEFAULT_BACKEND: &str = "https://api.cursor.com";
/// KSG `CURSOR_SDK_CATALOG_TIMEOUT_MS`; the catalog poll budget is 20s.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(12);
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const PROBE_OUTPUT_LIMIT: usize = 1 << 20;
/// KSG `COMPACT_SLASH_COMMAND`; Cursor's CLI accepts `/compact` as a prompt.
const COMPACT_COMMAND: &str = "compact";
const COMPACT_DESCRIPTION: &str = "Summarize the conversation and reduce context usage";

fn candidates() -> Vec<PathBuf> {
    candidates_from(
        CLI_ENV.map(|name| std::env::var_os(name)),
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    )
}

/// Override paths first, then PATH names, then the directories the installer
/// writes (`%LOCALAPPDATA%\cursor-agent`, `~/.local/bin`).
fn candidates_from(
    overrides: [Option<OsString>; 2],
    home: Option<PathBuf>,
    local: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = overrides.into_iter().flatten().map(PathBuf::from).collect();
    candidates.extend(CLI_NAMES.iter().map(PathBuf::from));
    if cfg!(windows) {
        if let Some(local) =
            local.or_else(|| home.as_ref().map(|home| home.join("AppData").join("Local")))
        {
            for name in CLI_NAMES {
                candidates.push(local.join("cursor-agent").join(format!("{name}.exe")));
            }
        }
    } else if let Some(home) = home {
        for name in CLI_NAMES {
            candidates.push(home.join(".local").join("bin").join(name));
        }
    }
    candidates
}

/// The resolved CLI command with no subcommand arguments. Cursor's own
/// environment (`CURSOR_API_KEY`, `OPENAI_BASE_URL` and CPA routing variables)
/// passes through; only another Shina thread's bridge credentials are removed.
pub fn process() -> Result<Command, String> {
    resolve_process(candidates())
}

fn resolve_process(candidates: Vec<PathBuf>) -> Result<Command, String> {
    const ALLOWED_ENV: &[&str] = &[
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
        "CURSOR_API_KEY",
        "CURSOR_BACKEND_URL",
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "NO_PROXY",
        "SSL_CERT_FILE",
        "NODE_EXTRA_CA_CERTS",
    ];
    for path in candidates {
        if let Ok((executable, script)) = crate::harness::executable(&path.to_string_lossy()) {
            let mut command = Command::new(executable);
            if let Some(script) = script {
                command.arg(script);
            }
            command.env_clear();
            for name in ALLOWED_ENV {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
            return Ok(command);
        }
    }
    Err(MISSING.into())
}

/// `agent acp`, the stdio ACP server the Cursor CLI runs for custom clients.
/// `acp::run` sets the working directory and pipes.
pub fn command(workspace: &Path) -> Result<Command, String> {
    let mut command = process()?;
    command.arg("acp").current_dir(workspace);
    Ok(command)
}

/// KSG `fromExternal` PATH probe. Tests stay hermetic: the host's installed CLI
/// must not change picker snapshots, so the probe is off under cfg(test).
pub fn cli_available() -> bool {
    #[cfg(test)]
    return false;
    #[cfg(not(test))]
    process().is_ok()
}

/// Session start: the resolved `agent acp` command on the shared ACP runtime.
pub fn start(start: Start, wake: Waker) -> Result<Pi, String> {
    let agent = Agent {
        id: "cursor".into(),
        name: "Cursor".into(),
        command: "cursor-agent".into(),
        args: vec![],
        env: vec![],
    };
    let command = command(&start.workspace)?;
    crate::acp::start_command(agent, start, wake, command)
}

/// KSG `checkCursorProviderStatus`: a metadata-only catalog probe. No
/// conversation, MCP registration or prompt, like `Start::discovery` requires.
pub fn discover(workspace: PathBuf, wake: Waker) -> Pi {
    let start = Start::discovery("cursor", workspace);
    let owner = start.owner.clone();
    let pi = Pi::worker(
        wake,
        owner,
        None,
        move |input, events, _wake, stopped, _owner| {
            let command = process()?;
            if stopped.load(Ordering::Acquire) {
                return Ok(());
            }
            if let Some(version) = probe_version(&command) {
                let _ = events.send(Event::Record(
                    json!({"type":"harness_update","field":"version","value":version}),
                ));
            }
            let mut models = None;
            for record in input {
                if stopped.load(Ordering::Relaxed) {
                    break;
                }
                match record["type"].as_str() {
                    Some("get_available_models") => {
                        let result = models.get_or_insert_with(|| fetch_models(&command, stopped));
                        respond(
                            events,
                            "get_available_models",
                            result
                                .as_ref()
                                .map(|models| json!({"models": models}))
                                .map_err(Clone::clone),
                        );
                    }
                    Some("get_commands") => {
                        respond(events, "get_commands", Ok(json!({"commands": commands()})))
                    }
                    _ => {}
                }
            }
            Ok(())
        },
    );
    let _ = pi.send(json!({"type":"get_available_models"}));
    let _ = pi.send(json!({"type":"get_commands"}));
    pi
}

fn commands() -> Vec<Value> {
    vec![json!({
        "name": COMPACT_COMMAND,
        "description": COMPACT_DESCRIPTION,
        "source": "cursor",
    })]
}

fn respond(events: &SyncSender<Event>, command: &str, result: Result<Value, String>) {
    let record = match result {
        Ok(data) => json!({"type":"response","command":command,"success":true,"data":data}),
        Err(error) => {
            json!({"type":"response","command":command,"success":false,"error":error})
        }
    };
    let _ = events.send(Event::Record(record));
}

/// KSG orders the API-key catalog first and only then the agent runtime: the
/// HTTP route is the exact schema, `agent models` covers CLI-only sign-ins.
fn fetch_models(command: &Command, stopped: &AtomicBool) -> Result<Vec<Value>, String> {
    let key = std::env::var("CURSOR_API_KEY")
        .ok()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());
    let mut errors = vec![];
    if let Some(key) = &key {
        match models_from_api(key) {
            Ok(models) if !models.is_empty() => return Ok(models),
            Ok(_) => errors.push("Cursor model discovery returned no models.".into()),
            Err(error) => errors.push(error),
        }
    }
    if !stopped.load(Ordering::Relaxed) {
        match models_from_cli(command) {
            Ok(models) if !models.is_empty() => return Ok(models),
            Ok(_) => errors.push("`agent models` returned no models.".into()),
            Err(error) => errors.push(error),
        }
    }
    if key.is_none() {
        errors.push("Sign in with `agent login` or set CURSOR_API_KEY, then retry.".into());
    }
    Err(errors.join(" "))
}

/// The `GET {CURSOR_BACKEND_URL|api.cursor.com}/v1/models` probe KSG's
/// `CursorSdkCatalog` runs, bearer-key only and never a conversation.
fn models_from_api(key: &str) -> Result<Vec<Value>, String> {
    let url = format!("{}/v1/models", backend_url());
    let client = reqwest::blocking::Client::builder()
        .timeout(CATALOG_TIMEOUT)
        .build()
        .map_err(|error| format!("Cursor model catalog request failed: {error}"))?;
    let response = client
        .get(&url)
        .bearer_auth(key)
        .send()
        .map_err(|error| format!("Cursor model catalog request failed: {error}"))?;
    let status = response.status();
    let body: Value = response.json().unwrap_or_default();
    if !status.is_success() {
        let error = &body["error"];
        let message = error["message"]
            .as_str()
            .or_else(|| body["message"].as_str())
            .map(|text| format!(" {text}"))
            .unwrap_or_default();
        return Err(match status.as_u16() {
            401 => "Cursor authentication failed. Check CURSOR_API_KEY.".into(),
            403 if error["code"] == "plan_required" || body["code"] == "plan_required" => {
                "Cursor needs a paid plan to run in Shina. Upgrade the account that owns CURSOR_API_KEY or use a key from a paid account.".into()
            }
            _ => format!("Cursor /v1/models returned HTTP {status}.{message}"),
        });
    }
    Ok(catalog_models(&body))
}

/// `agent models` prints one model id per line (bullets and a header in some
/// versions); a JSON payload parses through the same catalog shape.
fn models_from_cli(command: &Command) -> Result<Vec<Value>, String> {
    let mut probe = probe_command(command, "models");
    let output = crate::process::output(&mut probe, CATALOG_TIMEOUT, PROBE_OUTPUT_LIMIT)
        .map_err(|error| format!("`agent models` failed: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr.lines().next().unwrap_or("").trim();
        return Err(format!(
            "`agent models` exited with {}.{tail}",
            output.status
        ));
    }
    Ok(list_models(&String::from_utf8_lossy(&output.stdout)))
}

fn probe_version(command: &Command) -> Option<String> {
    let mut probe = probe_command(command, "--version");
    let output = crate::process::output(&mut probe, PROBE_TIMEOUT, 4096).ok()?;
    output
        .status
        .success()
        .then(|| cli_version(&String::from_utf8_lossy(&output.stdout)))?
}

/// A short probe on the resolved CLI: program, npm-shim script and the explicit
/// env, plus the probe's argument and a null stdin.
fn probe_command(command: &Command, arg: &str) -> Command {
    let mut probe = Command::new(command.get_program());
    for arg in command.get_args() {
        probe.arg(arg);
    }
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => {
                probe.env(name, value);
            }
            None => {
                probe.env_remove(name);
            }
        }
    }
    probe.arg(arg).stdin(Stdio::null());
    probe
}

/// First version-looking token (`2026.05.09` style) in `--version` output.
fn cli_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|token| {
            token.chars().next().is_some_and(|c| c.is_ascii_digit()) && token.contains('.')
        })
        .map(|token| {
            token
                .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-')
                .to_owned()
        })
}

fn backend_url() -> String {
    std::env::var("CURSOR_BACKEND_URL")
        .ok()
        .map(|url| url.trim_end_matches('/').to_owned())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| DEFAULT_BACKEND.into())
}

/// KSG `buildCursorDiscoveredModelsFromSdk`: trimmed id/displayName rows,
/// deduped, with `aliases` and parameter-driven option descriptors. The
/// payload accepts `items` (the documented shape) or a bare/`models`/`data`
/// array so `agent models` JSON output parses through the same path. Rows are
/// wire values, the same shape `session/new` model lists arrive in.
fn catalog_models(payload: &Value) -> Vec<Value> {
    let items = ["items", "models", "data"]
        .iter()
        .find_map(|key| payload[key].as_array())
        .or_else(|| payload.as_array())
        .cloned()
        .unwrap_or_default();
    let mut seen = HashSet::new();
    items
        .iter()
        .filter_map(|model| {
            let id = model["id"].as_str()?.trim();
            let name = model["displayName"]
                .as_str()
                .or_else(|| model["name"].as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())?;
            if id.is_empty() || !seen.insert(id.to_owned()) {
                return None;
            }
            let capabilities = catalog_capabilities(model);
            let thinking_levels =
                crate::descriptors::model_reasoning_levels(&capabilities.option_descriptors);
            Some(json!({
                "provider": "cursor",
                "id": id,
                "name": name,
                "aliases": model["aliases"].as_array().into_iter().flatten()
                    .filter_map(|alias| alias.as_str().map(str::trim)
                        .filter(|alias| !alias.is_empty()).map(str::to_owned))
                    .collect::<Vec<_>>(),
                "thinkingLevels": thinking_levels,
                "capabilities": capabilities,
            }))
        })
        .collect()
}

/// `agent models` text output: drop headers and bullets, keep the leading id
/// token of each remaining line.
fn list_models(text: &str) -> Vec<Value> {
    let trimmed = text.trim();
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && let Ok(payload) = serde_json::from_str::<Value>(trimmed)
    {
        return catalog_models(&payload);
    }
    const HEADERS: &[&str] = &[
        "available",
        "models",
        "model",
        "id",
        "name",
        "cursor",
        "agent",
        "installed",
        "using",
    ];
    let mut seen = HashSet::new();
    trimmed
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line
                .strip_prefix(['-', '*', '•'])
                .filter(|tail| tail.starts_with(char::is_whitespace))
                .or_else(|| {
                    let tail = line.trim_start_matches(|c: char| c.is_ascii_digit());
                    (tail.len() < line.len())
                        .then_some(tail)?
                        .strip_prefix(['.', ')'])
                        .filter(|tail| tail.starts_with(char::is_whitespace))
                })
                .unwrap_or(line)
                .trim_start();
            let id = line
                .split(|c: char| c.is_whitespace() || c == '(' || c == '[' || c == ':')
                .next()?
                .trim();
            if id.is_empty()
                || !id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
                || HEADERS.contains(&id.to_ascii_lowercase().as_str())
                || line.ends_with(':')
            {
                return None;
            }
            if !seen.insert(id.to_owned()) {
                return None;
            }
            Some(json!({"provider":"cursor","id":id,"name":id}))
        })
        .collect()
}

/// KSG `buildCursorCapabilitiesFromSdkModel`: catalog `parameters` become
/// option descriptors sorted by `cursorSdkParameterPriority`, ids pass through
/// `cursorSdkProviderOptionId`, true/false pairs collapse to booleans and the
/// default `variant.params` value marks the current/default choice.
fn catalog_capabilities(model: &Value) -> ModelCapabilities {
    let mut parameters: Vec<(usize, &Value)> = model["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .collect();
    parameters.sort_by_key(|(index, parameter)| {
        (
            parameter_priority(parameter["id"].as_str().unwrap_or_default()),
            *index,
        )
    });
    let mut seen = HashSet::new();
    let mut descriptors = vec![];
    for (_, parameter) in parameters {
        let native = parameter["id"].as_str().unwrap_or_default().trim();
        let id = provider_option_id(native);
        if native.is_empty() || id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let values: Vec<(String, String)> = parameter["values"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let value = entry["value"].as_str()?.trim();
                (!value.is_empty()).then(|| {
                    (
                        value.to_owned(),
                        entry["displayName"]
                            .as_str()
                            .map(str::trim)
                            .filter(|name| !name.is_empty())
                            .unwrap_or(value)
                            .to_owned(),
                    )
                })
            })
            .collect();
        if values.is_empty() {
            continue;
        }
        let label = parameter["displayName"]
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| title_case(&id));
        let default = default_parameter(model, native);
        let normalized: HashSet<String> = values
            .iter()
            .map(|(value, _)| value.to_lowercase())
            .collect();
        if values.len() == 2 && normalized.contains("true") && normalized.contains("false") {
            descriptors.push(ProviderOptionDescriptor::Boolean {
                id,
                label,
                description: None,
                current_value: match default.as_deref() {
                    Some("true") => Some(true),
                    Some("false") => Some(false),
                    _ => None,
                },
            });
            continue;
        }
        descriptors.push(ProviderOptionDescriptor::Select {
            id,
            label,
            description: None,
            options: values
                .into_iter()
                .map(|(value, label)| ProviderOptionChoice {
                    is_default: default.as_deref() == Some(value.as_str()),
                    id: value,
                    label,
                    description: None,
                })
                .collect(),
            current_value: None,
            prompt_injected_values: vec![],
            role: matches!(native, "effort" | "reasoning" | "thinking")
                .then_some(ProviderOptionRole::Reasoning),
        });
    }
    ModelCapabilities {
        option_descriptors: descriptors,
    }
}

/// KSG `CURSOR_SDK_PARAMETER_TO_PROVIDER_OPTION`.
fn provider_option_id(native: &str) -> String {
    match native {
        "context" => "contextWindow",
        "fast" => "fastMode",
        _ => native,
    }
    .into()
}

/// KSG `cursorSdkParameterPriority`: reasoning first, then context, fast and
/// thinking; everything else keeps its catalog order.
fn parameter_priority(native: &str) -> usize {
    match native {
        "effort" | "reasoning" => 0,
        "context" => 1,
        "fast" => 2,
        "thinking" => 3,
        _ => 4,
    }
}

/// KSG `cursorSdkDefaultParameterValue`: the default variant's value for `native`.
fn default_parameter(model: &Value, native: &str) -> Option<String> {
    model["variants"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|variant| variant["isDefault"] == true)?["params"]
        .as_array()?
        .iter()
        .find(|parameter| parameter["id"] == native)?["value"]
        .as_str()
        .map(str::to_owned)
}

/// KSG `toTitleCaseWords`.
fn title_case(id: &str) -> String {
    id.split(|ch: char| ch.is_whitespace() || matches!(ch, '_' | '-'))
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str().to_lowercase()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[path = "cursor_tests.rs"]
mod cursor_tests;
