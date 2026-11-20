//! Provider launch overrides (`providers.json`) and CLIPROXYAPI-style gateway
//! configuration, mirroring KSG Code's provider-instance environment.
//!
//! `providers.json` sits next to `harnesses.json` in Shina's data directory
//! (`crate::store::directory()`). Each key names a provider, the harness id,
//! with Pi filed under its `cpa` provider id:
//!
//! ```json
//! {
//!   "cpa": {
//!     "gateway": {
//!       "url": "https://cpa.tailnet-name.ts.net:18317",
//!       "apiKey": { "env": "CPA_API_KEY" },
//!       "models": "gpt-6.1-sol"
//!     }
//!   },
//!   "codex": {
//!     "environment": [
//!       { "name": "OPENAI_BASE_URL", "value": "https://cpa.example/v1" },
//!       { "name": "OPENAI_API_KEY", "env": "CPA_API_KEY" }
//!     ]
//!   },
//!   "claude": {
//!     "environment": [
//!       { "name": "ANTHROPIC_BASE_URL", "value": "https://cpa.example" },
//!       { "name": "ANTHROPIC_AUTH_TOKEN", "env": "CPA_API_KEY" },
//!       { "name": "ANTHROPIC_API_KEY", "value": "" }
//!     ]
//!   },
//!   "antigravity": {
//!     "gateway": { "url": "https://cpa.example", "apiKey": { "env": "CPA_API_KEY" } }
//!   }
//! }
//! ```
//!
//! Each environment entry resolves one of three ways: a literal `value` (an
//! explicitly empty string clears the inherited variable, as in KSG), `env`
//! naming another process variable read at launch, or neither to inherit the
//! ambient variable of the same name. A gateway resolves field by field from
//! the file first, then the ambient `SHINA_<PROVIDER>_GATEWAY_URL`,
//! `SHINA_<PROVIDER>_GATEWAY_MODELS` and `SHINA_<PROVIDER>_API_KEY` variables,
//! so `SHINA_ANTIGRAVITY_GATEWAY_URL` alone configures `agy`.
//!
//! Secrets enter a child only as resolved environment variables. Every
//! `env`-sourced value and any literal marked `"sensitive": true` is passed to
//! the harness process and nowhere else: they are never logged and never
//! written into Shina's thread store. Shina reads this file; it never writes it.
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    fmt,
    path::Path,
    process::Command,
};

/// The user-managed override file, beside `harnesses.json`.
pub const FILE: &str = "providers.json";

/// `harness == "pi"` talks to the CLIPROXYAPI proxy, so its section is `cpa`.
/// Every other harness's provider id is its own id, including ACP agents.
pub fn provider_id(harness: &str) -> &str {
    if harness == "pi" { "cpa" } else { harness }
}

/// KSG `ProviderInstanceEnvironmentVariableName`: `^[a-zA-Z_][a-zA-Z0-9_]*$`, 128 max.
fn valid_variable_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
        })
}

/// Environment names are case-insensitive on Windows only, as in KSG's merge.
fn name_key(name: &str) -> String {
    if cfg!(windows) {
        name.to_uppercase()
    } else {
        name.to_owned()
    }
}

/// KSG's gateway check: an `http`/`https` URL with a nonempty host.
fn valid_gateway_url(url: &str) -> bool {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| !host.is_empty() && !host.chars().any(char::is_whitespace))
}

/// Where an entry's value comes from at launch.
#[derive(Clone, PartialEq)]
enum Source {
    /// Pass the ambient variable of the same name through when set.
    Inherit,
    /// Literal value; an empty string clears an inherited variable, as in KSG.
    Literal(String),
    /// Read another process variable at launch; unset removes the target.
    Env(String),
}

/// One configured variable in a provider's `environment` list.
#[derive(Clone)]
pub struct EnvOverride {
    /// The variable the child sees.
    pub name: String,
    /// Literals marked sensitive are redacted from `Debug`; `env`-sourced
    /// values are always treated as sensitive.
    pub sensitive: bool,
    source: Source,
}

impl fmt::Debug for EnvOverride {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("EnvOverride");
        debug
            .field("name", &self.name)
            .field("sensitive", &self.sensitive);
        match &self.source {
            Source::Inherit => {
                debug.field("source", &"inherit");
            }
            Source::Literal(value) => {
                debug.field(
                    "value",
                    &(if self.sensitive { "[redacted]" } else { value }),
                );
            }
            Source::Env(source) => {
                debug.field("env", source);
            }
        }
        debug.finish()
    }
}

/// A configured secret: a literal string, or `{"env": "NAME"}` read at launch.
/// `Debug` never shows the resolved or literal value.
#[derive(Clone, PartialEq)]
pub enum Secret {
    Literal(String),
    Env(String),
}

impl Secret {
    fn resolve(&self, lookup: &dyn Fn(&str) -> Option<OsString>) -> Option<OsString> {
        match self {
            Secret::Literal(value) => Some(value.clone().into()),
            Secret::Env(name) => lookup(name),
        }
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Secret::Literal(_) => write!(f, "Secret::Literal([redacted])"),
            Secret::Env(name) => write!(f, "Secret::Env({name})"),
        }
    }
}

/// One provider's `gateway` block: an OpenAI/Anthropic-compatible endpoint
/// such as CLIPROXYAPI, with its credential and optional model allowlist.
#[derive(Clone, PartialEq)]
pub struct Gateway {
    url: Option<String>,
    api_key: Option<Secret>,
    models: Option<String>,
}

impl fmt::Debug for Gateway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gateway")
            .field("url", &self.url)
            .field("api_key", &self.api_key)
            .field("models", &self.models)
            .finish()
    }
}

/// A gateway resolved against the ambient environment. `api_key` may carry a
/// credential; `Debug` redacts it and it must never be logged or persisted.
#[derive(Clone, PartialEq)]
pub struct ResolvedGateway {
    pub url: String,
    pub api_key: Option<OsString>,
    pub models: Option<String>,
}

impl fmt::Debug for ResolvedGateway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedGateway")
            .field("url", &self.url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .field("models", &self.models)
            .finish()
    }
}

impl ResolvedGateway {
    /// KSG `authMethod: "gateway"` for `agy acp`: the rollout flag plus the
    /// `AGY_LLM_GATEWAY_*` endpoint, credential and model list.
    pub fn antigravity_environment(&self) -> Vec<(String, OsString)> {
        let mut env = vec![
            ("AGY_ACP_ENABLE_GATEWAY_AUTH".into(), OsString::from("1")),
            ("AGY_LLM_GATEWAY_URL".into(), OsString::from(&self.url)),
        ];
        if let Some(api_key) = &self.api_key {
            env.push(("AGY_LLM_GATEWAY_API_KEY".into(), api_key.clone()));
        }
        if let Some(models) = &self.models {
            env.push(("AGY_LLM_GATEWAY_MODELS".into(), models.into()));
        }
        env
    }
}

/// One `providers.json` section after validation.
#[derive(Default)]
pub struct Provider {
    environment: Vec<EnvOverride>,
    gateway: Option<Gateway>,
}

impl Provider {
    /// The configured variables, in file order.
    pub fn overrides(&self) -> &[EnvOverride] {
        &self.environment
    }
}

/// The parsed `providers.json`. Like `harness::Registry`, an unreadable or
/// invalid file reports `error` and yields an empty registry; ambient
/// environment resolution still applies through `gateway`.
#[derive(Default)]
pub struct Registry {
    providers: BTreeMap<String, Provider>,
    /// The first read or validation problem, if any.
    pub error: Option<String>,
}

/// The resolved launch environment for one harness: variables to set and
/// variables to clear. `Debug` prints variable names only, because resolved
/// values may carry secrets headed to the child process.
#[derive(Default)]
#[must_use]
pub struct LaunchEnvironment {
    set: Vec<(String, OsString)>,
    remove: Vec<String>,
}

impl fmt::Debug for LaunchEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchEnvironment")
            .field(
                "set",
                &self.set.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            )
            .field("remove", &self.remove)
            .finish()
    }
}

impl LaunchEnvironment {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.remove.is_empty()
    }
    /// The resolved pairs, for harnesses that build an explicit environment
    /// themselves (Antigravity clears the environment it hands `agy`).
    pub fn pairs(&self) -> &[(String, OsString)] {
        &self.set
    }
    /// Every variable the launch explicitly controls, set or cleared.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.set
            .iter()
            .map(|(name, _)| name.as_str())
            .chain(self.remove.iter().map(String::as_str))
    }
    /// Clear, then set, the configured variables on `command`. The child
    /// still inherits every other ambient variable unless the caller cleared
    /// the environment first.
    pub fn apply(&self, command: &mut Command) {
        for name in &self.remove {
            command.env_remove(name);
        }
        for (name, value) in &self.set {
            command.env(name, value);
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEntry {
    name: String,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    env: Option<String>,
    #[serde(default)]
    sensitive: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FileSecret {
    Literal(String),
    Env { env: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FileGateway {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    api_key: Option<FileSecret>,
    #[serde(default)]
    models: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FileProvider {
    #[serde(default)]
    environment: Vec<FileEntry>,
    #[serde(default)]
    gateway: Option<FileGateway>,
}

/// `SHINA_<PROVIDER>_GATEWAY_URL`, `_GATEWAY_MODELS` and `_API_KEY`: the
/// ambient fallbacks for a provider's gateway, e.g. `SHINA_ANTIGRAVITY_*`
/// and `SHINA_CPA_*`.
fn ambient_gateway_names(provider: &str) -> (String, String, String) {
    let prefix = format!(
        "SHINA_{}",
        provider
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
    );
    (
        format!("{prefix}_GATEWAY_URL"),
        format!("{prefix}_GATEWAY_MODELS"),
        format!("{prefix}_API_KEY"),
    )
}

impl Registry {
    /// Read `providers.json` from Shina's data directory.
    pub fn read() -> Self {
        let Some(directory) = crate::store::directory() else {
            return Self {
                error: Some("Could not locate providers.json.".into()),
                ..Self::default()
            };
        };
        Self::read_in(&directory)
    }

    fn read_in(directory: &Path) -> Self {
        match std::fs::read(directory.join(FILE)) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) => match Self::from_value(&value) {
                    Ok(registry) => registry,
                    Err(error) => Self {
                        error: Some(format!("Invalid providers.json. {error}")),
                        ..Self::default()
                    },
                },
                Err(error) => Self {
                    error: Some(format!("Could not read providers.json: {error}")),
                    ..Self::default()
                },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => Self {
                error: Some(format!("Could not read providers.json: {error}")),
                ..Self::default()
            },
        }
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        let map = value
            .as_object()
            .ok_or("Use an object keyed by provider id.")?;
        let mut providers = BTreeMap::new();
        for (key, section) in map {
            let id = provider_id(key);
            if !crate::harness::valid_id(key) || providers.contains_key(id) {
                return Err("Use unique lowercase provider ids, with Pi filed under cpa.".into());
            }
            let file: FileProvider = serde_json::from_value(section.clone()).map_err(|_| {
                format!(
                    "Invalid {key} section in providers.json. Check field names and value types."
                )
            })?;
            let provider = Self::section(key, file)?;
            providers.insert(id.to_owned(), provider);
        }
        Ok(Self {
            providers,
            error: None,
        })
    }

    fn section(key: &str, file: FileProvider) -> Result<Provider, String> {
        let mut environment = Vec::new();
        let mut seen = HashSet::new();
        for entry in file.environment {
            if !valid_variable_name(&entry.name)
                || !seen.insert(name_key(&entry.name))
                || entry
                    .env
                    .as_deref()
                    .is_some_and(|name| !valid_variable_name(name))
            {
                return Err(format!(
                    "Invalid {key} environment. Use unique environment variable names."
                ));
            }
            if entry.value.is_some() && entry.env.is_some() {
                return Err(format!(
                    "Invalid {key} environment. Use value or env, not both."
                ));
            }
            let source = match (entry.value, entry.env) {
                (Some(value), None) => Source::Literal(value),
                (None, Some(env)) => Source::Env(env),
                (None, None) => Source::Inherit,
                (Some(_), Some(_)) => unreachable!("checked above"),
            };
            environment.push(EnvOverride {
                name: entry.name,
                sensitive: entry.sensitive || matches!(&source, Source::Env(_)),
                source,
            });
        }
        let gateway = file
            .gateway
            .map(|gateway| {
                if gateway
                    .url
                    .as_deref()
                    .is_some_and(|url| !valid_gateway_url(url))
                {
                    return Err(format!("Invalid {key} gateway. Use an http or https URL."));
                }
                if let Some(FileSecret::Env { env }) = &gateway.api_key
                    && !valid_variable_name(env)
                {
                    return Err(format!(
                        "Invalid {key} gateway. Use a valid environment variable name."
                    ));
                }
                Ok(Gateway {
                    url: gateway.url,
                    api_key: gateway.api_key.map(|secret| match secret {
                        FileSecret::Literal(value) => Secret::Literal(value),
                        FileSecret::Env { env } => Secret::Env(env),
                    }),
                    models: gateway.models,
                })
            })
            .transpose()?;
        Ok(Provider {
            environment,
            gateway,
        })
    }

    /// A provider section by harness id (`pi` reads `cpa`).
    pub fn provider(&self, harness: &str) -> Option<&Provider> {
        self.providers.get(provider_id(harness))
    }

    /// Every configured provider id, sorted.
    pub fn providers(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(String::as_str)
    }

    /// Resolve a harness's configured environment against `lookup`
    /// (the ambient environment outside tests). A literal `value` wins; an
    /// `env` source or a name-only entry reads the ambient variable, and an
    /// unset source removes the target so a stale inherited credential can
    /// never silently serve.
    pub fn environment_with(
        &self,
        harness: &str,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> LaunchEnvironment {
        let mut environment = LaunchEnvironment::default();
        let Some(provider) = self.provider(harness) else {
            return environment;
        };
        for entry in &provider.environment {
            match &entry.source {
                Source::Literal(value) => environment
                    .set
                    .push((entry.name.clone(), value.clone().into())),
                Source::Env(source) => match lookup(source) {
                    Some(value) => environment.set.push((entry.name.clone(), value)),
                    None => environment.remove.push(entry.name.clone()),
                },
                Source::Inherit => match lookup(&entry.name) {
                    Some(value) => environment.set.push((entry.name.clone(), value)),
                    None => environment.remove.push(entry.name.clone()),
                },
            }
        }
        environment
    }

    /// `environment_with` bound to the process environment.
    pub fn environment(&self, harness: &str) -> LaunchEnvironment {
        self.environment_with(harness, |name| std::env::var_os(name))
    }

    /// Clear, then set, the harness's configured variables on `command`.
    /// Returns the applied variable names — names only, never values.
    pub fn apply(&self, command: &mut Command, harness: &str) -> Vec<String> {
        let environment = self.environment(harness);
        environment.apply(command);
        environment.names().map(str::to_owned).collect()
    }

    /// A provider's gateway, resolved field by field from `providers.json`,
    /// then the ambient `SHINA_<PROVIDER>_GATEWAY_URL`,
    /// `SHINA_<PROVIDER>_GATEWAY_MODELS` and `SHINA_<PROVIDER>_API_KEY`
    /// variables. `None` when no endpoint is configured anywhere.
    pub fn gateway_with(
        &self,
        provider: &str,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Option<ResolvedGateway> {
        let (url_name, models_name, api_key_name) = ambient_gateway_names(provider);
        let configured = self
            .providers
            .get(provider_id(provider))
            .and_then(|p| p.gateway.as_ref());
        let url = configured
            .and_then(|gateway| gateway.url.clone())
            .or_else(|| lookup(&url_name).and_then(|v| v.into_string().ok()))?;
        let api_key = configured
            .and_then(|gateway| gateway.api_key.as_ref())
            .and_then(|secret| secret.resolve(&lookup))
            .or_else(|| lookup(&api_key_name));
        let models = configured
            .and_then(|gateway| gateway.models.clone())
            .or_else(|| lookup(&models_name).and_then(|v| v.into_string().ok()));
        Some(ResolvedGateway {
            url,
            api_key,
            models,
        })
    }

    /// `gateway_with` bound to the process environment.
    pub fn gateway(&self, provider: &str) -> Option<ResolvedGateway> {
        self.gateway_with(provider, |name| std::env::var_os(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn parse(json: serde_json::Value) -> Registry {
        match Registry::from_value(&json) {
            Ok(registry) => registry,
            Err(error) => Registry {
                error: Some(error),
                ..Registry::default()
            },
        }
    }

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(name, value)| (name.to_string(), OsString::from(value)))
            .collect();
        move |name| vars.get(name).cloned()
    }

    #[test]
    fn a_missing_file_is_empty_and_quiet() {
        let dir = crate::store::directory().unwrap();
        assert!(!dir.join(FILE).exists());
        let registry = Registry::read_in(&dir);
        assert!(registry.error.is_none());
        assert!(registry.provider("codex").is_none());
        assert!(registry.environment("codex").is_empty());
    }

    #[test]
    fn unreadable_and_invalid_files_report_an_error_without_dropping_ambient_gateways() {
        let dir = crate::store::directory().unwrap();
        std::fs::write(dir.join(FILE), "not json").unwrap();
        let registry = Registry::read_in(&dir);
        assert!(registry.error.is_some());
        assert!(registry.provider("codex").is_none());
        let ambient = lookup(&[("SHINA_CPA_GATEWAY_URL", "https://cpa.example")]);
        let gateway = registry.gateway_with("cpa", ambient).unwrap();
        assert_eq!(gateway.url, "https://cpa.example");
    }

    #[test]
    fn every_source_kind_resolves_in_file_order() {
        let registry = parse(json!({"codex": {"environment": [
            {"name": "OPENAI_BASE_URL", "value": "https://cpa.example/v1"},
            {"name": "OPENAI_API_KEY", "env": "CPA_API_KEY"},
            {"name": "OPENAI_MODEL"},
            {"name": "EMPTY_INHERIT"},
            {"name": "ANTHROPIC_API_KEY", "value": ""}
        ]}}));
        assert!(registry.error.is_none());
        let env = registry.environment_with(
            "codex",
            lookup(&[("CPA_API_KEY", "sk-cpa"), ("OPENAI_MODEL", "gpt-6.1-sol")]),
        );
        assert_eq!(
            env.pairs(),
            &[
                (
                    "OPENAI_BASE_URL".to_owned(),
                    OsString::from("https://cpa.example/v1")
                ),
                ("OPENAI_API_KEY".to_owned(), OsString::from("sk-cpa")),
                ("OPENAI_MODEL".to_owned(), OsString::from("gpt-6.1-sol")),
                ("ANTHROPIC_API_KEY".to_owned(), OsString::from("")),
            ]
        );
        assert_eq!(env.remove, vec!["EMPTY_INHERIT".to_owned()]);
    }

    #[test]
    fn an_unset_env_source_removes_the_target_instead_of_leaking_the_inherited_credential() {
        let registry = parse(json!({"claude": {"environment": [
            {"name": "ANTHROPIC_AUTH_TOKEN", "env": "SHINA_TEST_MISSING_VAR_9"}
        ]}}));
        assert!(registry.error.is_none());
        let env = registry.environment_with("claude", lookup(&[]));
        assert!(env.pairs().is_empty());
        assert_eq!(env.remove, vec!["ANTHROPIC_AUTH_TOKEN".to_owned()]);
    }

    #[test]
    fn apply_clears_then_sets_only_the_configured_names() {
        let registry = parse(json!({"codex": {"environment": [
            {"name": "OPENAI_BASE_URL", "value": "https://cpa.example/v1"},
            {"name": "OPENAI_API_KEY", "env": "SHINA_TEST_MISSING_VAR_9"}
        ]}}));
        assert!(registry.error.is_none());
        let mut command = Command::new("fixture");
        command.env("KEPT", "ambient");
        // An env-sourced variable whose source is unset clears the target.
        command.env("OPENAI_API_KEY", "stale-inherited");
        let names = registry.environment("codex").names().count();
        let applied = registry.apply(&mut command, "codex");
        assert_eq!(applied.len(), names);
        let envs: HashMap<_, _> = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert_eq!(envs["KEPT"], Some("ambient".into()));
        assert_eq!(
            envs["OPENAI_BASE_URL"],
            Some("https://cpa.example/v1".into())
        );
        assert_eq!(envs["OPENAI_API_KEY"], None);
    }

    #[test]
    fn pi_reads_the_cpa_section_and_duplicates_collapse_to_it() {
        let registry = parse(json!({"cpa": {"environment": [
            {"name": "OPENAI_BASE_URL", "value": "https://cpa.example/v1"}
        ]}}));
        assert!(registry.error.is_none());
        assert_eq!(provider_id("pi"), "cpa");
        assert!(registry.provider("pi").is_some());
        assert_eq!(
            registry.environment_with("pi", lookup(&[])).pairs()[0].0,
            "OPENAI_BASE_URL"
        );
        // "pi" and "cpa" name the same provider, so both cannot appear.
        let registry = parse(json!({"pi": {}, "cpa": {}}));
        assert!(registry.error.is_some());
    }

    #[test]
    fn gateway_resolves_file_first_then_shina_ambient_per_field() {
        let registry = parse(json!({"antigravity": {"gateway": {
            "url": "https://cpa.example",
            "apiKey": {"env": "CPA_API_KEY"}
        }}}));
        assert!(registry.error.is_none());
        let gateway = registry
            .gateway_with(
                "antigravity",
                lookup(&[
                    ("CPA_API_KEY", "sk-cpa"),
                    ("SHINA_ANTIGRAVITY_GATEWAY_MODELS", "gemini-4-carbon"),
                    ("SHINA_ANTIGRAVITY_GATEWAY_URL", "https://ambient.example"),
                ]),
            )
            .unwrap();
        // The file's url wins over the ambient one; models fall back to ambient.
        assert_eq!(gateway.url, "https://cpa.example");
        assert_eq!(gateway.api_key, Some(OsString::from("sk-cpa")));
        assert_eq!(gateway.models.as_deref(), Some("gemini-4-carbon"));
    }

    #[test]
    fn ambient_shina_variables_alone_configure_a_gateway() {
        let registry = Registry::default();
        let gateway = registry
            .gateway_with(
                "antigravity",
                lookup(&[
                    ("SHINA_ANTIGRAVITY_GATEWAY_URL", "https://agy.example"),
                    ("SHINA_ANTIGRAVITY_API_KEY", "sk-agy"),
                ]),
            )
            .unwrap();
        assert_eq!(gateway.url, "https://agy.example");
        assert_eq!(gateway.api_key, Some(OsString::from("sk-agy")));
        assert!(gateway.models.is_none());
        // No url from either source means no gateway.
        assert!(registry.gateway_with("cpa", lookup(&[])).is_none());
    }

    #[test]
    fn antigravity_environment_matches_ksg_gateway_auth() {
        let gateway = ResolvedGateway {
            url: "https://cpa.example".into(),
            api_key: Some(OsString::from("sk")),
            models: Some("gemini-4-carbon".into()),
        };
        assert_eq!(
            gateway.antigravity_environment(),
            vec![
                (
                    "AGY_ACP_ENABLE_GATEWAY_AUTH".to_owned(),
                    OsString::from("1")
                ),
                (
                    "AGY_LLM_GATEWAY_URL".to_owned(),
                    OsString::from("https://cpa.example")
                ),
                ("AGY_LLM_GATEWAY_API_KEY".to_owned(), OsString::from("sk")),
                (
                    "AGY_LLM_GATEWAY_MODELS".to_owned(),
                    OsString::from("gemini-4-carbon")
                ),
            ]
        );
    }

    #[test]
    fn invalid_files_are_rejected_like_harnesses_json() {
        for invalid in [
            json!([]),
            json!({"Bad Id": {}}),
            json!({"codex": {"environment": [{"name": "bad name", "value": "x"}]}}),
            json!({"codex": {"environment": [{"name": "1FOO", "value": "x"}]}}),
            json!({"codex": {"environment": [{"name": "DUP", "value": "a"}, {"name": "DUP", "value": "b"}]}}),
            json!({"codex": {"environment": [{"name": "BOTH", "value": "a", "env": "X"}]}}),
            json!({"codex": {"environment": [{"name": "OK", "env": "bad name"}]}}),
            json!({"codex": {"unknown": true}}),
            json!({"codex": {"gateway": {"url": "ftp://no"}}}),
            json!({"codex": {"gateway": {"url": "https://"}}}),
            json!({"codex": {"gateway": {"apiKey": {"env": "bad name"}}}}),
        ] {
            let registry = parse(invalid.clone());
            assert!(registry.error.is_some(), "{invalid}");
            assert!(registry.providers().next().is_none(), "{invalid}");
        }
        for valid in [
            json!({}),
            json!({"cpa": {"gateway": {"url": "https://cpa.example", "apiKey": {"env": "K"}}}}),
            json!({"codex": {"environment": [{"name": "A", "value": ""}]}}),
            json!({"codex": {"environment": [{"name": "A"}]}}),
            json!({"codex": {"environment": [{"name": "A", "env": "B", "sensitive": true}]}}),
            json!({"my-agent": {"environment": [{"name": "X", "value": "1"}]}}),
        ] {
            let registry = parse(valid.clone());
            assert!(registry.error.is_none(), "{valid}: {:?}", registry.error);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_variable_names_dedupe_case_insensitively() {
        let registry = parse(json!({"codex": {"environment": [
            {"name": "OPENAI_API_KEY", "value": "a"},
            {"name": "openai_api_key", "value": "b"}
        ]}}));
        assert!(registry.error.is_some());
    }

    #[test]
    fn secrets_stay_out_of_debug_and_names_never_leak_values() {
        let registry = parse(json!({"codex": {"environment": [
            {"name": "PLAIN", "value": "shown"},
            {"name": "TOKEN", "value": "sk-secret", "sensitive": true},
            {"name": "INDIRECT", "env": "HIDDEN"}
        ]}, "cpa": {"gateway": {"url": "https://cpa.example", "apiKey": "sk-file"}}}));
        assert!(registry.error.is_none());
        let overrides = registry.provider("codex").unwrap().overrides();
        let debug = format!("{overrides:?}");
        assert!(debug.contains("shown"));
        assert!(!debug.contains("sk-secret"));
        assert!(debug.contains("[redacted]"));
        let gateway = registry.gateway_with("cpa", lookup(&[])).unwrap();
        let debug = format!("{gateway:?}");
        assert!(debug.contains("https://cpa.example"));
        assert!(!debug.contains("sk-file"));
        let env = registry.environment_with("codex", lookup(&[("HIDDEN", "sk-env")]));
        let debug = format!("{env:?}");
        assert!(debug.contains("PLAIN") && debug.contains("TOKEN") && debug.contains("INDIRECT"));
        assert!(!debug.contains("sk-env"));
    }
}
