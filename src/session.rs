//! Provider metadata is independent of conversation runtimes. No prompts or credentials are cached.
use crate::backend::{Event, Pi};
use rune::Waker;
use serde_json::Value;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const START_TIMEOUT: Duration = Duration::from_secs(20);
const CATALOG_TTL: Duration = Duration::from_secs(5 * 60);
const CATALOG_RETRY_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentModel {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub thinking_levels: Vec<String>,
    /// The model's real context capacity, from Pi's model metadata. `None` when unknown.
    pub context_window: Option<u64>,
    /// Whether the model's `input` metadata accepts images.
    pub supports_images: Option<bool>,
    #[serde(default)]
    pub service_tiers: Vec<Value>,
    #[serde(default)]
    pub short_name: Option<String>,
    #[serde(default)]
    pub sub_provider: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub badge: Option<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub is_custom: bool,
    #[serde(default)]
    pub is_legacy: bool,
    #[serde(default)]
    pub is_unavailable: bool,
    #[serde(default)]
    pub capabilities: crate::descriptors::ModelCapabilities,
}

impl AgentModel {
    /// The levels the reasoning menu offers for this model: its reasoning
    /// control's selectable choices, or the plain list when it has no control.
    pub(crate) fn reasoning_levels(&self) -> Vec<String> {
        let choices =
            crate::descriptors::model_reasoning_levels(&self.capabilities.option_descriptors);
        if choices.is_empty() {
            self.thinking_levels.clone()
        } else {
            choices
        }
    }

    pub fn from_harness_value(model: &Value, harness: &str) -> Option<Self> {
        if harness == "pi" {
            return Self::from_value(model);
        }
        if model["provider"] != harness {
            return None;
        }
        let id = model["id"].as_str()?;
        let thinking_levels: Vec<String> = model["thinkingLevels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|level| level.as_str().map(str::to_owned))
            .collect();
        let service_tiers = model["serviceTiers"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let parsed_caps: Option<crate::descriptors::ModelCapabilities> = model
            .get("capabilities")
            .and_then(|c| serde_json::from_value(c.clone()).ok())
            .filter(|c: &crate::descriptors::ModelCapabilities| !c.option_descriptors.is_empty());
        let manifest_claude = if harness == "claude" {
            crate::descriptors::claude_manifest_model_entry(id)
        } else {
            None
        };
        let capabilities = if let Some(caps) = parsed_caps {
            caps
        } else if harness == "codex" {
            if model.get("supportedReasoningEfforts").is_some() {
                crate::descriptors::map_codex_model_capabilities(model, None)
            } else {
                let mut synthesized = serde_json::Map::new();
                synthesized.insert("model".into(), serde_json::json!(id));
                synthesized.insert(
                    "supportedReasoningEfforts".into(),
                    serde_json::json!(
                        thinking_levels
                            .iter()
                            .map(|l| serde_json::json!({"reasoningEffort": l}))
                            .collect::<Vec<_>>()
                    ),
                );
                if let Some(d) = model.get("defaultReasoningEffort") {
                    synthesized.insert("defaultReasoningEffort".into(), d.clone());
                }
                if !service_tiers.is_empty() {
                    synthesized.insert("serviceTiers".into(), serde_json::json!(service_tiers));
                }
                if let Some(d) = model.get("defaultServiceTier") {
                    synthesized.insert("defaultServiceTier".into(), d.clone());
                }
                crate::descriptors::map_codex_model_capabilities(&Value::Object(synthesized), None)
            }
        } else if let Some(entry) = &manifest_claude {
            entry.capabilities.clone()
        } else if harness == "claude" && !thinking_levels.is_empty() {
            crate::descriptors::ModelCapabilities {
                option_descriptors: vec![crate::descriptors::ProviderOptionDescriptor::Select {
                    id: "effort".into(),
                    label: "Reasoning".into(),
                    description: None,
                    options: std::iter::once(crate::descriptors::ProviderOptionChoice {
                        id: "default".into(),
                        label: "Default".into(),
                        description: None,
                        is_default: true,
                    })
                    .chain(thinking_levels.iter().map(|lvl| {
                        crate::descriptors::ProviderOptionChoice {
                            id: lvl.clone(),
                            label: match lvl.as_str() {
                                "low" => "Low".into(),
                                "medium" => "Medium".into(),
                                "high" => "High".into(),
                                "xhigh" => "Extra High".into(),
                                "max" => "Max".into(),
                                other => other.to_owned(),
                            },
                            description: None,
                            is_default: false,
                        }
                    }))
                    .collect(),
                    current_value: None,
                    prompt_injected_values: vec![],
                    role: Some(crate::descriptors::ProviderOptionRole::Reasoning),
                }],
            }
        } else {
            Default::default()
        };
        Some(Self {
            provider: harness.into(),
            id: id.into(),
            name: model["name"]
                .as_str()
                .or_else(|| model["id"].as_str())?
                .into(),
            thinking_levels,
            context_window: model["contextWindow"].as_u64().filter(|window| *window > 0),
            supports_images: model["input"]
                .as_array()
                .map(|input| input.iter().any(|kind| kind == "image")),
            service_tiers,
            short_name: model["shortName"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| manifest_claude.as_ref().and_then(|m| m.short_name.clone())),
            sub_provider: model["subProvider"].as_str().map(str::to_owned),
            aliases: model["aliases"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_else(|| {
                    manifest_claude
                        .as_ref()
                        .map(|m| m.aliases.clone())
                        .unwrap_or_default()
                }),
            badge: model["badge"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| manifest_claude.as_ref().and_then(|m| m.badge.clone())),
            is_default: model["isDefault"] == true,
            is_custom: model["isCustom"] == true,
            is_legacy: model["isLegacy"] == true
                || manifest_claude.as_ref().is_some_and(|m| m.is_legacy),
            is_unavailable: model["isUnavailable"] == true,
            capabilities,
        })
    }
    pub fn from_value(model: &Value) -> Option<Self> {
        if model["provider"] != "cpa" {
            return None;
        }
        let id = model["id"].as_str().filter(|id| !id.trim().is_empty())?;
        let capabilities = crate::descriptors::pi_model_capabilities(model, None);
        let thinking_levels = if model["reasoning"] == true {
            crate::descriptors::model_reasoning_levels(&capabilities.option_descriptors)
        } else {
            vec!["off".into()]
        };
        Some(Self {
            provider: "cpa".into(),
            id: id.into(),
            name: model["name"].as_str().unwrap_or(id).into(),
            thinking_levels,
            context_window: model["contextWindow"].as_u64().filter(|window| *window > 0),
            supports_images: model["input"]
                .as_array()
                .map(|input| input.iter().any(|kind| kind == "image")),
            service_tiers: model["serviceTiers"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            short_name: model["shortName"].as_str().map(str::to_owned),
            sub_provider: Some("cpa".into()),
            aliases: model["aliases"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a.as_str().map(str::to_owned))
                .collect(),
            badge: model["badge"].as_str().map(str::to_owned),
            is_default: model["isDefault"] == true,
            is_custom: model["isCustom"] == true,
            is_legacy: model["isLegacy"] == true,
            is_unavailable: model["isUnavailable"] == true,
            capabilities,
        })
    }

    pub(crate) fn pi_models(models: &[Value]) -> Vec<Self> {
        let mut seen = HashSet::new();
        models
            .iter()
            .filter_map(Self::from_value)
            .filter(|model| seen.insert(model.id.clone()))
            .collect()
    }

    pub(crate) fn apply_pi_thinking_default(&mut self, level: &str) {
        for descriptor in &mut self.capabilities.option_descriptors {
            if let crate::descriptors::ProviderOptionDescriptor::Select { id, options, .. } =
                descriptor
                && id == "thinking"
            {
                let available: Vec<_> = options.iter().map(|choice| choice.id.as_str()).collect();
                let default = crate::descriptors::clamp_pi_thinking_level(Some(level), &available)
                    .map(str::to_owned);
                for choice in options {
                    choice.is_default = default.as_ref() == Some(&choice.id);
                }
            }
        }
    }
}

/// KSG's cached provider probe. One model probe per app start/explicit refresh,
/// with a separate working-directory skill inventory. No thread or prompt is created.
#[derive(Default)]
pub struct NativeCatalog {
    pub models: Vec<AgentModel>,
    pub error: Option<String>,
    pub commands: Vec<Value>,
    pub extensions: bool,
    pub cwd: Option<PathBuf>,
    /// The workspace that produced `self.models`, tracked separately from `self.cwd`
    /// while a scan for another workspace is in flight.
    pub models_cwd: Option<PathBuf>,
    pub revision: u64,
    pub installed: bool,
    pub version: Option<String>,
    pub auth_status: Option<String>,
    pub update_required_models: Vec<crate::model_picker::UpdateRequiredModel>,
    /// KSG's `probe.message`: advisory text (e.g. a version-upgrade notice) that is
    /// not an error.
    pub provider_message: Option<String>,
    pub usage_limits: Value,
    pub usage_limits_raw: Value,
    pub usage_account: Value,
    /// KSG `workspaceSnapshots[cwd].models`: a folder's own provider config can
    /// add models that only its workspace lists; every discovered cwd keeps its
    /// own list after the catalog moves on.
    pub workspace_snapshots: std::collections::BTreeMap<PathBuf, Vec<AgentModel>>,
    pi: Option<Pi>,
    attempted: bool,
    complete: bool,
    models_loaded: bool,
    commands_loaded: bool,
    started: Option<Instant>,
    loaded: Option<Instant>,
    last_attempt: Option<Instant>,
    agent: Option<crate::harness::Agent>,
}

impl NativeCatalog {
    pub(crate) fn stop_for_update(&mut self) -> Option<crate::backend::ShutdownFence> {
        let fence = self.pi.take().map(|pi| pi.stop_fence());
        self.started = None;
        // The first post-update read must discover the new executable's metadata.
        self.attempted = false;
        self.loaded = None;
        self.last_attempt = None;
        fence
    }

    pub fn snapshot(&self) -> Value {
        serde_json::json!({"models":self.models,"commands":self.commands,"cwd":self.cwd,
            "modelsCwd":self.models_cwd,"workspaceSnapshots":self.workspace_snapshots,
            "extensions":self.extensions,"ready":self.complete,"error":self.error,"loading":self.loading(),
            "installed":self.installed,"version":self.version,"authStatus":self.auth_status,
            "updateRequiredModels":self.update_required_models,"providerMessage":self.provider_message,
            "usageLimits":self.usage_limits,"usageLimitsRaw":self.usage_limits_raw,
            "usageAccount":self.usage_account,"name":self.agent.as_ref().map(|agent| &agent.name)})
    }
    pub fn loading(&self) -> bool {
        self.pi.is_some()
    }

    pub fn refresh(
        &mut self,
        harness: &str,
        workspace: &Path,
        wake: Waker,
        force: bool,
        interval: bool,
    ) -> bool {
        // Claude discovery starts a CLI. After the first attempt, recheck on
        // demand or on a workspace change, never on the idle timer alone.
        if harness == "claude"
            && interval
            && !force
            && self.attempted
            && self.cwd.as_deref() == Some(workspace)
        {
            return false;
        }
        self.refresh_with(workspace, force, |models, workspace| match harness {
            "codex" => Ok(crate::codex::discover(workspace, wake, models)),
            "claude" => Ok(crate::claude::discover(workspace, wake)),
            "antigravity" => crate::antigravity::discover(workspace, wake),
            "cursor" => Ok(crate::cursor::discover(workspace, wake)),
            _ => unreachable!(),
        })
    }

    pub fn refresh_acp(
        &mut self,
        agent: &crate::harness::Agent,
        workspace: &Path,
        wake: Waker,
        force: bool,
        interval: bool,
    ) -> bool {
        if self.agent.as_ref() != Some(agent) {
            *self = Self {
                agent: Some(agent.clone()),
                revision: self.revision,
                ..Default::default()
            };
        }
        // KSG `refreshOnInterval`: each check starts the CLI again, so a healthy
        // Grok or OpenCode catalog is rechecked only on an explicit refresh or a
        // directory change; an unhealthy one keeps polling until it recovers.
        const INTERVAL_SKIP: &[&str] = &["grok", "opencode"];
        if interval
            && !force
            && INTERVAL_SKIP.contains(&agent.id.as_str())
            && self.complete
            && self.error.is_none()
            && self.cwd.as_deref() == Some(workspace)
        {
            return false;
        }
        if !force
            && self
                .error
                .as_deref()
                .is_some_and(crate::usage_limits::authentication_error)
        {
            return false;
        }
        self.refresh_with(workspace, force, |_, workspace| {
            let start = crate::harness::Start::discovery(&agent.id, workspace);
            crate::acp::start(agent.clone(), start, wake)
        })
    }

    #[cfg(test)]
    pub(crate) fn from_acp_fixture(agent: crate::harness::Agent, models: Vec<AgentModel>) -> Self {
        Self {
            agent: Some(agent),
            models,
            installed: true,
            complete: true,
            revision: 1,
            ..Default::default()
        }
    }

    fn refresh_with(
        &mut self,
        workspace: &Path,
        force: bool,
        discover: impl FnOnce(bool, PathBuf) -> Result<Pi, String>,
    ) -> bool {
        if self.pi.is_some() {
            return false;
        }
        let now = Instant::now();
        let changed = self.cwd.as_deref() != Some(workspace);
        if !force {
            if self
                .loaded
                .is_some_and(|at| now.saturating_duration_since(at) < CATALOG_TTL)
                && !changed
            {
                return false;
            }
            if self.error.is_some()
                && self
                    .last_attempt
                    .is_some_and(|at| now.saturating_duration_since(at) < CATALOG_RETRY_TTL)
            {
                return false;
            }
            if self.attempted && !changed && self.loaded.is_none() && self.error.is_none() {
                return false;
            }
        }
        let models = force
            || changed
            || !self.attempted
            || self
                .loaded
                .is_some_and(|at| now.saturating_duration_since(at) >= CATALOG_TTL);
        self.attempted = true;
        self.revision += 1;
        self.cwd = Some(workspace.to_path_buf());
        self.commands.clear();
        self.commands_loaded = false;
        self.models_loaded = !models;
        self.complete = false;
        self.error = None;
        self.started = Some(now);
        self.last_attempt = Some(now);
        match discover(models, workspace.to_path_buf()) {
            Ok(pi) => self.pi = Some(pi),
            Err(error) => self.fail(error),
        }
        true
    }

    pub fn poll(&mut self, harness: &str) -> bool {
        let mut changed = false;
        for _ in 0..256 {
            let Some(event) = self.pi.as_ref().and_then(|pi| pi.events.try_recv().ok()) else {
                break;
            };
            changed = true;
            match event {
                Event::Record(record) if record["type"] == "harness_update" => {
                    match record["field"].as_str() {
                        Some("codexNativeExtensions") => {
                            self.extensions = record["value"] == true;
                        }
                        Some("usageLimits") => {
                            self.usage_limits = record["value"].clone();
                        }
                        Some("usageLimitsRaw") => {
                            self.usage_limits_raw = record["value"].clone();
                        }
                        Some("usageAccount") => {
                            self.usage_account = record["value"].clone();
                        }
                        Some("updateRequiredModels") => {
                            self.update_required_models =
                                serde_json::from_value(record["value"].clone()).unwrap_or_default();
                        }
                        Some("providerMessage") => {
                            self.provider_message = record["value"].as_str().map(str::to_owned);
                        }
                        Some("version") => {
                            self.version = record["value"].as_str().map(str::to_owned);
                        }
                        _ => {}
                    }
                }
                Event::Record(record) if record["type"] == "response" => {
                    // A real CLI response proves that this harness is installed.
                    // The old default false leaked into projections even after a
                    // successful catalog probe, disabling working remote harnesses.
                    self.installed = true;
                    if record["success"] == false {
                        self.fail(
                            record["error"]
                                .as_str()
                                .unwrap_or("Discovery failed.")
                                .into(),
                        );
                        break;
                    }
                    match record["command"].as_str() {
                        Some("get_available_models") => {
                            self.models = record["data"]["models"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|model| AgentModel::from_harness_value(model, harness))
                                .collect();
                            self.models_loaded = true;
                            self.models_cwd = self.cwd.clone();
                            if let Some(cwd) = self.cwd.clone() {
                                self.workspace_snapshots.insert(cwd, self.models.clone());
                            }
                            self.revision += 1;
                        }
                        Some("get_commands") => {
                            self.commands = record["data"]["commands"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            self.commands_loaded = true;
                        }
                        _ => {}
                    }
                    if self.models_loaded && self.commands_loaded {
                        self.pi = None;
                        self.started = None;
                        self.loaded = Some(Instant::now());
                        self.complete = true;
                        self.revision += 1;
                        break;
                    }
                }
                Event::Error(error) => {
                    self.fail(error);
                    break;
                }
                Event::Exited => {
                    self.fail(format!("{harness} stopped during catalog discovery."));
                    break;
                }
                _ => {}
            }
        }
        let timeout = if self.agent.is_some() || harness == "antigravity" {
            Duration::from_secs(27)
        } else {
            START_TIMEOUT
        };
        if self.started.is_some_and(|at| at.elapsed() >= timeout) {
            self.fail(format!("{harness} catalog discovery timed out."));
            changed = true;
        }
        changed
    }

    pub fn commands_ready(&self) -> bool {
        self.complete
    }

    /// See `models_owned_by`.
    pub(crate) fn models_for_workspace(&self, cwd: Option<&Path>) -> Option<&[AgentModel]> {
        models_owned_by(
            &self.models,
            &self.workspace_snapshots,
            self.cwd.as_deref(),
            self.models_cwd.as_deref(),
            cwd,
        )
    }

    pub(crate) fn set_workspace_models(&mut self, cwd: &Path, models: &[AgentModel]) {
        if self.workspace_snapshots.get(cwd).map(Vec::as_slice) != Some(models) {
            self.workspace_snapshots
                .insert(cwd.to_path_buf(), models.to_vec());
            self.revision += 1;
        }
    }

    /// A live session's catalog must not replace the list for another folder.
    pub(crate) fn apply_live_acp_models(&mut self, models: &[AgentModel], cwd: Option<&Path>) {
        if models.is_empty() {
            return;
        }
        if let Some(cwd) = cwd {
            self.set_workspace_models(cwd, models);
        }
        // Discovery's own list changes only for the folder it is scanning. A session
        // in another folder, or one whose folder is unknown, cannot claim it.
        if self.cwd.is_some() && self.cwd.as_deref() != cwd {
            return;
        }
        let target_cwd = cwd.map(Path::to_path_buf);
        if self.models == models && self.models_cwd == target_cwd {
            return;
        }
        self.models = models.to_vec();
        self.models_cwd = target_cwd;
        self.installed = true;
        self.revision += 1;
    }

    /// Matches `refresh_with`'s retry gate: a failed or stale scan may run again.
    pub(crate) fn retry_ready(&self) -> bool {
        !self
            .error
            .as_deref()
            .is_some_and(crate::usage_limits::authentication_error)
            && self
                .last_attempt
                .is_none_or(|at| at.elapsed() >= CATALOG_RETRY_TTL)
    }

    fn fail(&mut self, error: String) {
        self.revision += 1;
        self.error = Some(error);
        self.pi = None;
        self.started = None;
    }
}

/// The one rule for which workspace a catalog's models belong to, shared by a
/// server's `NativeCatalog` and a client's projected copy of it.
///
/// A folder's recorded snapshot wins, including an empty one (`Some(&[])`).
/// Otherwise `models` answer only the folder that produced them (`models_cwd`).
/// A catalog that has recorded no folder at all, such as a fixture or a server
/// from before folders were tracked, answers every caller. An unknown `cwd`
/// matches nothing else, so it can never borrow another folder's list.
pub(crate) fn models_owned_by<'a>(
    models: &'a [AgentModel],
    workspace_snapshots: &'a std::collections::BTreeMap<PathBuf, Vec<AgentModel>>,
    catalog_cwd: Option<&Path>,
    models_cwd: Option<&Path>,
    cwd: Option<&Path>,
) -> Option<&'a [AgentModel]> {
    if let Some(models) = cwd.and_then(|cwd| workspace_snapshots.get(cwd)) {
        return Some(models.as_slice());
    }
    let unowned = models_cwd.is_none() && catalog_cwd.is_none();
    let owned = unowned || (cwd.is_some() && cwd == models_cwd);
    (owned && !models.is_empty()).then_some(models)
}

#[cfg(test)]
mod native_catalog_tests {
    use super::*;

    #[test]
    fn claude_catalog_does_not_relaunch_on_the_poll_timer() {
        let workspace = Path::new("workspace");
        let mut catalog = NativeCatalog {
            cwd: Some(workspace.into()),
            loaded: Some(Instant::now() - CATALOG_TTL),
            attempted: true,
            complete: true,
            ..Default::default()
        };
        let wake: Waker = std::sync::Arc::new(|| {});
        assert!(!catalog.refresh("claude", workspace, wake.clone(), false, true));
        catalog.fail("fixture probe failed".into());
        assert!(!catalog.refresh("claude", workspace, wake, false, true));
    }

    #[test]
    fn an_unknown_workspace_never_owns_or_borrows_a_folders_models() {
        let folder = PathBuf::from("known-workspace");
        let model = |id: &str| AgentModel {
            id: id.into(),
            ..Default::default()
        };
        // Discovery is scanning `folder` and holds models of unrecorded origin.
        let mut catalog = NativeCatalog {
            cwd: Some(folder.clone()),
            models: vec![model("unrecorded")],
            ..Default::default()
        };
        assert!(catalog.models_for_workspace(None).is_none());
        assert!(catalog.models_for_workspace(Some(&folder)).is_none());
        // A live session with no known folder cannot become that folder's list.
        let revision = catalog.revision;
        catalog.apply_live_acp_models(&[model("unknown-session")], None);
        assert_eq!(catalog.models, [model("unrecorded")]);
        assert_eq!(catalog.models_cwd, None);
        assert_eq!(catalog.revision, revision);
        assert!(catalog.workspace_snapshots.is_empty());
        // A catalog that never recorded a folder still answers every caller.
        let fixture = NativeCatalog {
            models: vec![model("fixture")],
            ..Default::default()
        };
        assert_eq!(
            fixture.models_for_workspace(None),
            Some(&[model("fixture")][..])
        );
        assert_eq!(
            fixture.models_for_workspace(Some(&folder)),
            Some(&[model("fixture")][..])
        );
    }

    #[test]
    fn live_acp_models_stay_in_the_session_workspace() {
        let first = PathBuf::from("first-workspace");
        let second = PathBuf::from("second-workspace");
        let model = |id: &str| AgentModel {
            id: id.into(),
            ..Default::default()
        };
        let mut catalog = NativeCatalog {
            cwd: Some(first.clone()),
            models: vec![model("first-model")],
            ..Default::default()
        };
        catalog.set_workspace_models(&first, &[model("first-model")]);
        let revision = catalog.revision;
        catalog.apply_live_acp_models(&[model("new-second-model")], Some(&second));
        assert_eq!(catalog.models, [model("first-model")]);
        assert_eq!(catalog.workspace_snapshots[&first], [model("first-model")]);
        assert_eq!(
            catalog.workspace_snapshots[&second],
            [model("new-second-model")]
        );
        assert!(catalog.revision > revision);
    }
}

#[derive(Default)]
pub struct Catalog {
    pub models: Vec<AgentModel>,
    pub default_model: Option<AgentModel>,
    pub default_thinking: String,
    pub error: Option<String>,
    pi: Option<Pi>,
    started: Option<Instant>,
    loaded: Option<Instant>,
    last_attempt: Option<Instant>,
    state_loaded: bool,
    models_loaded: bool,
}

impl Catalog {
    pub fn loading(&self) -> bool {
        self.pi.is_some()
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.started
            .map(|at| START_TIMEOUT.saturating_sub(at.elapsed()))
    }

    pub fn should_refresh(&self, force: bool, now: Instant) -> bool {
        if self.pi.is_some() {
            return false;
        }
        if force {
            return true;
        }
        if self.error.is_none()
            && self
                .loaded
                .is_some_and(|at| now.saturating_duration_since(at) < CATALOG_TTL)
        {
            return false;
        }
        if self
            .last_attempt
            .is_some_and(|at| now.saturating_duration_since(at) < CATALOG_RETRY_TTL)
        {
            return false;
        }
        true
    }

    pub fn refresh(&mut self, workspace: PathBuf, wake: Waker) {
        self.refresh_with_mode(workspace, wake, false);
    }

    pub fn refresh_with_mode(&mut self, workspace: PathBuf, wake: Waker, force: bool) {
        let now = Instant::now();
        if !self.should_refresh(force, now) {
            return;
        }
        self.error = None;
        self.state_loaded = false;
        self.models_loaded = false;
        self.started = Some(now);
        self.last_attempt = Some(now);
        self.pi = Some(Pi::discover(workspace, wake));
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..256 {
            let Some(event) = self.pi.as_ref().and_then(|pi| pi.events.try_recv().ok()) else {
                break;
            };
            changed = true;
            match event {
                Event::Record(record) if record["type"] == "response" => {
                    if record["success"] == false {
                        self.fail(
                            record["error"]
                                .as_str()
                                .unwrap_or("Model discovery failed.")
                                .into(),
                        );
                        break;
                    }
                    match record["command"].as_str() {
                        Some("get_state") => {
                            self.default_model = AgentModel::from_value(&record["data"]["model"]);
                            self.default_thinking = record["data"]["thinkingLevel"]
                                .as_str()
                                .unwrap_or("")
                                .into();
                            self.state_loaded = true;
                        }
                        Some("get_available_models") => {
                            let Some(models) = record["data"]["models"].as_array() else {
                                self.fail(
                                    "Pi returned an invalid model catalog. Retry to read it."
                                        .into(),
                                );
                                break;
                            };
                            self.models = AgentModel::pi_models(models);
                            if self.models.is_empty() {
                                self.fail("Pi returned no CPA models. Check the existing CPA enrollment launcher and retry catalog discovery. No direct-provider fallback was used.".into());
                                break;
                            }
                            self.models_loaded = true;
                        }
                        _ => {}
                    }
                    if self.state_loaded && self.models_loaded {
                        if let Some(model) = &mut self.default_model {
                            model.apply_pi_thinking_default(&self.default_thinking);
                            if let Some(row) = self.models.iter_mut().find(|row| row.id == model.id)
                            {
                                row.apply_pi_thinking_default(&self.default_thinking);
                            }
                        }
                        self.pi = None;
                        self.started = None;
                        self.loaded = Some(Instant::now());
                    }
                }
                Event::Error(error) => self.fail(error),
                Event::Exited => self.fail(
                    "Pi stopped during model discovery. Retry to read the CPA catalog.".into(),
                ),
                _ => {}
            }
        }
        if self.started.is_some_and(|at| at.elapsed() >= START_TIMEOUT) {
            self.fail("Model discovery timed out. Retry to read the CPA catalog.".into());
            changed = true;
        }
        changed
    }

    fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.pi = None;
        self.started = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_catalog_caches_models_and_workspace_commands_without_a_thread() {
        for harness in ["codex", "claude"] {
            let (pi, commands, events) = Pi::test_connection();
            let mut catalog = NativeCatalog {
                pi: Some(pi),
                ..Default::default()
            };
            events
                .send(Event::Record(
                    json!({"type":"response","command":"get_available_models","success":true,
                "data":{"models":[{"provider":harness,"id":"fixture","thinkingLevels":["high"]}]}}),
                ))
                .unwrap();
            events
                .send(Event::Record(
                    json!({"type":"response","command":"get_commands","success":true,
                "data":{"commands":[{"name":"$fixture","description":"Project skill"}]}}),
                ))
                .unwrap();
            assert!(catalog.poll(harness));
            assert!(catalog.pi.is_none());
            assert!(catalog.commands_ready());
            assert!(catalog.installed);
            assert_eq!(catalog.snapshot()["installed"], true);
            assert_eq!(catalog.models[0].thinking_levels, ["high"]);
            assert!(commands.try_recv().is_err());
        }
    }

    #[test]
    fn catalog_discovery_is_independent_of_a_thread_and_releases_its_process() {
        let (pi, commands, events) = Pi::test_connection();
        let mut catalog = Catalog {
            pi: Some(pi),
            ..Default::default()
        };
        for record in [
            json!({"type":"response","command":"get_state","success":true,"data":{
                "model":{"provider":"cpa","id":"default","reasoning":true},"thinkingLevel":"high"
            }}),
            json!({"type":"response","command":"get_available_models","success":true,"data":{"models":[
                {"provider":"direct","id":"hidden"},
                {"provider":"cpa","id":"default","reasoning":true,"thinkingLevelMap":{"minimal":null,"xhigh":"xhigh","max":"max"},"contextWindow":200000,"input":["text","image"]},
                {"provider":"cpa","id":"default","name":"Duplicate"},
                {"provider":"cpa","id":"   ","name":"Invalid"},
                {"provider":"cpa","id":"plain","reasoning":false,"contextWindow":32000,"input":["text"]}
            ]}}),
        ] {
            events.send(Event::Record(record)).unwrap();
        }
        assert!(catalog.poll());
        assert!(catalog.pi.is_none());
        assert!(commands.try_recv().is_err(), "discovery sends no prompts");
        assert_eq!(catalog.models.len(), 2);
        assert_eq!(catalog.default_model.as_ref().unwrap().id, "default");
        assert_eq!(catalog.default_thinking, "high");
        assert_eq!(
            catalog.models[0].thinking_levels,
            ["off", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(catalog.models[1].thinking_levels, ["off"]);
        assert_eq!(catalog.models[0].context_window, Some(200000));
        assert_eq!(catalog.models[0].sub_provider.as_deref(), Some("cpa"));
        let descriptor = &catalog.models[0].capabilities.option_descriptors[0];
        assert_eq!(
            crate::descriptors::selectable_provider_option_choices(descriptor)
                .into_iter()
                .find(|c| c.is_default)
                .unwrap()
                .id,
            "high"
        );
        assert_eq!(catalog.models[0].supports_images, Some(true));
        assert_eq!(catalog.models[1].context_window, Some(32000));
        assert_eq!(catalog.models[1].supports_images, Some(false));
        assert!(catalog.loaded.is_some());
    }
    #[test]
    fn a_pi_response_without_cpa_models_is_an_explicit_failure() {
        let (pi, _, events) = Pi::test_connection();
        let mut catalog = Catalog {
            pi: Some(pi),
            ..Default::default()
        };
        events
            .send(Event::Record(
                json!({"type":"response","command":"get_available_models",
            "success":true,"data":{"models":[{"provider":"direct","id":"not-cpa"}]}}),
            ))
            .unwrap();
        assert!(catalog.poll());
        assert!(catalog.models.is_empty());
        assert!(catalog.error.as_deref().unwrap().contains("no CPA models"));
        assert!(!catalog.loading());
    }

    #[test]
    fn model_metadata_does_not_advertise_unmapped_extended_levels() {
        let model =
            AgentModel::from_value(&json!({"provider":"cpa","id":"reasoning","reasoning":true}))
                .unwrap();
        assert_eq!(
            model.thinking_levels,
            ["off", "minimal", "low", "medium", "high"]
        );
        assert!(AgentModel::from_value(&json!({"provider":"direct","id":"hidden"})).is_none());
    }

    #[test]
    fn model_metadata_keeps_known_capacity_and_vision_support() {
        let model = AgentModel::from_value(&json!({
            "provider":"cpa","id":"vision","name":"Vision","contextWindow":1000000,
            "input":["text","image"]
        }))
        .unwrap();
        assert_eq!(model.context_window, Some(1000000));
        assert_eq!(model.supports_images, Some(true));

        let text = AgentModel::from_value(&json!({
            "provider":"cpa","id":"text","contextWindow":128000,"input":["text"]
        }))
        .unwrap();
        assert_eq!(text.context_window, Some(128000));
        assert_eq!(text.supports_images, Some(false));

        for bad in [
            json!({"provider":"cpa","id":"missing"}),
            json!({"provider":"cpa","id":"zero","contextWindow":0,"input":[]}),
            json!({"provider":"cpa","id":"null","contextWindow":null,"input":null}),
        ] {
            let model = AgentModel::from_value(&bad).unwrap();
            assert_eq!(model.context_window, None, "{bad}");
            assert_eq!(
                model.supports_images,
                bad["input"].as_array().map(|_| false),
                "{bad}"
            );
        }
    }

    #[test]
    fn discovery_failure_keeps_the_previous_catalog_available() {
        let (pi, _commands, events) = Pi::test_connection();
        let mut catalog = Catalog {
            pi: Some(pi),
            models: vec![AgentModel::from_value(&json!({"provider":"cpa","id":"cached"})).unwrap()],
            started: Some(Instant::now()),
            ..Default::default()
        };
        events
            .send(Event::Error("Discovery failed".into()))
            .unwrap();
        catalog.poll();
        assert!(catalog.error.is_some());
        assert!(catalog.pi.is_none());
        assert_eq!(catalog.models[0].id, "cached");
        assert!(catalog.remaining().is_none());
    }

    #[test]
    fn catalog_on_demand_throttles_success_and_failure_while_force_bypasses_backoff() {
        let now = Instant::now();
        let mut catalog = Catalog::default();
        // Recently loaded within 5 minutes -> on-demand should skip
        catalog.loaded = Some(now - Duration::from_secs(60));
        assert!(!catalog.should_refresh(false, now));
        assert!(catalog.should_refresh(true, now));

        // Stale > 5 minutes -> on-demand should refresh
        catalog.loaded = Some(now - Duration::from_secs(301));
        assert!(catalog.should_refresh(false, now));

        // Failed within 5 minutes -> on-demand backs off, force retries immediately
        catalog.loaded = None;
        catalog.error = Some("Offline".into());
        catalog.last_attempt = Some(now - Duration::from_secs(60));
        assert!(!catalog.should_refresh(false, now));
        assert!(catalog.should_refresh(true, now));

        // Failed > 5 minutes ago -> on-demand may retry
        catalog.last_attempt = Some(now - Duration::from_secs(301));
        assert!(catalog.should_refresh(false, now));

        // Single-flight: in-flight probe blocks both on-demand and force
        let (pi, _, _) = Pi::test_connection();
        catalog.pi = Some(pi);
        assert!(!catalog.should_refresh(false, now));
        assert!(!catalog.should_refresh(true, now));
    }
}
