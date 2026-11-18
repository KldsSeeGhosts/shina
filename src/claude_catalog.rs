//! Claude Code model catalog, ported function by function from KSG Code:
//! - `apps/server/src/provider/ClaudeModelCatalog.ts`
//! - `apps/server/src/provider/ClaudeModelManifest.ts`
//! - `apps/server/src/provider/ClaudeCodeModelRoutes.ts` (`claudeModelSettingsKey`)
//! - `apps/server/src/provider/ClaudeProvider.ts` (`parseClaudeEffortCapSources`)
//! - `apps/server/src/provider/ModelManifest.ts` (`resolveProviderCatalog`)
//! - `packages/provider-core/src/server/ModelCatalog.ts` (`catalogServerModel`)
//! - `packages/shared/src/semver.ts` (`parseSemver`, `compareSemverVersions`)
//!
//! KSG's `scopeClaudeModelCatalog` (custom models) is not ported: Shina has no
//! Claude `custom_models`/`customModels` setting.
//!
//! KSG routes each session through `routeModelCatalog(base, cwd, launchSettings)`:
//! routes hold where the turn cwd resolves the same model settings as the probe's
//! cwd, and never when launch args carry model settings. Shina launches Claude
//! without a `--settings` argument and every session reports its own model list at
//! `initialize`, so each session routes from its own report; the settings-key
//! comparison is meaningful only between a shared probe and a different cwd, which
//! per-session reports already cover. `claude_model_settings_key` is ported for
//! the catalog probe's metadata and the launch-settings rule.
use crate::descriptors::{
    ModelCapabilities, ProviderOptionChoice, ProviderOptionDescriptor,
    get_provider_option_current_value, get_provider_option_descriptors,
    selectable_provider_option_choices, with_ultracode_toggle,
};
use crate::model_selection::{ModelSelection, OptionValue, ProviderOptionSelection};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

const MODEL_MANIFEST_JSON: &str = include_str!("../assets/model-manifest.json");
const MODEL_NEW_BADGE_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// KSG `ClaudeModelInfo` (`@anthropic-ai/claude-agent-sdk` `ModelInfo`), as the
/// `initialize` control response reports it.
#[derive(Clone, Debug, Default)]
pub struct ClaudeReportedModel {
    pub value: String,
    pub resolved_model: Option<String>,
    pub display_name: String,
    pub supports_effort: bool,
    pub supported_effort_levels: Vec<String>,
}

pub fn parse_claude_reported_models(data: &Value) -> Vec<ClaudeReportedModel> {
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let value = model["value"].as_str()?.to_owned();
            Some(ClaudeReportedModel {
                value,
                resolved_model: model["resolvedModel"].as_str().map(str::to_owned),
                display_name: model["displayName"].as_str().unwrap_or("").to_owned(),
                supports_effort: model["supportsEffort"] == true,
                supported_effort_levels: model["supportedEffortLevels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
            })
        })
        .collect()
}

/// Ported from `ClaudeModelManifest.ts` (`ClaudeCodeProfileSchema`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClaudeCodeProfile {
    pub effort_map: BTreeMap<String, Option<String>>,
    pub model_suffixes: BTreeMap<String, BTreeMap<String, String>>,
    pub context_window_tokens: BTreeMap<String, u64>,
    pub fixed_context_window_tokens: Option<u64>,
}

/// Ported from `ClaudeModelManifest.ts` (`ClaudeCodeCompatibilitySchema`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClaudeCodeCompatibility {
    pub min_version: Option<String>,
    pub max_version_exclusive: Option<String>,
}

/// KSG `ServerProviderModel`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeModelRow {
    pub slug: String,
    pub name: String,
    pub short_name: Option<String>,
    pub sub_provider: Option<String>,
    pub aliases: Vec<String>,
    pub badge: Option<String>,
    pub is_default: bool,
    pub is_custom: bool,
    pub is_legacy: bool,
    pub capabilities: ModelCapabilities,
}

/// KSG `ClaudeCatalogModel`.
#[derive(Clone, Debug)]
pub struct ClaudeCatalogModel {
    pub model: ClaudeModelRow,
    pub runtime: ClaudeCodeProfile,
    pub compatibility: ClaudeCodeCompatibility,
    /// What to send Claude Code instead of the slug, from its own model list.
    pub api_model: Option<String>,
}

/// KSG `ClaudeModelCatalog`.
#[derive(Clone, Debug, Default)]
pub struct ClaudeModelCatalog {
    pub models: Vec<ClaudeCatalogModel>,
}

/// `TrimmedNonEmptyString`: the manifest schema rejects empty and padded values.
fn trimmed_non_empty(value: &Value) -> Option<String> {
    let trimmed = value.as_str()?.trim();
    (!trimmed.is_empty() && trimmed.len() == value.as_str()?.len())
        .then(|| trimmed.to_owned())
}

/// Ported from `ClaudeModelManifest.ts` (`decodeClaudeProfileAdapter`): the
/// `claudeCode` key is optional and decodes to an empty profile when absent.
fn decode_claude_profile_adapter(adapter: &Value) -> Option<ClaudeCodeProfile> {
    let object = adapter.as_object()?;
    let Some(claude) = object.get("claudeCode") else {
        return Some(ClaudeCodeProfile::default());
    };
    if !claude.is_object() {
        return None;
    }
    let mut profile = ClaudeCodeProfile::default();
    if let Some(map) = claude.get("effortMap") {
        let object = map.as_object()?;
        for (key, value) in object {
            profile.effort_map.insert(
                trimmed_non_empty(&json!(key))?,
                if value.is_null() {
                    None
                } else {
                    Some(trimmed_non_empty(value)?)
                },
            );
        }
    }
    if let Some(map) = claude.get("modelSuffixes") {
        let object = map.as_object()?;
        for (option_id, suffixes) in object {
            trimmed_non_empty(&json!(option_id))?;
            let suffixes = suffixes.as_object()?;
            let mut entries = BTreeMap::new();
            for (choice, suffix) in suffixes {
                entries.insert(
                    trimmed_non_empty(&json!(choice))?,
                    trimmed_non_empty(suffix)?,
                );
            }
            profile
                .model_suffixes
                .insert(option_id.clone(), entries);
        }
    }
    if let Some(map) = claude.get("contextWindowTokens") {
        let object = map.as_object()?;
        for (choice, tokens) in object {
            profile.context_window_tokens.insert(
                trimmed_non_empty(&json!(choice))?,
                tokens.as_u64().or_else(|| tokens.as_f64().map(|n| n as u64))?,
            );
        }
    }
    if let Some(tokens) = claude.get("fixedContextWindowTokens") {
        profile.fixed_context_window_tokens =
            Some(tokens.as_u64().or_else(|| tokens.as_f64().map(|n| n as u64))?);
    }
    Some(profile)
}

fn parse_semver_parts(version: &str) -> Option<(u64, u64, u64, Vec<String>)> {
    let normalized = version
        .trim()
        .split('+')
        .next()
        .unwrap_or("")
        .trim_start_matches('v');
    let (main, prerelease) = match normalized.split_once('-') {
        Some((main, rest)) => (main, rest),
        None => (normalized, ""),
    };
    let mut segments: Vec<&str> = main
        .split('.')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    while segments.len() < 3 {
        segments.push("0");
    }
    if segments.len() != 3 {
        return None;
    }
    let mut parts = [0u64; 3];
    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() || !segment.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        parts[index] = segment.parse().ok()?;
    }
    let prerelease: Vec<String> = prerelease
        .split('.')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    Some((parts[0], parts[1], parts[2], prerelease))
}

/// Ported from `ClaudeModelManifest.ts` (`decodeClaudeModelAdapter`): the
/// `claudeCode` key is optional and decodes to an empty compatibility when absent.
fn decode_claude_model_adapter(adapter: &Value) -> Option<ClaudeCodeCompatibility> {
    let object = adapter.as_object()?;
    let Some(claude) = object.get("claudeCode") else {
        return Some(ClaudeCodeCompatibility::default());
    };
    if !claude.is_object() {
        return None;
    }
    let version = |key: &str| -> Option<Option<String>> {
        match claude.get(key) {
            None => Some(None),
            Some(value) => match trimmed_non_empty(value) {
                Some(version) if parse_semver_parts(&version).is_some() => Some(Some(version)),
                _ => None,
            },
        }
    };
    let min_version = version("minVersion")?;
    let max_version_exclusive = version("maxVersionExclusive")?;
    // The schema requires a minimum version below the exclusive maximum.
    if let (Some(min), Some(max)) = (&min_version, &max_version_exclusive)
        && compare_semver_versions(min, max) >= 0
    {
        return None;
    }
    Some(ClaudeCodeCompatibility {
        min_version,
        max_version_exclusive,
    })
}

/// Ported from `packages/shared/src/semver.ts` (`parseSemver` /
/// `compareSemverVersions`): prerelease identifiers compare numerically when both
/// are numeric, numerics sort before strings, and a version without a prerelease
/// sorts after one with it.
fn compare_prerelease_identifier(left: &str, right: &str) -> i32 {
    let numeric = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    match (numeric(left), numeric(right)) {
        (true, true) => left
            .parse::<u64>()
            .unwrap_or(u64::MAX)
            .cmp(&right.parse().unwrap_or(u64::MAX)) as i32,
        (true, false) => -1,
        (false, true) => 1,
        (false, false) => {
            if left == right {
                0
            } else if left < right {
                -1
            } else {
                1
            }
        }
    }
}

pub fn compare_semver_versions(left: &str, right: &str) -> i32 {
    let Some((lm, ln, lp, lpre)) = parse_semver_parts(left) else {
        return 1;
    };
    let Some((rm, rn, rp, rpre)) = parse_semver_parts(right) else {
        return -1;
    };
    for (l, r) in [(lm, rm), (ln, rn), (lp, rp)] {
        if l != r {
            return if l < r { -1 } else { 1 };
        }
    }
    match (lpre.is_empty(), rpre.is_empty()) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => -1,
        (false, false) => {
            let len = lpre.len().min(rpre.len());
            for index in 0..len {
                let order = compare_prerelease_identifier(&lpre[index], &rpre[index]);
                if order != 0 {
                    return order;
                }
            }
            lpre.len().cmp(&rpre.len()) as i32
        }
    }
}

/// `Date.parse` for the manifest's `releasedAt` values: `YYYY-MM-DD` or an ISO
/// timestamp, to epoch milliseconds. Anything else is not a release date.
fn parse_release_ms(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    let date = trimmed.split(['T', ' ']).next()?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days since the Unix epoch for a civil date (Howard Hinnant's algorithm).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9).rem_euclid(12);
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    if days < 0 {
        return None;
    }
    let mut ms = days as u64 * 86_400_000;
    if trimmed.len() > date.len() {
        let time = trimmed[date.len()..].trim_start_matches(['T', ' ']);
        let time = time.trim_end_matches('Z').split(['+', '-']).next().unwrap_or("");
        let mut segments = time.split(':');
        if let (Some(h), Some(m)) = (
            segments.next().and_then(|s| s.parse::<u64>().ok()),
            segments.next().and_then(|s| s.parse::<u64>().ok()),
        ) {
            let s = segments
                .next()
                .and_then(|s| s.split('.').next()?.parse::<u64>().ok())
                .unwrap_or(0);
            ms += h * 3_600_000 + m * 60_000 + s * 1000;
        }
    }
    Some(ms)
}

/// One `models[]` row joined to its profile, as `resolveProviderCatalog` yields it.
struct ProviderCatalogModel {
    slug: String,
    name: String,
    short_name: Option<String>,
    sub_provider: Option<String>,
    aliases: Vec<String>,
    badge: Option<String>,
    status: String,
    capabilities: ModelCapabilities,
    adapter: Value,
    profile_adapter: Value,
}

/// KSG `ModelCatalog.ProviderCatalog`.
struct ProviderCatalog {
    models: Vec<ProviderCatalogModel>,
    default_chat_model: Option<String>,
}

/// Ported from `apps/server/src/provider/ModelManifest.ts` (`resolveProviderCatalog`),
/// specialized to `providers.claudeAgent`. Returns `None` on any schema or
/// reference violation, matching KSG's `null`.
fn resolve_claude_provider_catalog(manifest: &Value, now_ms: u64) -> Option<ProviderCatalog> {
    let catalog = manifest["providers"]["claudeAgent"].as_object()?;
    let profiles = catalog["profiles"].as_object()?;
    let entries = catalog["models"].as_array()?;

    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for entry in entries {
        let slug = trimmed_non_empty(&entry["slug"])?;
        if !seen.insert(slug.clone()) {
            return None;
        }
        let name = trimmed_non_empty(&entry["name"])?;
        let status = entry["status"].as_str()?;
        if status != "current" && status != "legacy" {
            return None;
        }
        let profile = match entry.get("profile") {
            None => None,
            Some(profile) => {
                let key = trimmed_non_empty(profile)?;
                Some(profiles.get(&key)?)
            }
        };
        let capabilities: ModelCapabilities = profile
            .and_then(|p| serde_json::from_value(p["capabilities"].clone()).ok())
            .unwrap_or_default();
        let released_ms = entry
            .get("releasedAt")
            .and_then(|v| v.as_str())
            .and_then(parse_release_ms);
        let badge = (status == "current"
            && released_ms.is_some_and(|ms| now_ms >= ms && now_ms - ms < MODEL_NEW_BADGE_DURATION_MS))
        .then(|| "new".to_owned());
        let aliases = entry["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .map(trimmed_non_empty)
            .collect::<Option<Vec<_>>>()?;
        models.push(ProviderCatalogModel {
            slug,
            name,
            short_name: entry
                .get("shortName")
                .and_then(|v| trimmed_non_empty(v)),
            sub_provider: entry
                .get("subProvider")
                .and_then(|v| trimmed_non_empty(v)),
            aliases,
            badge,
            status: status.to_owned(),
            capabilities,
            adapter: entry.get("adapter").cloned().unwrap_or_else(|| json!({})),
            profile_adapter: profile
                .and_then(|p| p.get("adapter").cloned())
                .unwrap_or_else(|| json!({})),
        });
    }
    let default_chat_model = catalog
        .get("defaults")
        .and_then(|d| d.get("chat"))
        .and_then(trimmed_non_empty);
    if let Some(default) = &default_chat_model
        && !seen.contains(default)
    {
        return None;
    }
    Some(ProviderCatalog {
        models,
        default_chat_model,
    })
}

/// Ported from `packages/provider-core/src/server/ModelCatalog.ts` (`catalogServerModel`).
fn catalog_server_model(
    catalog: &ProviderCatalog,
    entry: &ProviderCatalogModel,
) -> ClaudeModelRow {
    ClaudeModelRow {
        slug: entry.slug.clone(),
        name: entry.name.clone(),
        short_name: entry.short_name.clone(),
        sub_provider: entry.sub_provider.clone(),
        aliases: entry.aliases.clone(),
        badge: entry.badge.clone(),
        is_default: catalog.default_chat_model.as_deref() == Some(entry.slug.as_str()),
        is_custom: false,
        is_legacy: entry.status == "legacy",
        capabilities: entry.capabilities.clone(),
    }
}

/// Ported from `ClaudeModelCatalog.ts` (`tryResolveClaudeModelCatalog`).
fn try_resolve_claude_model_catalog(resolved: Option<ProviderCatalog>) -> Option<ClaudeModelCatalog> {
    let resolved = resolved?;
    let mut models = Vec::new();
    for entry in &resolved.models {
        let profile = decode_claude_profile_adapter(&entry.profile_adapter)?;
        let adapter = decode_claude_model_adapter(&entry.adapter)?;
        let mut model = catalog_server_model(&resolved, entry);
        model.capabilities = with_ultracode_toggle(model.capabilities);
        models.push(ClaudeCatalogModel {
            model,
            runtime: profile,
            compatibility: adapter,
            api_model: None,
        });
    }
    Some(ClaudeModelCatalog { models })
}

static BUNDLED_MANIFEST: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(MODEL_MANIFEST_JSON).unwrap_or(Value::Null));

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeModelCatalog`): resolves the
/// supplied manifest, falling back to the bundled manifest, then to an empty catalog.
pub fn resolve_claude_model_catalog(manifest: &Value, now_ms: u64) -> ClaudeModelCatalog {
    try_resolve_claude_model_catalog(resolve_claude_provider_catalog(manifest, now_ms))
        .or_else(|| {
            try_resolve_claude_model_catalog(resolve_claude_provider_catalog(
                &BUNDLED_MANIFEST,
                now_ms,
            ))
        })
        .unwrap_or_default()
}

/// Resolve the fallback on each read so its new-model badges can expire.
pub fn bundled_claude_model_catalog(now_ms: u64) -> ClaudeModelCatalog {
    resolve_claude_model_catalog(&BUNDLED_MANIFEST, now_ms)
}

pub const CLAUDE_EFFORT_LABELS: [(&str, &str); 5] = [
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "Extra High"),
    ("max", "Max"),
];

fn claude_effort_label(id: &str) -> &str {
    CLAUDE_EFFORT_LABELS
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, label)| *label)
        .unwrap_or(id)
}

/// Ported from `ClaudeModelCatalog.ts` (`stripClaudeContextWindowSuffix`).
pub fn strip_claude_context_window_suffix(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.ends_with(']')
        && let Some(open) = trimmed.rfind('[')
    {
        return trimmed[..open].trim().to_owned();
    }
    trimmed.to_owned()
}

/// Ported from `ClaudeModelCatalog.ts` (`claudeReportedModels`), without the
/// `customModels` argument (Shina has no custom-model setting).
pub fn claude_reported_models(
    catalog: &ClaudeModelCatalog,
    reported: &[ClaudeReportedModel],
) -> Vec<ClaudeModelRow> {
    if reported.is_empty() {
        return Vec::new();
    }
    let mut known: HashSet<String> = catalog
        .models
        .iter()
        .flat_map(|entry| {
            std::iter::once(entry.model.slug.clone()).chain(entry.model.aliases.iter().cloned())
        })
        .map(|id| strip_claude_context_window_suffix(&id).to_lowercase())
        .collect();
    let mut models = Vec::new();
    for info in reported {
        let slug = info.value.trim();
        // `default` is Claude Code's pointer at its own default model, not a model.
        if slug.is_empty() || slug == "default" {
            continue;
        }
        let ids: Vec<String> = [slug, info.resolved_model.as_deref().unwrap_or("")]
            .into_iter()
            .map(|id| strip_claude_context_window_suffix(id).to_lowercase())
            .filter(|id| !id.is_empty())
            .collect();
        if ids.iter().any(|id| known.contains(id)) {
            continue;
        }
        for id in ids {
            known.insert(id);
        }
        let efforts: Vec<String> = if info.supports_effort {
            info.supported_effort_levels.clone()
        } else {
            Vec::new()
        };
        let mut options = vec![ProviderOptionChoice {
            id: "default".into(),
            label: "Default".into(),
            description: None,
            is_default: true,
        }];
        options.extend(efforts.iter().map(|id| ProviderOptionChoice {
            id: id.clone(),
            label: claude_effort_label(id).into(),
            description: None,
            is_default: false,
        }));
        models.push(ClaudeModelRow {
            slug: slug.to_owned(),
            name: {
                let name = info.display_name.trim();
                if name.is_empty() {
                    slug.to_owned()
                } else {
                    name.to_owned()
                }
            },
            short_name: None,
            sub_provider: None,
            aliases: Vec::new(),
            badge: None,
            is_default: false,
            is_custom: false,
            is_legacy: false,
            capabilities: ModelCapabilities {
                option_descriptors: if efforts.is_empty() {
                    Vec::new()
                } else {
                    vec![ProviderOptionDescriptor::Select {
                        id: "effort".into(),
                        label: "Reasoning".into(),
                        description: None,
                        options,
                        current_value: None,
                        prompt_injected_values: Vec::new(),
                        role: None,
                    }]
                },
            },
        });
    }
    models
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeCatalogModel`).
pub fn resolve_claude_catalog_model<'a>(
    catalog: &'a ClaudeModelCatalog,
    slug_or_alias: &str,
) -> Option<&'a ClaudeCatalogModel> {
    let value = slug_or_alias.trim();
    if value.is_empty() {
        return None;
    }
    catalog
        .models
        .iter()
        .find(|entry| entry.model.slug == value)
        .or_else(|| {
            let lower = value.to_lowercase();
            catalog
                .models
                .iter()
                .find(|entry| {
                    entry
                        .model
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase() == lower)
                })
        })
}

/// Ported from `ClaudeModelCatalog.ts` (`stripClaudeBackendDecoration`): cloud
/// backends decorate first-party ids: Bedrock adds a region prefix and a version
/// ("us.anthropic.claude-opus-4-6-v1"), Vertex adds "@<date>".
fn strip_claude_backend_decoration(wire_model: &str) -> String {
    static BACKEND_PREFIX: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^(?:[a-z-]+\.)?anthropic\.").unwrap());
    static VERSION_SUFFIX: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"-v\d+(?::\d+)?$").unwrap());
    static AT_SUFFIX: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"@[^@]*$").unwrap());
    let model = BACKEND_PREFIX.replace(wire_model, "");
    let model = VERSION_SUFFIX.replace(&model, "");
    AT_SUFFIX.replace(&model, "").into_owned()
}

/// Ported from `ClaudeModelCatalog.ts` (`withClaudeCodeModelRoutes`): on Bedrock,
/// Vertex and similar backends Claude Code does not accept the first-party slug;
/// its model list names the id that does work.
pub fn with_claude_code_model_routes(
    catalog: &ClaudeModelCatalog,
    reported_models: &[ClaudeReportedModel],
) -> ClaudeModelCatalog {
    let mut api_models: HashMap<String, String> = HashMap::new();
    for reported in reported_models {
        if reported.value == "default" {
            continue;
        }
        let wire_model = strip_claude_context_window_suffix(
            reported.resolved_model.as_deref().unwrap_or(&reported.value),
        );
        let first_party_model = strip_claude_backend_decoration(&wire_model);
        if first_party_model == wire_model {
            continue;
        }
        let Some(entry) = resolve_claude_catalog_model(catalog, &first_party_model) else {
            continue;
        };
        api_models
            .entry(entry.model.slug.clone())
            .or_insert_with(|| strip_claude_context_window_suffix(&reported.value));
    }
    if api_models.is_empty() {
        return ClaudeModelCatalog {
            models: catalog.models.clone(),
        };
    }
    ClaudeModelCatalog {
        models: catalog
            .models
            .iter()
            .map(|entry| {
                let mut entry = entry.clone();
                if let Some(api_model) = api_models.get(&entry.model.slug) {
                    entry.api_model = Some(api_model.clone());
                }
                entry
            })
            .collect(),
    }
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeModelSlug`).
pub fn resolve_claude_model_slug(catalog: &ClaudeModelCatalog, slug_or_alias: &str) -> String {
    resolve_claude_catalog_model(catalog, slug_or_alias)
        .map(|entry| entry.model.slug.clone())
        .unwrap_or_else(|| slug_or_alias.to_owned())
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeReportedModelSlug`): Claude
/// reports dated IDs, such as `claude-opus-4-5-20251101`, that the catalog may not list.
pub fn resolve_claude_reported_model_slug(catalog: &ClaudeModelCatalog, reported: &str) -> String {
    if let Some(entry) = resolve_claude_catalog_model(catalog, reported) {
        return entry.model.slug.clone();
    }
    static DATED_SUFFIX: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"-\d{8}$").unwrap());
    resolve_claude_catalog_model(catalog, &DATED_SUFFIX.replace(reported, ""))
        .map(|entry| entry.model.slug.clone())
        .unwrap_or_else(|| reported.to_owned())
}

/// Ported from `ClaudeModelCatalog.ts` (`getClaudeCatalogModelCapabilities`).
pub fn get_claude_catalog_model_capabilities(
    catalog: &ClaudeModelCatalog,
    slug_or_alias: &str,
) -> ModelCapabilities {
    resolve_claude_catalog_model(catalog, slug_or_alias)
        .map(|entry| entry.model.capabilities.clone())
        .unwrap_or_default()
}

/// Ported from `ClaudeModelCatalog.ts` (`isVersionSupported`).
fn is_version_supported(compatibility: &ClaudeCodeCompatibility, version: Option<&str>) -> bool {
    if compatibility.min_version.is_none() && compatibility.max_version_exclusive.is_none() {
        return true;
    }
    let Some(version) = version else {
        return false;
    };
    if let Some(min) = &compatibility.min_version
        && compare_semver_versions(version, min) < 0
    {
        return false;
    }
    !compatibility
        .max_version_exclusive
        .as_ref()
        .is_some_and(|max| compare_semver_versions(version, max) >= 0)
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeModelsForVersion`).
pub fn resolve_claude_models_for_version(
    catalog: &ClaudeModelCatalog,
    version: Option<&str>,
) -> Vec<ClaudeModelRow> {
    catalog
        .models
        .iter()
        .filter(|entry| is_version_supported(&entry.compatibility, version))
        .map(|entry| entry.model.clone())
        .collect()
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeUpdateRequiredModels`).
pub fn resolve_claude_update_required_models(
    catalog: &ClaudeModelCatalog,
    version: Option<&str>,
) -> Vec<crate::model_picker::UpdateRequiredModel> {
    let Some(version) = version else {
        return Vec::new();
    };
    catalog
        .models
        .iter()
        .filter_map(|entry| {
            let min_version = entry.compatibility.min_version.as_ref()?;
            if entry.model.is_legacy
                || compare_semver_versions(version, min_version) >= 0
            {
                return None;
            }
            Some(crate::model_picker::UpdateRequiredModel {
                slug: entry.model.slug.clone(),
                name: entry.model.name.clone(),
                badge: entry.model.badge.clone(),
                min_version: min_version.clone(),
            })
        })
        .collect()
}

/// Ported from `ClaudeModelCatalog.ts` (`formatClaudeVersionUpgradeMessage`).
pub fn format_claude_version_upgrade_message(
    catalog: &ClaudeModelCatalog,
    version: Option<&str>,
) -> Option<String> {
    let unavailable = catalog
        .models
        .iter()
        .filter(|entry| {
            entry.compatibility.min_version.as_ref().is_some_and(|min| {
                version.is_none_or(|v| compare_semver_versions(v, min) < 0)
            })
        })
        .min_by(|left, right| {
            compare_semver_versions(
                left.compatibility.min_version.as_deref().unwrap_or(""),
                right.compatibility.min_version.as_deref().unwrap_or(""),
            )
            .cmp(&0)
        });
    let min = unavailable?.compatibility.min_version.as_ref()?;
    let label = version.map_or_else(|| "the installed version".into(), |v| format!("v{v}"));
    Some(format!(
        "Claude Code {label} is too old for {}. Upgrade to v{min} or newer to access it.",
        unavailable?.model.name
    ))
}

/// Claude Code's effort levels, lowest first.
pub const CLAUDE_EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

fn claude_effort_rank(level: Option<&str>) -> i32 {
    level
        .and_then(|level| CLAUDE_EFFORT_LEVELS.iter().position(|l| *l == level))
        .map(|rank| rank as i32)
        .unwrap_or(-1)
}

/// One settings source's effort caps, with `modelSettings` keys as the user wrote them.
/// Ported from `ClaudeModelCatalog.ts` (`ClaudeEffortCapSource`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClaudeEffortCapSource {
    pub max_effort_level: Option<String>,
    pub model_max_effort_levels: BTreeMap<String, String>,
}

const CLAUDE_1M_SUFFIX: &str = "[1m]";
const CLAUDE_CODE_FAMILY_ALIASES: [&str; 4] = ["opus", "sonnet", "haiku", "fable"];

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeModelSettingsKey`): names the
/// model a `modelSettings` key caps the way Claude Code does, narrower than the
/// catalog's aliases.
fn resolve_claude_model_settings_key(
    catalog: &ClaudeModelCatalog,
    key: &str,
) -> Option<String> {
    let lowered = key.trim().to_lowercase();
    let unsuffixed = lowered
        .strip_suffix(CLAUDE_1M_SUFFIX)
        .unwrap_or(&lowered);
    if CLAUDE_CODE_FAMILY_ALIASES.contains(&unsuffixed) {
        return catalog
            .models
            .iter()
            .find(|entry| {
                !entry.model.is_custom && entry.model.aliases.iter().any(|a| a == unsuffixed)
            })
            .map(|entry| entry.model.slug.clone());
    }
    let mut best: Option<String> = None;
    for entry in &catalog.models {
        if !entry.model.is_custom
            && lowered.contains(&entry.model.slug)
            && entry.model.slug.len() > best.as_ref().map_or(0, String::len)
        {
            best = Some(entry.model.slug.clone());
        }
    }
    best
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeModelMaxEffortLevels`): keys
/// `modelSettings` caps by canonical slug, keeping the lowest cap when several keys
/// name one model.
fn resolve_claude_model_max_effort_levels(
    catalog: &ClaudeModelCatalog,
    levels: &BTreeMap<String, String>,
) -> HashMap<String, String> {
    let mut resolved = HashMap::new();
    for (key, level) in levels {
        let Some(slug) = resolve_claude_model_settings_key(catalog, key) else {
            continue;
        };
        let lower = claude_effort_rank(Some(level.as_str()));
        let current = resolved.get(&slug);
        if current.is_none() || lower < claude_effort_rank(current.map(String::as_str)) {
            resolved.insert(slug, level.clone());
        }
    }
    resolved
}

/// Ported from `ClaudeModelCatalog.ts` (`normalizeClaudeCatalogEffort`).
pub fn normalize_claude_catalog_effort(
    catalog: &ClaudeModelCatalog,
    effort: Option<&str>,
    model: &str,
) -> Option<String> {
    let effort = effort?;
    let effort_map = &resolve_claude_catalog_model(catalog, model)?.runtime.effort_map;
    if !effort_map.contains_key(effort) {
        return Some(effort.to_owned());
    }
    effort_map.get(effort)?.clone()
}

/// Ported from `ClaudeModelCatalog.ts` (`capClaudeCatalogEffort`): Claude Code
/// clamps efforts above `maxEffortLevel` to the model's highest level under it, so
/// such options are relabelled, not removed, keeping stored selections valid.
pub fn cap_claude_catalog_effort(
    catalog: &ClaudeModelCatalog,
    models: Vec<ClaudeModelRow>,
    sources: &[ClaudeEffortCapSource],
) -> Vec<ClaudeModelRow> {
    if sources.is_empty() {
        return models;
    }
    let resolved: Vec<(Option<String>, HashMap<String, String>)> = sources
        .iter()
        .map(|source| {
            (
                source.max_effort_level.clone(),
                resolve_claude_model_max_effort_levels(catalog, &source.model_max_effort_levels),
            )
        })
        .collect();
    models
        .into_iter()
        .map(|mut model| {
            // Custom model labels are user-authored, so they are left alone.
            if model.is_custom {
                return model;
            }
            let model_max = resolved
                .iter()
                .fold(None::<String>, |lowest, (max_level, per_model)| {
                    let level = per_model
                        .get(&model.slug)
                        .or(max_level.as_ref());
                    match level {
                        Some(level)
                            if level != "max"
                                && (lowest.is_none()
                                    || claude_effort_rank(Some(level.as_str()))
                                        < claude_effort_rank(lowest.as_deref())) =>
                        {
                            Some(level.clone())
                        }
                        _ => lowest,
                    }
                });
            let Some(model_max) = model_max else {
                return model;
            };
            let cap_rank = claude_effort_rank(Some(model_max.as_str()));
            model.capabilities.option_descriptors = model
                .capabilities
                .option_descriptors
                .into_iter()
                .map(|descriptor| {
                    let ProviderOptionDescriptor::Select {
                        id,
                        label,
                        description,
                        options,
                        current_value,
                        prompt_injected_values,
                        role,
                    } = descriptor
                    else {
                        return descriptor;
                    };
                    if id != "effort" {
                        return ProviderOptionDescriptor::Select {
                            id,
                            label,
                            description,
                            options,
                            current_value,
                            prompt_injected_values,
                            role,
                        };
                    }
                    let cap = options
                        .iter()
                        .fold(None::<(i32, String)>, |best, option| {
                            let rank = claude_effort_rank(Some(option.id.as_str()));
                            if rank >= 0
                                && rank <= cap_rank
                                && rank > best.as_ref().map(|(r, _)| *r).unwrap_or(-1)
                            {
                                Some((rank, option.label.clone()))
                            } else {
                                best
                            }
                        });
                    let Some((cap_rank, cap_label)) = cap else {
                        return ProviderOptionDescriptor::Select {
                            id,
                            label,
                            description,
                            options,
                            current_value,
                            prompt_injected_values,
                            role,
                        };
                    };
                    let options = options
                        .into_iter()
                        .map(|mut option| {
                            let id_rank = claude_effort_rank(Some(option.id.as_str()));
                            let sent_rank = claude_effort_rank(
                                normalize_claude_catalog_effort(
                                    catalog,
                                    Some(option.id.as_str()),
                                    &model.slug,
                                )
                                .as_deref(),
                            );
                            let displayed_rank = if id_rank >= 0 { id_rank } else { sent_rank };
                            let runs_rank = if sent_rank >= 0 { sent_rank } else { id_rank };
                            if displayed_rank > cap_rank && runs_rank > cap_rank {
                                option.label = format!("{} (capped to {})", option.label, cap_label);
                            }
                            option
                        })
                        .collect();
                    ProviderOptionDescriptor::Select {
                        id,
                        label,
                        description,
                        options,
                        current_value,
                        prompt_injected_values,
                        role,
                    }
                })
                .collect();
            model
        })
        .collect()
}

fn effort_selection(raw: &str) -> Vec<ProviderOptionSelection> {
    vec![ProviderOptionSelection {
        id: "effort".into(),
        value: OptionValue::Str(raw.to_owned()),
    }]
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeCatalogEffort`).
pub fn resolve_claude_catalog_effort(
    catalog: &ClaudeModelCatalog,
    model: &str,
    raw: Option<&str>,
) -> Option<String> {
    if resolve_claude_catalog_model(catalog, model).is_none() {
        let effort = raw?.trim();
        return CLAUDE_EFFORT_LEVELS
            .contains(&effort)
            .then(|| effort.to_owned());
    }
    let caps = get_claude_catalog_model_capabilities(catalog, model);
    let selections = raw.map_or_else(Vec::new, effort_selection);
    let descriptors = get_provider_option_descriptors(&caps, &selections);
    let descriptor = descriptors.iter().find(|d| d.id() == "effort")?;
    get_provider_option_current_value(descriptor, None, None)
        .and_then(|value| value.as_str().map(str::to_owned))
}

fn model_selection_string_option<'a>(
    model_selection: Option<&'a ModelSelection>,
    id: &str,
) -> Option<&'a str> {
    model_selection?
        .options
        .iter()
        .find(|o| o.id == id)
        .and_then(|o| o.value.as_str())
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeCatalogContextWindow`).
fn resolve_claude_catalog_context_window(
    catalog: &ClaudeModelCatalog,
    model_selection: Option<&ModelSelection>,
) -> Option<String> {
    let caps =
        get_claude_catalog_model_capabilities(catalog, model_selection.map_or("", |s| &s.model));
    let raw = model_selection_string_option(model_selection, "contextWindow");
    let selections = raw.map_or_else(Vec::new, |raw| {
        vec![ProviderOptionSelection {
            id: "contextWindow".into(),
            value: OptionValue::Str(raw.to_owned()),
        }]
    });
    let descriptors = get_provider_option_descriptors(&caps, &selections);
    let descriptor = descriptors.iter().find(|d| d.id() == "contextWindow")?;
    get_provider_option_current_value(descriptor, None, None)
        .and_then(|value| value.as_str().map(str::to_owned))
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeCatalogApiModelId`): the id
/// sent to Claude Code: `apiModel` when present, else the manifest slug, never the
/// display name. Runtime suffixes apply on top.
pub fn resolve_claude_catalog_api_model_id(
    catalog: &ClaudeModelCatalog,
    model_selection: &ModelSelection,
) -> String {
    let entry = resolve_claude_catalog_model(catalog, &model_selection.model);
    let slug = entry
        .and_then(|e| e.api_model.clone())
        .or_else(|| entry.map(|e| e.model.slug.clone()))
        .unwrap_or_else(|| model_selection.model.clone());
    let caps = entry.map_or_else(ModelCapabilities::default, |e| e.model.capabilities.clone());
    let descriptors = get_provider_option_descriptors(&caps, &model_selection.options);
    for (option_id, suffixes) in entry.map_or(&BTreeMap::new(), |e| &e.runtime.model_suffixes) {
        let value = descriptors
            .iter()
            .find(|d| d.id() == option_id)
            .and_then(|d| get_provider_option_current_value(d, None, None))
            .and_then(|v| v.as_str().map(str::to_owned));
        if let Some(value) = value
            && let Some(suffix) = suffixes.get(&value)
        {
            return format!("{slug}{suffix}");
        }
    }
    slug
}

/// Ported from `ClaudeModelCatalog.ts` (`resolveClaudeCatalogContextWindowTokens`).
pub fn resolve_claude_catalog_context_window_tokens(
    catalog: &ClaudeModelCatalog,
    model_selection: Option<&ModelSelection>,
) -> Option<u64> {
    let entry = resolve_claude_catalog_model(catalog, model_selection?.model.as_str())?;
    if let Some(tokens) = entry.runtime.fixed_context_window_tokens {
        return Some(tokens);
    }
    let context_window = resolve_claude_catalog_context_window(catalog, model_selection)?;
    entry
        .runtime
        .context_window_tokens
        .get(&context_window)
        .copied()
}

/// The selectable ids (excluding prompt-injected values and the `default` alias)
/// that Shina reports as a model's thinking levels.
pub fn claude_thinking_levels(capabilities: &ModelCapabilities) -> Vec<String> {
    capabilities
        .option_descriptors
        .iter()
        .find(|d| d.id() == "effort")
        .map(|d| {
            selectable_provider_option_choices(d)
                .into_iter()
                .map(|c| c.id)
                .filter(|id| id != "default")
                .collect()
        })
        .unwrap_or_default()
}

// Env names that pick the backend, its region, or the model a family alias resolves to.
// Ported from `ClaudeCodeModelRoutes.ts` (`MODEL_SETTINGS_ENV_PREFIXES`).
const MODEL_SETTINGS_ENV_PREFIXES: [&str; 5] = [
    "ANTHROPIC_",
    "CLAUDE_CODE_USE_",
    "AWS_",
    "CLOUD_ML_",
    "VERTEX_",
];

/// Ported from `ClaudeCodeModelRoutes.ts` (`claudeModelSettingsKey`): equal keys
/// mean Claude Code resolves model ids the same way under both settings.
pub fn claude_model_settings_key(settings: &Value) -> String {
    let sorted_entries = |record: &Value| -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = record
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| {
                Some((
                    key.clone(),
                    value.as_str().unwrap_or_default().to_owned(),
                ))
            })
            .collect();
        entries.sort();
        entries
    };
    json!([
        settings["availableModels"].clone(),
        sorted_entries(&settings["modelOverrides"]),
        sorted_entries(&settings["env"])
            .into_iter()
            .filter(|(name, _)| {
                MODEL_SETTINGS_ENV_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            })
            .collect::<Vec<_>>(),
    ])
    .to_string()
}

/// `claudeModelSettingsKey({})`: the key when no settings carry model context.
pub fn no_model_settings_key() -> String {
    claude_model_settings_key(&json!({}))
}

/// Ported from `ClaudeProvider.ts` (`parseClaudeMaxEffortLevel`).
fn parse_claude_max_effort_level(value: &Value) -> Option<String> {
    let level = value.as_str()?;
    CLAUDE_EFFORT_LEVELS
        .contains(&level)
        .then(|| level.to_owned())
}

/// Ported from `ClaudeProvider.ts` (`parseClaudeModelMaxEffortLevels`).
fn parse_claude_model_max_effort_levels(model_settings: &Value) -> BTreeMap<String, String> {
    let mut levels = BTreeMap::new();
    if let Some(object) = model_settings.as_object() {
        for (model, settings) in object {
            if let Some(level) = parse_claude_max_effort_level(&settings["maxEffortLevel"]) {
                levels.insert(model.clone(), level);
            }
        }
    }
    levels
}

/// Ported from `ClaudeProvider.ts` (`parseClaudeEffortCapSource`).
fn parse_claude_effort_cap_source(settings: &Value) -> Option<ClaudeEffortCapSource> {
    if !settings.is_object() {
        return None;
    }
    let level = parse_claude_max_effort_level(&settings["maxEffortLevel"]);
    let model_max_effort_levels = parse_claude_model_max_effort_levels(&settings["modelSettings"]);
    (level.is_some() || !model_max_effort_levels.is_empty()).then_some(ClaudeEffortCapSource {
        max_effort_level: level,
        model_max_effort_levels,
    })
}

/// Ported from `ClaudeProvider.ts` (`parseClaudeEffortCapSources`): `sources` holds
/// each settings file's raw settings; CLIs that report only the merged `effective`
/// settings are read as a single source.
pub fn parse_claude_effort_cap_sources(settings: &Value) -> Vec<ClaudeEffortCapSource> {
    if let Some(sources) = settings["sources"].as_array() {
        return sources
            .iter()
            .filter_map(|source| parse_claude_effort_cap_source(&source["settings"]))
            .collect();
    }
    parse_claude_effort_cap_source(&settings["effective"])
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(ids: &[(&str, &str, bool)]) -> ProviderOptionDescriptor {
        ProviderOptionDescriptor::Select {
            id: "effort".into(),
            label: "Reasoning".into(),
            description: None,
            options: ids
                .iter()
                .map(|(id, label, is_default)| ProviderOptionChoice {
                    id: (*id).into(),
                    label: (*label).into(),
                    description: None,
                    is_default: *is_default,
                })
                .collect(),
            current_value: None,
            prompt_injected_values: vec![],
            role: None,
        }
    }

    fn row(slug: &str, name: &str, aliases: &[&str], descriptors: Vec<ProviderOptionDescriptor>) -> ClaudeModelRow {
        ClaudeModelRow {
            slug: slug.into(),
            name: name.into(),
            short_name: None,
            sub_provider: None,
            aliases: aliases.iter().map(|a| (*a).into()).collect(),
            badge: None,
            is_default: false,
            is_custom: false,
            is_legacy: false,
            capabilities: ModelCapabilities {
                option_descriptors: descriptors,
            },
        }
    }

    /// The synthetic manifest from `ClaudeModelCatalog.test.ts`.
    fn synthetic_manifest() -> Value {
        json!({
            "version": 1,
            "currentModels": {},
            "providers": {
                "claudeAgent": {
                    "profiles": {
                        "synthetic": {
                            "capabilities": {
                                "optionDescriptors": [
                                    {
                                        "id": "effort",
                                        "label": "Reasoning",
                                        "type": "select",
                                        "options": [{"id":"extreme","label":"Extreme","isDefault":true}],
                                    },
                                    {
                                        "id": "contextWindow",
                                        "label": "Context Window",
                                        "type": "select",
                                        "options": [
                                            {"id":"large","label":"Large","isDefault":true},
                                            {"id":"small","label":"Small"},
                                        ],
                                    },
                                ],
                            },
                            "adapter": {
                                "claudeCode": {
                                    "effortMap": {"extreme":"high"},
                                    "modelSuffixes": {"contextWindow":{"large":"[large]"}},
                                },
                            },
                        },
                    },
                    "models": [
                        {
                            "slug":"claude-synthetic-next",
                            "name":"Claude Synthetic Next",
                            "aliases":["synthetic"],
                            "status":"current",
                            "profile":"synthetic",
                            "adapter":{"claudeCode":{"minVersion":"3.2.0"}},
                        },
                    ],
                },
            },
        })
    }

    fn catalog_from_manifest(data: &Value) -> ClaudeModelCatalog {
        try_resolve_claude_model_catalog(resolve_claude_provider_catalog(data, 0))
            .unwrap_or_default()
    }

    fn selection(model: &str) -> ModelSelection {
        ModelSelection {
            instance_id: "claudeAgent".into(),
            model: model.into(),
            options: vec![],
        }
    }

    fn reported(value: &str, resolved: &str) -> ClaudeReportedModel {
        ClaudeReportedModel {
            value: value.into(),
            resolved_model: Some(resolved.into()),
            ..Default::default()
        }
    }

    #[test]
    fn capacity_resolves_from_selected_options_and_fixed_windows() {
        let mut data = synthetic_manifest();
        let profiles = &mut data["providers"]["claudeAgent"]["profiles"];
        profiles["fixed"] = json!({
            "capabilities": {"optionDescriptors": []},
            "adapter": {"claudeCode": {"fixedContextWindowTokens": 64_000}},
        });
        profiles["synthetic"]["adapter"] = json!({
            "claudeCode": {"contextWindowTokens": {"large": 1_000_000, "small": 32_000}},
        });
        let models = data["providers"]["claudeAgent"]["models"]
            .as_array_mut()
            .unwrap();
        models.push(json!({"slug":"fixed","name":"Fixed","status":"current","profile":"fixed"}));
        let catalog = catalog_from_manifest(&data);

        let base = selection("synthetic");
        assert_eq!(
            resolve_claude_catalog_context_window_tokens(&catalog, Some(&base)),
            Some(1_000_000)
        );
        let mut small = base.clone();
        small.options.push(ProviderOptionSelection {
            id: "contextWindow".into(),
            value: OptionValue::Str("small".into()),
        });
        assert_eq!(
            resolve_claude_catalog_context_window_tokens(&catalog, Some(&small)),
            Some(32_000)
        );
        assert_eq!(
            resolve_claude_catalog_context_window_tokens(&catalog, Some(&selection("fixed"))),
            Some(64_000)
        );
        assert_eq!(
            resolve_claude_catalog_context_window_tokens(&catalog, Some(&selection("custom"))),
            None
        );
    }

    #[test]
    fn version_filters_models_and_builds_update_notices() {
        let catalog = catalog_from_manifest(&synthetic_manifest());
        assert!(resolve_claude_models_for_version(&catalog, Some("3.1.9")).is_empty());
        assert_eq!(
            resolve_claude_models_for_version(&catalog, Some("3.2.0"))
                .iter()
                .map(|m| m.slug.as_str())
                .collect::<Vec<_>>(),
            ["claude-synthetic-next"]
        );
        let required = resolve_claude_update_required_models(&catalog, Some("3.1.9"));
        assert_eq!(required.len(), 1);
        assert_eq!(required[0].slug, "claude-synthetic-next");
        assert_eq!(required[0].min_version, "3.2.0");
        assert!(resolve_claude_update_required_models(&catalog, Some("3.2.0")).is_empty());
        assert_eq!(
            format_claude_version_upgrade_message(&catalog, Some("3.1.9")).as_deref(),
            Some("Claude Code v3.1.9 is too old for Claude Synthetic Next. Upgrade to v3.2.0 or newer to access it.")
        );
        // Claude Code below claude-fable-5-2's minVersion does not list it.
        let bundled = bundled_claude_model_catalog(0);
        assert!(
            !resolve_claude_models_for_version(&bundled, Some("2.1.289"))
                .iter()
                .any(|m| m.slug == "claude-fable-5-2")
        );
        assert!(
            resolve_claude_models_for_version(&bundled, Some("2.1.290"))
                .iter()
                .any(|m| m.slug == "claude-fable-5-2")
        );
    }

    #[test]
    fn resolves_aliases_and_declarative_adapter_mappings() {
        let mut data = synthetic_manifest();
        data["providers"]["claudeAgent"]["models"]
            .as_array_mut()
            .unwrap()
            .insert(
                0,
                json!({
                    "slug":"claude-synthetic-collision",
                    "name":"Claude Synthetic Collision",
                    "aliases":["claude-synthetic-next"],
                    "status":"current",
                }),
            );
        let catalog = catalog_from_manifest(&data);
        assert_eq!(
            resolve_claude_model_slug(&catalog, "synthetic"),
            "claude-synthetic-next"
        );
        assert_eq!(
            resolve_claude_model_slug(&catalog, "claude-synthetic-next"),
            "claude-synthetic-next"
        );
        assert_eq!(
            normalize_claude_catalog_effort(&catalog, Some("extreme"), "synthetic"),
            Some("high".into())
        );
        assert_eq!(
            resolve_claude_catalog_api_model_id(&catalog, &selection("synthetic")),
            "claude-synthetic-next[large]"
        );
    }

    #[test]
    fn reported_models_route_catalog_slugs_to_the_ids_claude_code_accepts() {
        let bundled = bundled_claude_model_catalog(0);
        let api_model = |catalog: &ClaudeModelCatalog, model: &str, options: Vec<(&str, &str)>| {
            let mut sel = selection(model);
            sel.options = options
                .into_iter()
                .map(|(id, value)| ProviderOptionSelection {
                    id: id.into(),
                    value: OptionValue::Str(value.into()),
                })
                .collect();
            resolve_claude_catalog_api_model_id(catalog, &sel)
        };

        // Bedrock with ANTHROPIC_DEFAULT_*_MODEL and an availableModels allowlist of their ids.
        let configured_bedrock = with_claude_code_model_routes(
            &bundled,
            &[
                reported("default", "global.anthropic.claude-opus-5-5"),
                reported("sonnet", "global.anthropic.claude-sonnet-5-5[1m]"),
                reported("opus", "global.anthropic.claude-opus-5-5"),
                reported("haiku", "global.anthropic.claude-haiku-5-5"),
            ],
        );
        assert_eq!(api_model(&configured_bedrock, "claude-haiku-5-5", vec![]), "haiku");
        assert_eq!(api_model(&configured_bedrock, "haiku-5.5", vec![]), "haiku");
        assert_eq!(api_model(&configured_bedrock, "claude-sonnet-5-5", vec![]), "sonnet");
        assert_eq!(api_model(&configured_bedrock, "claude-opus-5-5", vec![]), "opus");
        assert_eq!(
            api_model(&configured_bedrock, "claude-opus-5", vec![]),
            "claude-opus-5"
        );

        // Bedrock with no model settings: Claude Code's `haiku` is Haiku 4.5 there.
        let default_bedrock = with_claude_code_model_routes(
            &bundled,
            &[
                reported(
                    "us.anthropic.claude-opus-4-6-v1",
                    "us.anthropic.claude-opus-4-6-v1",
                ),
                reported(
                    "us.anthropic.claude-opus-4-6-v1[1m]",
                    "us.anthropic.claude-opus-4-6-v1[1m]",
                ),
                reported("haiku", "us.anthropic.claude-haiku-4-5-20251001-v1:0"),
            ],
        );
        assert_eq!(
            api_model(
                &default_bedrock,
                "claude-opus-4-6",
                vec![("contextWindow", "1m")]
            ),
            "us.anthropic.claude-opus-4-6-v1[1m]"
        );
        assert_eq!(
            api_model(
                &default_bedrock,
                "claude-opus-4-6",
                vec![("contextWindow", "200k")]
            ),
            "us.anthropic.claude-opus-4-6-v1"
        );
        assert_eq!(api_model(&default_bedrock, "claude-haiku-4-5", vec![]), "haiku");
        assert_eq!(
            api_model(&default_bedrock, "claude-haiku-5-5", vec![]),
            "claude-haiku-5-5"
        );

        // First-party ids already are what Claude Code runs, even when an alias points elsewhere.
        let first_party = with_claude_code_model_routes(
            &bundled,
            &[
                reported("default", "claude-opus-5-5"),
                reported("opus", "claude-opus-5-5"),
                reported("claude-fable-5-1[1m]", "claude-fable-5-1[1m]"),
                reported("haiku", "claude-haiku-4-5-20251001"),
            ],
        );
        assert!(
            first_party
                .models
                .iter()
                .all(|entry| entry.api_model.is_none())
        );
    }

    #[test]
    fn marks_effort_options_above_the_cap_without_changing_ids_or_defaults() {
        let effort_model = |slug: &str| {
            row(
                slug,
                slug,
                &[],
                vec![descriptor(&[
                    ("medium", "Medium", false),
                    ("high", "High", true),
                    ("xhigh", "Extra High", false),
                    ("max", "Max", false),
                    ("ultracode", "Ultracode", false),
                    ("ultrathink", "Ultrathink", false),
                ])],
            )
        };
        let models = vec![effort_model("claude-plain"), effort_model("claude-remapped")];
        let mut runtime_a = ClaudeCodeProfile::default();
        runtime_a
            .effort_map
            .insert("ultracode".into(), Some("xhigh".into()));
        runtime_a.effort_map.insert("ultrathink".into(), None);
        let mut runtime_b = ClaudeCodeProfile::default();
        runtime_b.effort_map.insert("max".into(), Some("high".into()));
        runtime_b
            .effort_map
            .insert("xhigh".into(), Some("max".into()));
        runtime_b.effort_map.insert("ultrathink".into(), None);
        let catalog = ClaudeModelCatalog {
            models: vec![
                ClaudeCatalogModel {
                    model: models[0].clone(),
                    runtime: runtime_a,
                    compatibility: Default::default(),
                    api_model: None,
                },
                ClaudeCatalogModel {
                    model: models[1].clone(),
                    runtime: runtime_b,
                    compatibility: Default::default(),
                    api_model: None,
                },
            ],
        };
        let labels = |cap: &str| -> Vec<Vec<(String, String)>> {
            cap_claude_catalog_effort(
                &catalog,
                models.clone(),
                &[ClaudeEffortCapSource {
                    max_effort_level: Some(cap.into()),
                    ..Default::default()
                }],
            )
            .into_iter()
            .map(|model| {
                model
                    .capabilities
                    .option_descriptors
                    .iter()
                    .find_map(|d| match d {
                        ProviderOptionDescriptor::Select { options, .. } => Some(
                            options
                                .iter()
                                .map(|o| (o.id.clone(), o.label.clone()))
                                .collect::<Vec<_>>(),
                        ),
                        _ => None,
                    })
                    .unwrap_or_default()
            })
            .collect()
        };
        assert_eq!(
            labels("high"),
            vec![
                vec![
                    ("medium".into(), "Medium".into()),
                    ("high".into(), "High".into()),
                    ("xhigh".into(), "Extra High (capped to High)".into()),
                    ("max".into(), "Max (capped to High)".into()),
                    ("ultracode".into(), "Ultracode (capped to High)".into()),
                    ("ultrathink".into(), "Ultrathink".into()),
                ],
                vec![
                    ("medium".into(), "Medium".into()),
                    ("high".into(), "High".into()),
                    ("xhigh".into(), "Extra High (capped to High)".into()),
                    ("max".into(), "Max".into()),
                    ("ultracode".into(), "Ultracode".into()),
                    ("ultrathink".into(), "Ultrathink".into()),
                ],
            ]
        );
        // An effortMap that sends Extra High as max stays unmarked under an xhigh cap:
        // the CLI runs it at xhigh, which is what the label says.
        assert_eq!(
            labels("xhigh")
                .into_iter()
                .map(|m| m.into_iter().map(|(_, l)| l).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![
                vec!["Medium", "High", "Extra High", "Max (capped to Extra High)", "Ultracode", "Ultrathink"],
                vec!["Medium", "High", "Extra High", "Max", "Ultracode", "Ultrathink"],
            ]
        );

        // A per-model cap applies without a top-level one and leaves other models uncapped.
        let mut per_model = BTreeMap::new();
        per_model.insert("claude-plain".to_owned(), "high".to_owned());
        let capped = cap_claude_catalog_effort(
            &catalog,
            models.clone(),
            &[ClaudeEffortCapSource {
                max_effort_level: None,
                model_max_effort_levels: per_model,
            }],
        );
        let labels_of = |model: &ClaudeModelRow| -> Vec<String> {
            model
                .capabilities
                .option_descriptors
                .iter()
                .find_map(|d| match d {
                    ProviderOptionDescriptor::Select { options, .. } => {
                        Some(options.iter().map(|o| o.label.clone()).collect())
                    }
                    _ => None,
                })
                .unwrap_or_default()
        };
        assert_eq!(
            labels_of(&capped[0]),
            vec!["Medium", "High", "Extra High (capped to High)", "Max (capped to High)", "Ultracode (capped to High)", "Ultrathink"]
        );
        assert_eq!(
            labels_of(&capped[1]),
            vec!["Medium", "High", "Extra High", "Max", "Ultracode", "Ultrathink"]
        );

        // Without an Extra High option (Opus 4.6), the CLI clamps an xhigh cap down to High.
        let no_xhigh = row(
            "claude-no-xhigh",
            "claude-no-xhigh",
            &[],
            vec![descriptor(&[
                ("low", "Low", false),
                ("medium", "Medium", false),
                ("high", "High", true),
                ("max", "Max", false),
            ])],
        );
        let capped = cap_claude_catalog_effort(
            &ClaudeModelCatalog::default(),
            vec![no_xhigh],
            &[ClaudeEffortCapSource {
                max_effort_level: Some("xhigh".into()),
                ..Default::default()
            }],
        );
        assert_eq!(
            labels_of(&capped[0]),
            vec!["Low", "Medium", "High", "Max (capped to High)"]
        );
    }

    #[test]
    fn custom_models_keep_their_labels_under_a_cap() {
        let mut custom = row(
            "claude-custom-copy",
            "Copy",
            &[],
            vec![descriptor(&[
                ("high", "High", true),
                ("max", "Max (capped to High)", false),
            ])],
        );
        custom.is_custom = true;
        let capped = cap_claude_catalog_effort(
            &ClaudeModelCatalog::default(),
            vec![custom.clone()],
            &[ClaudeEffortCapSource {
                max_effort_level: Some("high".into()),
                ..Default::default()
            }],
        );
        assert_eq!(capped, vec![custom]);
    }

    #[test]
    fn resolves_max_effort_level_per_source_and_model_key_like_claude_code() {
        let opus = row(
            "claude-opus-5-5",
            "Opus 5.5",
            &["opus", "opus-5.5", "claude-opus-5.5"],
            vec![descriptor(&[
                ("low", "Low", false),
                ("medium", "Medium", false),
                ("high", "High", true),
                ("max", "Max", false),
            ])],
        );
        let catalog = ClaudeModelCatalog {
            models: vec![
                ClaudeCatalogModel {
                    model: opus.clone(),
                    runtime: Default::default(),
                    compatibility: Default::default(),
                    api_model: None,
                },
                ClaudeCatalogModel {
                    model: row("claude-opus-5", "Opus 5", &[], vec![]),
                    runtime: Default::default(),
                    compatibility: Default::default(),
                    api_model: None,
                },
            ],
        };
        let max_label = |sources: Vec<ClaudeEffortCapSource>| -> String {
            let capped = cap_claude_catalog_effort(&catalog, vec![opus.clone()], &sources);
            capped[0]
                .capabilities
                .option_descriptors
                .iter()
                .find_map(|d| match d {
                    ProviderOptionDescriptor::Select { options, .. } => {
                        Some(options.last()?.label.clone())
                    }
                    _ => None,
                })
                .unwrap_or_default()
        };
        let per_model = |entries: &[(&str, &str)]| ClaudeEffortCapSource {
            max_effort_level: None,
            model_max_effort_levels: entries
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        };
        let top = |level: &str| ClaudeEffortCapSource {
            max_effort_level: Some(level.into()),
            ..Default::default()
        };

        // Claude Code matches family aliases and [1m] spellings case-insensitively.
        assert_eq!(max_label(vec![per_model(&[("Opus", "medium")])]), "Max (capped to Medium)");
        assert_eq!(
            max_label(vec![per_model(&[("claude-opus-5-5[1M]", "medium")])]),
            "Max (capped to Medium)"
        );
        assert_eq!(
            max_label(vec![per_model(&[(" OPUS[1m] ", "medium")])]),
            "Max (capped to Medium)"
        );
        // Other keys name the most specific model id they contain, so dated and
        // provider-prefixed ids match while dotted aliases and less specific ids do not.
        for key in [
            "claude-opus-5-5-20260101",
            "CLAUDE-OPUS-5-5-20260101[1m]",
            "us.anthropic.claude-opus-5-5-v1:0",
            "claude-opus-5-5-latest",
        ] {
            assert_eq!(
                max_label(vec![per_model(&[(key, "medium")])]),
                "Max (capped to Medium)",
                "{key}"
            );
        }
        for key in ["claude-opus-5.5", "opus-5.5", "opus-5-5", "opus [1m]", "claude-opus-5"] {
            assert_eq!(max_label(vec![per_model(&[(key, "medium")])]), "Max", "{key}");
        }
        // Keys naming the same model keep the lowest cap, and a `max` key does not exempt it.
        assert_eq!(
            max_label(vec![per_model(&[("opus", "max"), ("claude-opus-5-5", "low")])]),
            "Max (capped to Low)"
        );
        // A per-model `max` only exempts the model from its own source's top-level cap.
        assert_eq!(
            max_label(vec![ClaudeEffortCapSource {
                max_effort_level: Some("medium".into()),
                model_max_effort_levels: [("opus".to_owned(), "max".to_owned())].into_iter().collect(),
            }]),
            "Max"
        );
        assert_eq!(
            max_label(vec![per_model(&[("opus", "max")]), top("high")]),
            "Max (capped to High)"
        );
        assert_eq!(
            max_label(vec![top("medium"), per_model(&[("opus", "high")])]),
            "Max (capped to Medium)"
        );
    }

    #[test]
    fn reported_models_skip_known_ids_and_synthesize_effort_descriptors() {
        let bundled = bundled_claude_model_catalog(0);
        let reported = vec![
            reported("default", "claude-opus-5-5"),
            ClaudeReportedModel {
                value: "opus".into(),
                resolved_model: Some("claude-opus-5-5".into()),
                display_name: "Opus".into(),
                supports_effort: true,
                supported_effort_levels: vec!["low".into(), "high".into()],
            },
            ClaudeReportedModel {
                value: "opusplan".into(),
                resolved_model: None,
                display_name: "".into(),
                supports_effort: false,
                supported_effort_levels: vec![],
            },
            ClaudeReportedModel {
                value: "sonnet[1m]".into(),
                resolved_model: Some("claude-sonnet-5-5[1m]".into()),
                display_name: "Sonnet".into(),
                supports_effort: true,
                supported_effort_levels: vec!["high".into()],
            },
            ClaudeReportedModel {
                value: "mystery".into(),
                resolved_model: None,
                display_name: " Mystery ".into(),
                supports_effort: true,
                supported_effort_levels: vec!["low".into(), "extreme".into()],
            },
        ];
        let rows = claude_reported_models(&bundled, &reported);
        let slugs: Vec<&str> = rows.iter().map(|r| r.slug.as_str()).collect();
        // `default` and `opus`/`sonnet[1m]` resolve to catalog slugs; `opusplan` and
        // `mystery` stay.
        assert_eq!(slugs, ["opusplan", "mystery"]);
        let mystery = &rows[1];
        assert_eq!(mystery.name, "Mystery");
        let ProviderOptionDescriptor::Select { options, .. } =
            &mystery.capabilities.option_descriptors[0]
        else {
            panic!("expected effort select");
        };
        assert_eq!(options[0].id, "default");
        assert!(options[0].is_default);
        // `extreme` is not a Claude effort level, so it keeps its raw id as the label.
        assert_eq!(options[2].label, "extreme");
    }
}
