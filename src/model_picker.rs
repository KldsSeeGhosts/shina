//! Model picker search ranking, ordering, favorites, continuation restrictions,
//! and provider readiness state, ported rule-for-rule from KSG Code.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MODEL_PICKER_FAVORITE_SCORE_BOOST: i32 = 24;
pub const MODEL_PICKER_EXACT_NAME_SCORE: i32 = -1000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFavorite {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequiredModel {
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    pub min_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPickerSearchableModel<'a> {
    pub driver_kind: &'a str,
    pub provider_display_name: &'a str,
    pub name: &'a str,
    pub short_name: Option<&'a str>,
    pub sub_provider: Option<&'a str>,
    pub is_favorite: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderStatusKind {
    Ready,
    Warning,
    Error,
    Disabled,
}

impl ProviderStatusKind {
    pub fn parse(value: &str) -> Self {
        match value {
            "ready" => Self::Ready,
            "warning" => Self::Warning,
            "error" => Self::Error,
            "disabled" => Self::Disabled,
            _ => Self::Warning,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderAuthStatus {
    Authenticated,
    Unauthenticated,
    Unknown,
}

impl ProviderAuthStatus {
    pub fn parse(value: &str) -> Self {
        match value {
            "authenticated" => Self::Authenticated,
            "unauthenticated" => Self::Unauthenticated,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderInstanceEntry {
    pub instance_id: String,
    pub driver_kind: String,
    pub display_name: String,
    pub continuation_group_key: Option<String>,
    pub enabled: bool,
    pub installed: bool,
    pub status: ProviderStatusKind,
    pub auth_status: ProviderAuthStatus,
    pub can_setup: bool,
    pub is_available: bool,
    pub is_acp: bool,
    pub message: Option<String>,
    pub compatibility_broken_message: Option<String>,
    pub update_required_models: Vec<UpdateRequiredModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerModelOption {
    pub slug: String,
    pub name: String,
    pub short_name: Option<String>,
    pub sub_provider: Option<String>,
    pub aliases: Vec<String>,
    pub badge: Option<String>,
    pub is_default: bool,
    pub is_custom: bool,
    pub is_legacy: bool,
    pub is_unavailable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPickerItem {
    pub slug: String,
    pub name: String,
    pub short_name: Option<String>,
    pub sub_provider: Option<String>,
    pub badge: Option<String>,
    pub is_legacy: bool,
    pub is_unavailable: bool,
    pub instance_id: String,
    pub driver_kind: String,
    pub instance_display_name: String,
    pub continuation_group_key: Option<String>,
    pub catalog_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelPickerTab {
    Favorites,
    Instance(String),
}

/// Ported from KSG Code's `packages/shared/src/searchRanking.ts` (`normalizeSearchQuery`).
pub fn normalize_search_query(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_lowercase()
    }
}

/// Ported from KSG Code's `packages/shared/src/searchRanking.ts` (`scoreSubsequenceMatch`).
pub fn score_subsequence_match(value: &str, query: &str, ignore_position: bool) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let v_chars: Vec<char> = value.chars().collect();
    let q_chars: Vec<char> = query.chars().collect();
    let mut query_index = 0usize;
    let mut first_match: Option<usize> = None;
    let mut prev_match: Option<usize> = None;
    let mut gap_penalty = 0usize;

    for (value_index, &ch) in v_chars.iter().enumerate() {
        if ch != q_chars[query_index] {
            continue;
        }
        if first_match.is_none() {
            first_match = Some(value_index);
        }
        if let Some(prev) = prev_match {
            gap_penalty += value_index - prev - 1;
        }
        prev_match = Some(value_index);
        query_index += 1;
        if query_index == q_chars.len() {
            let first = first_match.unwrap_or(0);
            let span_penalty = value_index - first + 1 - q_chars.len();
            if ignore_position {
                return Some((gap_penalty * 3 + span_penalty) as i32);
            }
            let length_penalty = v_chars.len().saturating_sub(q_chars.len()).min(64);
            return Some((first * 2 + gap_penalty * 3 + span_penalty + length_penalty) as i32);
        }
    }
    None
}

fn find_boundary_match_index(value: &str, query: &str, boundary_markers: &[&str]) -> Option<usize> {
    let mut best_index: Option<usize> = None;
    for marker in boundary_markers {
        let needle = format!("{marker}{query}");
        if let Some(byte_index) = value.find(&needle) {
            let match_index = value[..byte_index].chars().count() + marker.chars().count();
            if best_index.is_none_or(|best| match_index < best) {
                best_index = Some(match_index);
            }
        }
    }
    best_index
}

/// Ported from KSG Code's `packages/shared/src/searchRanking.ts` (`scoreQueryMatch`).
pub fn score_query_match(
    value: &str,
    query: &str,
    exact_base: i32,
    prefix_base: Option<i32>,
    boundary_base: Option<i32>,
    includes_base: Option<i32>,
    fuzzy_base: Option<i32>,
    boundary_markers: &[&str],
    ignore_position: bool,
) -> Option<i32> {
    if value.is_empty() || query.is_empty() {
        return None;
    }
    let penalty = if ignore_position {
        0
    } else {
        value
            .chars()
            .count()
            .saturating_sub(query.chars().count())
            .min(64) as i32
    };
    if value == query {
        return Some(exact_base);
    }
    if let Some(base) = prefix_base
        && value.starts_with(query)
    {
        return Some(base + penalty);
    }
    if let Some(base) = boundary_base
        && let Some(boundary_index) = find_boundary_match_index(value, query, boundary_markers)
    {
        return Some(
            base + if ignore_position {
                0
            } else {
                boundary_index as i32 * 2
            } + penalty,
        );
    }
    if let Some(base) = includes_base
        && let Some(byte_index) = value.find(query)
    {
        let includes_index = value[..byte_index].chars().count();
        return Some(
            base + if ignore_position {
                0
            } else {
                includes_index as i32 * 2
            } + penalty,
        );
    }
    if let Some(base) = fuzzy_base
        && let Some(fuzzy_score) = score_subsequence_match(value, query, ignore_position)
    {
        return Some(base + fuzzy_score);
    }
    None
}

fn normalize_model_name(name: &str) -> String {
    normalize_search_query(name)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_model_picker_search_text(model: &ModelPickerSearchableModel<'_>) -> String {
    let mut parts = vec![model.name];
    if let Some(short_name) = model.short_name.filter(|s| !s.is_empty()) {
        parts.push(short_name);
    }
    if let Some(sub_provider) = model.sub_provider.filter(|s| !s.is_empty()) {
        parts.push(sub_provider);
    }
    if !model.driver_kind.is_empty() {
        parts.push(model.driver_kind);
    }
    if !model.provider_display_name.is_empty() {
        parts.push(model.provider_display_name);
    }
    normalize_search_query(&parts.join(" "))
}

fn get_model_picker_search_fields(model: &ModelPickerSearchableModel<'_>) -> Vec<String> {
    let mut fields = vec![normalize_search_query(model.name)];
    if let Some(short_name) = model.short_name {
        fields.push(normalize_search_query(short_name));
    }
    if let Some(sub_provider) = model.sub_provider {
        fields.push(normalize_search_query(sub_provider));
    }
    fields.push(normalize_search_query(model.driver_kind));
    fields.push(normalize_search_query(model.provider_display_name));
    fields.push(build_model_picker_search_text(model));
    fields
}

fn score_model_picker_search_token(field: &str, token: &str, field_base: i32) -> Option<i32> {
    score_query_match(
        field,
        token,
        field_base,
        Some(field_base + 2),
        Some(field_base + 4),
        Some(field_base + 6),
        (token.chars().count() >= 3).then_some(field_base + 100),
        &[" ", "-", "_", "/"],
        true,
    )
}

/// Ported from KSG Code's `packages/client-runtime/src/modelSearch.ts` (`scoreModelPickerSearch`).
pub fn score_model_picker_search(
    model: &ModelPickerSearchableModel<'_>,
    query: &str,
) -> Option<i32> {
    let normalized = normalize_search_query(query);
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    if tokens.is_empty() {
        return Some(0);
    }
    let normalized_query = tokens.join(" ");
    if normalized_query == normalize_model_name(model.name)
        || normalized_query == normalize_model_name(model.short_name.unwrap_or(""))
    {
        return Some(MODEL_PICKER_EXACT_NAME_SCORE);
    }

    let fields = get_model_picker_search_fields(model);
    let mut score = 0i32;
    for token in tokens {
        let best = fields
            .iter()
            .enumerate()
            .filter_map(|(index, field)| {
                score_model_picker_search_token(field, token, index as i32 * 10)
            })
            .min()?;
        score += best;
    }

    Some(if model.is_favorite {
        score - MODEL_PICKER_FAVORITE_SCORE_BOOST
    } else {
        score
    })
}

/// Ported from KSG Code's `packages/client-runtime/src/modelSearch.ts` (`rankModelPickerSearch`).
pub fn rank_model_picker_search<'a, T>(
    items: &'a [T],
    query: &str,
    to_searchable: impl Fn(&'a T) -> ModelPickerSearchableModel<'a>,
) -> Vec<&'a T> {
    let mut ranked: Vec<(&'a T, i32, bool, usize)> = Vec::new();
    for (order, item) in items.iter().enumerate() {
        let searchable = to_searchable(item);
        if let Some(score) = score_model_picker_search(&searchable, query) {
            ranked.push((item, score, searchable.is_favorite, order));
        }
    }
    ranked.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.3.cmp(&b.3))
    });
    ranked.into_iter().map(|entry| entry.0).collect()
}

pub const COMPOSER_MODEL_MENTION_LIMIT: usize = 8;

fn replace_runs(input: &str, matches: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_run = false;
    for ch in input.chars() {
        if matches(ch) {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            out.push(ch);
            in_run = false;
        }
    }
    out.trim().to_owned()
}

fn mention_search_queries(query: &str) -> Vec<String> {
    let spaced = replace_runs(query, |c| matches!(c, ':' | '/'));
    let unhyphenated = replace_runs(&spaced, |c| c == '-');
    // One letter would fill the menu with models and push files out of it.
    if unhyphenated.chars().filter(|c| !c.is_whitespace()).count() < 2 {
        return Vec::new();
    }
    if unhyphenated == spaced {
        vec![spaced]
    } else {
        vec![spaced, unhyphenated]
    }
}

fn contains_word_prefix(haystack: &str, token: &str) -> bool {
    if token.is_empty() {
        return true;
    }
    haystack.match_indices(token).any(|(byte_idx, _)| {
        byte_idx == 0
            || haystack[..byte_idx]
                .chars()
                .next_back()
                .is_none_or(|ch| !ch.is_alphanumeric())
    })
}

/// Ported from KSG Code's `packages/client-runtime/src/composerModelMentions.ts` (`rankComposerModelMentions`).
pub fn rank_composer_model_mentions<'a, T>(
    candidates: &'a [T],
    query: &str,
    to_searchable: impl Fn(&'a T) -> ModelPickerSearchableModel<'a>,
) -> Vec<&'a T> {
    for words in mention_search_queries(query) {
        let lower_words = words.to_lowercase();
        let tokens: Vec<&str> = lower_words.split_whitespace().collect();
        let literal: Vec<&'a T> = candidates
            .iter()
            .filter(|candidate| {
                let model = to_searchable(candidate);
                let haystack = [
                    Some(model.name),
                    model.short_name,
                    model.sub_provider,
                    Some(model.driver_kind),
                    Some(model.provider_display_name),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
                tokens
                    .iter()
                    .all(|token| contains_word_prefix(&haystack, token))
            })
            .collect();
        let ranked = rank_model_picker_search(&literal, &words, |item| to_searchable(*item));
        if !ranked.is_empty() {
            return ranked
                .into_iter()
                .copied()
                .take(COMPOSER_MODEL_MENTION_LIMIT)
                .collect();
        }
    }
    Vec::new()
}

pub fn provider_model_key(instance_id: &str, slug: &str) -> String {
    format!("{instance_id}:{slug}")
}

/// Ported from KSG Code's `apps/web/src/modelOrdering.ts` (`sortProviderModelItems`).
pub fn sort_provider_model_items<'a, T>(
    items: &'a [T],
    instance_id_of: impl Fn(&T) -> &str,
    slug_of: impl Fn(&T) -> &str,
    favorite_model_keys: &HashSet<String>,
    group_favorites: bool,
    instance_order: &[String],
) -> Vec<&'a T> {
    let instance_rank: HashMap<&str, usize> = instance_order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();
    let mut indexed: Vec<(&'a T, bool, usize, usize)> = items
        .iter()
        .enumerate()
        .map(|(original_order, item)| {
            let instance_id = instance_id_of(item);
            let slug = slug_of(item);
            let is_fav = favorite_model_keys.contains(&provider_model_key(instance_id, slug));
            let rank = instance_rank
                .get(instance_id)
                .copied()
                .unwrap_or(usize::MAX);
            (item, is_fav, rank, original_order)
        })
        .collect();
    indexed.sort_by(|a, b| {
        if group_favorites {
            b.1.cmp(&a.1)
        } else {
            Ordering::Equal
        }
        .then_with(|| a.2.cmp(&b.2))
        .then_with(|| a.3.cmp(&b.3))
    });
    indexed.into_iter().map(|entry| entry.0).collect()
}

fn strip_leading_qualifier(value: &str, qualifier: Option<&str>) -> String {
    let Some(trimmed_q) = qualifier.map(str::trim).filter(|s| !s.is_empty()) else {
        return value.to_owned();
    };
    let lower_val = value.to_lowercase();
    let lower_q = trimmed_q.to_lowercase();
    if let Some(after_q) = lower_val.strip_prefix(&lower_q) {
        let rest_bytes = value.len() - after_q.len();
        let rest = &value[rest_bytes..];
        let trimmed_ws = rest.trim_start();
        if let Some(first) = trimmed_ws.chars().next()
            && matches!(first, '.' | ':' | '/' | '-')
        {
            let after_sep = trimmed_ws[first.len_utf8()..].trim_start();
            if !after_sep.is_empty() {
                return after_sep.trim().to_owned();
            }
        } else if rest.len() > trimmed_ws.len() && !trimmed_ws.is_empty() {
            return trimmed_ws.trim().to_owned();
        }
    }
    value.to_owned()
}

/// Ported from KSG Code's `apps/web/src/components/chat/providerIconUtils.ts` (`getDisplayModelName`).
pub fn get_display_model_name(
    name: &str,
    short_name: Option<&str>,
    sub_provider: Option<&str>,
    prefer_short_name: bool,
) -> String {
    let base = if prefer_short_name {
        short_name.unwrap_or(name)
    } else {
        name
    };
    strip_leading_qualifier(base, sub_provider)
}

/// Ported from KSG Code's `apps/web/src/providerInstances.ts` (`isProviderInstancePickerReady`).
pub fn is_provider_instance_picker_ready(entry: &ProviderInstanceEntry) -> bool {
    entry.enabled && entry.is_available && entry.status == ProviderStatusKind::Ready
}

/// Ported from KSG Code's `apps/web/src/providerInstances.ts` (`isProviderInstancePickerVisible`).
pub fn is_provider_instance_picker_visible(entry: &ProviderInstanceEntry) -> bool {
    entry.enabled
}

fn has_provider_setup(entry: &ProviderInstanceEntry) -> bool {
    entry.driver_kind == "antigravity" || entry.can_setup
}

/// Ported from KSG Code's `apps/web/src/components/chat/ModelPickerContent.tsx` (`shouldIncludeModelPickerOption`).
pub fn should_include_model_picker_option(
    entry: &ProviderInstanceEntry,
    option: &PickerModelOption,
    active_instance_id: &str,
    active_model: &str,
) -> bool {
    if entry.driver_kind == "antigravity" && option.slug == "antigravity-default" {
        return false;
    }
    if is_provider_instance_picker_ready(entry) {
        return true;
    }
    entry.enabled
        && matches!(entry.driver_kind.as_str(), "opencode" | "antigravity")
        && entry.instance_id == active_instance_id
        && option.slug == active_model
        && option.is_unavailable
}

/// Ported from KSG Code's `apps/web/src/components/chat/ModelPickerContent.tsx` (`shouldOfferModelPickerSetup`).
pub fn should_offer_model_picker_setup(
    entry: &ProviderInstanceEntry,
    options: &[PickerModelOption],
) -> bool {
    entry.enabled
        && entry.status != ProviderStatusKind::Disabled
        && has_provider_setup(entry)
        && (!is_provider_instance_picker_ready(entry)
            || !entry.installed
            || entry.auth_status == ProviderAuthStatus::Unauthenticated
            || !options.iter().any(|option| !option.is_unavailable))
}

/// Ported from KSG Code's `apps/web/src/components/chat/ProviderModelPicker.logic.ts` (`retainedModelWhileProviderLocked`).
pub fn retained_model_while_provider_locked(
    entry: Option<&ProviderInstanceEntry>,
    models: &[PickerModelOption],
    model: &str,
) -> Option<PickerModelOption> {
    let entry = entry?;
    if (entry.enabled && entry.is_available) || model.is_empty() {
        return None;
    }
    if matches!(entry.driver_kind.as_str(), "opencode" | "antigravity") {
        return None;
    }
    Some(
        models
            .iter()
            .find(|candidate| candidate.slug == model)
            .cloned()
            .unwrap_or_else(|| PickerModelOption {
                slug: model.to_owned(),
                name: model.to_owned(),
                short_name: None,
                sub_provider: None,
                aliases: vec![],
                badge: None,
                is_default: false,
                is_custom: false,
                is_legacy: false,
                is_unavailable: false,
            }),
    )
}

/// Ported from KSG Code's `apps/web/src/components/chat/ModelPickerSidebar.tsx` (`describeUnavailableInstance`).
pub fn describe_unavailable_instance(entry: &ProviderInstanceEntry) -> String {
    let label = &entry.display_name;
    if (!entry.enabled && entry.is_available) || entry.status == ProviderStatusKind::Disabled {
        return format!("{label}: Disabled in settings.");
    }
    if entry.status == ProviderStatusKind::Ready && entry.is_available {
        return label.clone();
    }
    let kind = match entry.status {
        ProviderStatusKind::Error => "Unavailable",
        ProviderStatusKind::Warning => "Limited",
        _ => "Not ready",
    };
    match entry
        .message
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        Some(msg) => format!("{label}: {kind}. {msg}"),
        None => format!("{label}: {kind}."),
    }
}

/// Ported from KSG Code's `apps/web/src/components/chat/ProviderStatusBanner.tsx` (`getProviderStatusMessage`).
pub fn get_provider_status_message(entry: &ProviderInstanceEntry) -> String {
    if entry.auth_status != ProviderAuthStatus::Unauthenticated
        && let Some(msg) = &entry.compatibility_broken_message
        && !msg.is_empty()
    {
        return msg.clone();
    }
    if let Some(msg) = &entry.message
        && !msg.is_empty()
    {
        return msg.clone();
    }
    let provider_name = if entry.display_name.trim().is_empty() {
        entry.driver_kind.as_str()
    } else {
        entry.display_name.trim()
    };
    if !entry.installed && has_provider_setup(entry) {
        return format!("Open provider setup to install {provider_name} on this environment.");
    }
    if entry.auth_status == ProviderAuthStatus::Unauthenticated {
        if has_provider_setup(entry) {
            return if entry.driver_kind == "antigravity" {
                "Open provider setup to sign in with Google.".into()
            } else {
                "Open provider setup to sign in.".into()
            };
        }
        return "Sign in via the CLI to authenticate again.".into();
    }
    match entry.status {
        ProviderStatusKind::Ready => "No models are available for this provider.".into(),
        ProviderStatusKind::Error => format!("{provider_name} provider is unavailable."),
        _ => format!("{provider_name} provider has limited availability."),
    }
}

fn format_model_list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [first, second] => format!("{first} and {second}"),
        [init @ .., last] => format!("{}, and {last}", init.join(", ")),
    }
}

/// Ported from KSG Code's `packages/client-runtime/src/providerUpdateRequiredModels.ts` (`formatProviderUpdateRequiredNotice`).
pub fn format_provider_update_required_notice(
    driver_kind: &str,
    display_name: &str,
    update_required_models: &[UpdateRequiredModel],
    search_query: &str,
) -> Option<String> {
    let query = search_query.trim().to_lowercase();
    let models: Vec<&UpdateRequiredModel> = update_required_models
        .iter()
        .filter(|model| {
            query.is_empty()
                || model.name.to_lowercase().contains(&query)
                || model.slug.to_lowercase().contains(&query)
        })
        .collect();
    if models.is_empty() {
        return None;
    }
    let min_version = models
        .iter()
        .map(|m| m.min_version.as_str())
        .max_by(|a, b| compare_semver_versions(a, b))?;
    let provider_name = match driver_kind {
        "claude" | "claudeAgent" => "Claude Code",
        "codex" => "the Codex CLI",
        _ => display_name,
    };
    let version = if min_version.starts_with('v') {
        min_version.to_owned()
    } else {
        format!("v{min_version}")
    };
    let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
    Some(format!(
        "Update {provider_name} to {version} or newer to use {}.",
        format_model_list(&names)
    ))
}

#[derive(Debug)]
struct ParsedSemver<'a> {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Vec<&'a str>,
}

fn parse_semver(value: &str) -> Option<ParsedSemver<'_>> {
    let without_build = value.trim().split('+').next().unwrap_or("");
    let without_v = without_build.strip_prefix('v').unwrap_or(without_build);
    let (main, prerelease_str) = match without_v.split_once('-') {
        Some((m, p)) => (m, Some(p)),
        None => (without_v, None),
    };
    let mut parts: Vec<&str> = main
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    while parts.len() < 3 {
        parts.push("0");
    }
    if parts.len() != 3 {
        return None;
    }
    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;
    let patch = parts[2].parse::<u64>().ok()?;
    let prerelease = prerelease_str
        .map(|p| {
            p.split('.')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Some(ParsedSemver {
        major,
        minor,
        patch,
        prerelease,
    })
}

/// Ported from KSG Code's `packages/shared/src/semver.ts` (`compareSemverVersions`).
pub fn compare_semver_versions(left: &str, right: &str) -> Ordering {
    let (Some(l), Some(r)) = (parse_semver(left), parse_semver(right)) else {
        return left.cmp(right);
    };
    l.major
        .cmp(&r.major)
        .then_with(|| l.minor.cmp(&r.minor))
        .then_with(|| l.patch.cmp(&r.patch))
        .then_with(
            || match (l.prerelease.is_empty(), r.prerelease.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    let len = l.prerelease.len().max(r.prerelease.len());
                    for i in 0..len {
                        match (l.prerelease.get(i), r.prerelease.get(i)) {
                            (None, Some(_)) => return Ordering::Less,
                            (Some(_), None) => return Ordering::Greater,
                            (Some(li), Some(ri)) => {
                                let ord = match (li.parse::<u64>(), ri.parse::<u64>()) {
                                    (Ok(ln), Ok(rn)) => ln.cmp(&rn),
                                    (Ok(_), Err(_)) => Ordering::Less,
                                    (Err(_), Ok(_)) => Ordering::Greater,
                                    (Err(_), Err(_)) => li.cmp(ri),
                                };
                                if ord != Ordering::Equal {
                                    return ord;
                                }
                            }
                            (None, None) => break,
                        }
                    }
                    Ordering::Equal
                }
            },
        )
}

/// Ported from KSG Code's `apps/web/src/components/ChatView.logic.ts` (`getStartedThreadModelChangeBlockReason`).
pub fn get_started_thread_model_change_block_reason(
    has_started_session: bool,
    supports_handoff: bool,
    current_instance_id: &str,
    next_instance_id: &str,
) -> Option<(&'static str, &'static str)> {
    if !has_started_session {
        return None;
    }
    if current_instance_id == next_instance_id || supports_handoff {
        return None;
    }
    Some((
        "Start a new chat to switch providers",
        "This thread does not support switching providers after it has started.",
    ))
}

/// Ported from KSG Code's `apps/web/src/components/ChatView.logic.ts` (`makeModelDisabledReason`).
pub fn get_model_disabled_reason(
    has_started_session: bool,
    supports_handoff: bool,
    current_instance_id: &str,
    next_instance_id: &str,
) -> Option<String> {
    let (_, description) = get_started_thread_model_change_block_reason(
        has_started_session,
        supports_handoff,
        current_instance_id,
        next_instance_id,
    )?;
    Some(format!(
        "{description} Start a new thread to use this model."
    ))
}

pub fn matches_locked_provider(
    entry: &ProviderInstanceEntry,
    locked_provider: Option<&str>,
    locked_continuation_group_key: Option<&str>,
) -> bool {
    let Some(locked) = locked_provider else {
        return true;
    };
    if entry.driver_kind != locked && entry.instance_id != locked {
        return false;
    }
    if let Some(group_key) = locked_continuation_group_key {
        return entry.continuation_group_key.as_deref() == Some(group_key);
    }
    true
}

/// Ported from KSG Code's `apps/web/src/components/chat/ModelPickerContent.tsx` (`adjacentModelPickerProvider`).
pub fn adjacent_model_picker_provider(
    entries: &[ProviderInstanceEntry],
    selected: &ModelPickerTab,
    direction: isize,
    disabled_instance_ids: &HashSet<String>,
    selectable_unavailable_instance_ids: &HashSet<String>,
) -> ModelPickerTab {
    let mut providers = vec![ModelPickerTab::Favorites];
    for entry in entries {
        if !disabled_instance_ids.contains(&entry.instance_id)
            && (is_provider_instance_picker_ready(entry)
                || selectable_unavailable_instance_ids.contains(&entry.instance_id))
        {
            providers.push(ModelPickerTab::Instance(entry.instance_id.clone()));
        }
    }
    let len = providers.len() as isize;
    let index = providers.iter().position(|tab| tab == selected);
    let next = match index {
        None => {
            if direction >= 0 {
                0
            } else {
                providers.len() - 1
            }
        }
        Some(idx) => ((idx as isize + direction).rem_euclid(len)) as usize,
    };
    providers[next].clone()
}

/// Ported from KSG Code's `apps/web/src/components/chat/ModelPickerContent.tsx` initial tab rule.
pub fn initial_model_picker_tab(
    entries: &[ProviderInstanceEntry],
    options_by_instance: &BTreeMap<String, Vec<PickerModelOption>>,
    favorites: &[ModelFavorite],
    active_instance_id: &str,
    active_model: &str,
    locked_provider: Option<&str>,
) -> ModelPickerTab {
    let active_entry = entries
        .iter()
        .find(|entry| entry.instance_id == active_instance_id);
    let active_options = options_by_instance
        .get(active_instance_id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let active_has_selectable_unavailable = active_entry.is_some_and(|entry| {
        !is_provider_instance_picker_ready(entry)
            && active_options.iter().any(|option| {
                should_include_model_picker_option(entry, option, active_instance_id, active_model)
            })
    });
    let active_needs_setup =
        active_entry.is_some_and(|entry| should_offer_model_picker_setup(entry, active_options));
    if locked_provider.is_some() || active_has_selectable_unavailable || active_needs_setup {
        return ModelPickerTab::Instance(active_instance_id.to_owned());
    }
    let has_pickable_favorite = favorites.iter().any(|favorite| {
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.instance_id == favorite.provider)
        else {
            return false;
        };
        options_by_instance
            .get(&favorite.provider)
            .is_some_and(|options| {
                options.iter().any(|option| {
                    option.slug == favorite.model
                        && should_include_model_picker_option(
                            entry,
                            option,
                            active_instance_id,
                            active_model,
                        )
                })
            })
    });
    if has_pickable_favorite {
        ModelPickerTab::Favorites
    } else {
        ModelPickerTab::Instance(active_instance_id.to_owned())
    }
}

/// Ported from KSG Code's `apps/web/src/components/chat/ModelPickerContent.tsx` (`legacySection`).
pub fn split_legacy_models(
    filtered_models: Vec<ModelPickerItem>,
    is_searching: bool,
    is_favorites_tab: bool,
    favorites_set: &HashSet<String>,
) -> (Vec<ModelPickerItem>, Vec<ModelPickerItem>) {
    if is_searching || is_favorites_tab {
        return (filtered_models, Vec::new());
    }
    let mut current_models = Vec::new();
    let mut legacy_models = Vec::new();
    for model in filtered_models {
        let is_legacy = model.is_legacy
            && !favorites_set.contains(&provider_model_key(&model.instance_id, &model.slug));
        if is_legacy {
            legacy_models.push(model);
        } else {
            current_models.push(model);
        }
    }
    (current_models, legacy_models)
}

pub fn toggle_model_favorite_in_list(
    favorites: &mut Vec<ModelFavorite>,
    provider: &str,
    model: &str,
) -> bool {
    if let Some(index) = favorites
        .iter()
        .position(|fav| fav.provider == provider && fav.model == model)
    {
        favorites.remove(index);
        false
    } else {
        favorites.push(ModelFavorite {
            provider: provider.to_owned(),
            model: model.to_owned(),
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_fixture() -> Vec<(
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
        bool,
    )> {
        let mut out = Vec::new();
        for name in [
            "Claude Opus 5.5",
            "Claude Sonnet 5.5",
            "Claude Fable 5.1",
            "Claude Fable 5",
            "Claude Opus 5",
            "Claude Opus 4.8",
            "Claude Opus 4.7",
            "Claude Opus 4.6",
            "Claude Opus 4.5",
            "Claude Sonnet 5",
            "Claude Sonnet 4.6",
            "Claude Haiku 5.5",
            "Claude Haiku 4.5",
        ] {
            out.push(("claudeAgent", "Claude", name, None, false));
        }
        for name in [
            "GPT-6.1 Sol",
            "GPT-6 Astra",
            "GPT-6 Luna",
            "GPT-6 Sol",
            "GPT-5.6 Sol",
            "GPT-5.6 Terra",
            "GPT-5.6 Luna",
            "GPT-5.5",
        ] {
            out.push(("codex", "Codex", name, None, false));
        }
        for name in ["Grok 4.7", "Grok 4.6", "Grok 4.5"] {
            out.push(("grok", "Grok", name, None, false));
        }
        for name in [
            "Gemini 3.8 Flash (High)",
            "Gemini 3.8 Flash (Medium)",
            "Gemini 3.8 Flash (Low)",
            "Gemini 3.7 Flash (High)",
            "Gemini 3.7 Flash (Medium)",
            "Gemini 3.7 Flash (Low)",
            "Gemini 3.6 Flash (High)",
            "Gemini 3.6 Flash (Medium)",
            "Gemini 3.6 Flash (Low)",
            "Gemini 3.1 Pro (High)",
            "Gemini 3.1 Pro (Low)",
        ] {
            out.push(("antigravity", "Antigravity", name, None, false));
        }
        out.push(("opencode", "OpenCode", "Kimi K3", Some("Moonshot"), false));
        out.push(("opencode", "OpenCode", "Kimi K2.5", Some("Moonshot"), false));
        out.push(("opencode", "OpenCode", "Kimi K2", Some("Moonshot"), false));
        out
    }

    fn search(
        query: &str,
        models: &[(
            &'static str,
            &'static str,
            &'static str,
            Option<&'static str>,
            bool,
        )],
    ) -> Vec<&'static str> {
        rank_model_picker_search(models, query, |m| ModelPickerSearchableModel {
            driver_kind: m.0,
            provider_display_name: m.1,
            name: m.2,
            short_name: None,
            sub_provider: m.3,
            is_favorite: m.4,
        })
        .into_iter()
        .map(|m| m.2)
        .collect()
    }

    // Ported from KSG Code `packages/client-runtime/src/modelSearch.test.ts`.
    #[test]
    fn score_model_picker_search_ranks_exact_tokens_ahead_of_fuzzy_and_respects_better_text_over_favorite()
     {
        let exact_score = score_model_picker_search(
            &ModelPickerSearchableModel {
                driver_kind: "opencode",
                provider_display_name: "opencode",
                name: "Claude Opus 4.7",
                short_name: None,
                sub_provider: Some("GitHub Copilot"),
                is_favorite: false,
            },
            "copilot opus",
        );
        let fuzzy_score = score_model_picker_search(
            &ModelPickerSearchableModel {
                driver_kind: "opencode",
                provider_display_name: "opencode",
                name: "Claude Opus 4.7",
                short_name: None,
                sub_provider: Some("GitHub Copilot"),
                is_favorite: false,
            },
            "coplt op",
        );
        assert!(exact_score.is_some());
        assert!(fuzzy_score.is_some());
        assert!(exact_score.unwrap() < fuzzy_score.unwrap());

        let favorite_score = score_model_picker_search(
            &ModelPickerSearchableModel {
                driver_kind: "claudeAgent",
                provider_display_name: "Claude",
                name: "Claude Opus 4.7",
                short_name: None,
                sub_provider: None,
                is_favorite: true,
            },
            "opus 4.7",
        );
        let non_favorite_exact = score_model_picker_search(
            &ModelPickerSearchableModel {
                driver_kind: "cursor",
                provider_display_name: "Cursor",
                name: "Opus 4.7",
                short_name: None,
                sub_provider: None,
                is_favorite: false,
            },
            "opus 4.7",
        );
        assert!(favorite_score.is_some());
        assert!(non_favorite_exact.is_some());
        assert!(non_favorite_exact.unwrap() < favorite_score.unwrap());
    }

    // Ported from KSG Code `packages/client-runtime/src/modelSearch.test.ts`.
    #[test]
    fn rank_model_picker_search_matches_ksg_test_suite() {
        let catalog = catalog_fixture();
        assert_eq!(
            search("opus 4", &catalog),
            [
                "Claude Opus 4.8",
                "Claude Opus 4.7",
                "Claude Opus 4.6",
                "Claude Opus 4.5",
            ]
        );
        assert_eq!(
            search("opus 5", &catalog),
            ["Claude Opus 5.5", "Claude Opus 5", "Claude Opus 4.5"]
        );
        assert_eq!(search("opus 5.", &catalog), ["Claude Opus 5.5"]);

        assert_eq!(
            &search("gpt-6 sol", &catalog)[..2],
            ["GPT-6 Sol", "GPT-6.1 Sol"]
        );
        assert_eq!(
            &search("claude opus 5", &catalog)[..2],
            ["Claude Opus 5", "Claude Opus 5.5"]
        );
        assert_eq!(search("Grok 4.6", &catalog)[0], "Grok 4.6");
        assert_eq!(
            search("moonshot", &catalog),
            ["Kimi K3", "Kimi K2.5", "Kimi K2"]
        );

        let with_fav: Vec<_> = catalog
            .iter()
            .map(|m| {
                if m.2 == "Claude Opus 4.5" {
                    (m.0, m.1, m.2, m.3, true)
                } else {
                    *m
                }
            })
            .collect();
        assert_eq!(
            &search("opus", &with_fav)[..3],
            ["Claude Opus 4.5", "Claude Opus 5.5", "Claude Opus 5"]
        );
    }

    // Ported from KSG Code `ProviderModelPicker.logic.test.ts`.
    #[test]
    fn retained_model_while_provider_locked_keeps_hidden_saved_model() {
        let opus = PickerModelOption {
            slug: "claude-opus-5-5".into(),
            name: "Claude Opus 5.5".into(),
            short_name: Some("Opus 5.5".into()),
            sub_provider: None,
            aliases: vec![],
            badge: None,
            is_default: false,
            is_custom: false,
            is_legacy: false,
            is_unavailable: false,
        };
        let sonnet = PickerModelOption {
            slug: "claude-sonnet-5-5".into(),
            name: "Claude Sonnet 5.5".into(),
            short_name: Some("Sonnet 5.5".into()),
            sub_provider: None,
            aliases: vec![],
            badge: None,
            is_default: false,
            is_custom: false,
            is_legacy: false,
            is_unavailable: false,
        };
        let entry = |enabled: bool, is_available: bool| ProviderInstanceEntry {
            instance_id: "claude".into(),
            driver_kind: "claudeAgent".into(),
            display_name: "Claude".into(),
            continuation_group_key: None,
            enabled,
            installed: true,
            status: ProviderStatusKind::Ready,
            auth_status: ProviderAuthStatus::Authenticated,
            can_setup: false,
            is_available,
            is_acp: false,
            message: None,
            compatibility_broken_message: None,
            update_required_models: vec![],
        };
        assert_eq!(
            retained_model_while_provider_locked(
                Some(&entry(false, true)),
                &[sonnet.clone(), opus.clone()],
                &opus.slug,
            )
            .map(|m| m.slug),
            Some(opus.slug.clone())
        );
        assert_eq!(
            retained_model_while_provider_locked(
                Some(&entry(true, false)),
                &[sonnet.clone(), opus.clone()],
                &opus.slug,
            )
            .map(|m| m.slug),
            Some(opus.slug.clone())
        );
        let fallback =
            retained_model_while_provider_locked(Some(&entry(false, true)), &[sonnet], &opus.slug)
                .unwrap();
        assert_eq!(fallback.slug, opus.slug);
        assert_eq!(fallback.name, opus.slug);
        assert!(
            retained_model_while_provider_locked(
                Some(&entry(true, true)),
                &[opus.clone()],
                &opus.slug
            )
            .is_none()
        );
        assert!(retained_model_while_provider_locked(None, &[opus], &opus_slug_helper()).is_none());
    }

    fn opus_slug_helper() -> String {
        "claude-opus-5-5".into()
    }

    #[test]
    fn favorites_toggle_ordering_and_initial_tab_match_ksg() {
        let mut favs = Vec::new();
        assert!(toggle_model_favorite_in_list(
            &mut favs,
            "codex",
            "gpt-5.6-sol"
        ));
        assert_eq!(favs.len(), 1);
        assert!(!toggle_model_favorite_in_list(
            &mut favs,
            "codex",
            "gpt-5.6-sol"
        ));
        assert!(favs.is_empty());

        let items = vec![("codex", "a"), ("codex", "b"), ("pi", "c"), ("pi", "d")];
        let fav_keys = HashSet::from([
            provider_model_key("codex", "b"),
            provider_model_key("pi", "d"),
        ]);
        let sorted_provider =
            sort_provider_model_items(&items[..2], |i| i.0, |i| i.1, &fav_keys, true, &[]);
        assert_eq!(sorted_provider, vec![&("codex", "b"), &("codex", "a")]);

        let fav_items = vec![("codex", "b"), ("pi", "d")];
        let sorted_favs = sort_provider_model_items(
            &fav_items,
            |i| i.0,
            |i| i.1,
            &fav_keys,
            false,
            &["pi".into(), "codex".into()],
        );
        assert_eq!(sorted_favs, vec![&("pi", "d"), &("codex", "b")]);
    }

    #[test]
    fn legacy_models_collapse_unless_favorited_and_expand_when_searching() {
        let item = |slug: &str, is_legacy: bool| ModelPickerItem {
            slug: slug.into(),
            name: slug.into(),
            short_name: None,
            sub_provider: None,
            badge: None,
            is_legacy,
            is_unavailable: false,
            instance_id: "codex".into(),
            driver_kind: "codex".into(),
            instance_display_name: "Codex".into(),
            continuation_group_key: None,
            catalog_index: 0,
        };
        let models = vec![
            item("gpt-6-astra", false),
            item("gpt-5.5", true),
            item("gpt-5.4", true),
        ];
        let favs = HashSet::from([provider_model_key("codex", "gpt-5.5")]);
        let (current, legacy) = split_legacy_models(models.clone(), false, false, &favs);
        assert_eq!(
            current.iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(),
            ["gpt-6-astra", "gpt-5.5"]
        );
        assert_eq!(
            legacy.iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(),
            ["gpt-5.4"]
        );
        let (search_current, search_legacy) = split_legacy_models(models, true, false, &favs);
        assert_eq!(search_current.len(), 3);
        assert!(search_legacy.is_empty());
    }

    #[test]
    fn continuation_restrictions_and_update_required_notices_match_ksg() {
        assert_eq!(
            get_started_thread_model_change_block_reason(true, false, "pi", "codex"),
            Some((
                "Start a new chat to switch providers",
                "This thread does not support switching providers after it has started.",
            ))
        );
        assert_eq!(
            get_model_disabled_reason(true, false, "pi", "codex").as_deref(),
            Some(
                "This thread does not support switching providers after it has started. Start a new thread to use this model."
            )
        );
        assert!(get_model_disabled_reason(false, false, "pi", "codex").is_none());
        assert!(get_model_disabled_reason(true, false, "pi", "pi").is_none());

        let notice = format_provider_update_required_notice(
            "claude",
            "Claude",
            &[
                UpdateRequiredModel {
                    slug: "claude-opus-5-5".into(),
                    name: "Claude Opus 5.5".into(),
                    badge: None,
                    min_version: "2.1.280".into(),
                },
                UpdateRequiredModel {
                    slug: "claude-fable-5-2".into(),
                    name: "Claude Fable 5.2".into(),
                    badge: None,
                    min_version: "2.1.301".into(),
                },
            ],
            "",
        );
        assert_eq!(
            notice.as_deref(),
            Some(
                "Update Claude Code to v2.1.301 or newer to use Claude Opus 5.5 and Claude Fable 5.2."
            )
        );
        let filtered = format_provider_update_required_notice(
            "codex",
            "Codex",
            &[UpdateRequiredModel {
                slug: "gpt-6-luna".into(),
                name: "GPT-6 Luna".into(),
                badge: None,
                min_version: "0.155.0".into(),
            }],
            "luna",
        );
        assert_eq!(
            filtered.as_deref(),
            Some("Update the Codex CLI to v0.155.0 or newer to use GPT-6 Luna.")
        );
    }

    #[test]
    fn display_name_strips_leading_subprovider_qualifier() {
        assert_eq!(
            get_display_model_name(
                "Anthropic / Claude Opus 4.7",
                None,
                Some("Anthropic"),
                false
            ),
            "Claude Opus 4.7"
        );
        assert_eq!(
            get_display_model_name("openai/gpt-5", None, Some("openai"), false),
            "gpt-5"
        );
        assert_eq!(
            get_display_model_name("Moonshot Kimi K3", None, Some("Moonshot"), false),
            "Kimi K3"
        );
    }

    #[test]
    fn initial_picker_tab_skips_favorites_when_only_disabled_provider_is_favorited() {
        let entries = vec![
            ProviderInstanceEntry {
                instance_id: "codex".into(),
                driver_kind: "codex".into(),
                display_name: "Codex".into(),
                continuation_group_key: None,
                enabled: true,
                installed: true,
                status: ProviderStatusKind::Ready,
                auth_status: ProviderAuthStatus::Authenticated,
                can_setup: false,
                is_available: true,
                is_acp: false,
                message: None,
                compatibility_broken_message: None,
                update_required_models: vec![],
            },
            ProviderInstanceEntry {
                instance_id: "claude".into(),
                driver_kind: "claudeAgent".into(),
                display_name: "Claude".into(),
                continuation_group_key: None,
                enabled: false,
                installed: true,
                status: ProviderStatusKind::Disabled,
                auth_status: ProviderAuthStatus::Authenticated,
                can_setup: false,
                is_available: true,
                is_acp: false,
                message: None,
                compatibility_broken_message: None,
                update_required_models: vec![],
            },
        ];
        let mut options = BTreeMap::new();
        options.insert(
            "codex".into(),
            vec![PickerModelOption {
                slug: "codex".into(),
                name: "Codex model".into(),
                short_name: None,
                sub_provider: None,
                aliases: vec![],
                badge: None,
                is_default: true,
                is_custom: false,
                is_legacy: false,
                is_unavailable: false,
            }],
        );
        options.insert(
            "claude".into(),
            vec![PickerModelOption {
                slug: "claude".into(),
                name: "Claude model".into(),
                short_name: None,
                sub_provider: None,
                aliases: vec![],
                badge: None,
                is_default: true,
                is_custom: false,
                is_legacy: false,
                is_unavailable: false,
            }],
        );
        let disabled_favs = vec![ModelFavorite {
            provider: "claude".into(),
            model: "claude".into(),
        }];
        assert_eq!(
            initial_model_picker_tab(&entries, &options, &disabled_favs, "codex", "codex", None),
            ModelPickerTab::Instance("codex".into())
        );

        let ready_favs = vec![ModelFavorite {
            provider: "codex".into(),
            model: "codex".into(),
        }];
        assert_eq!(
            initial_model_picker_tab(&entries, &options, &ready_favs, "codex", "codex", None),
            ModelPickerTab::Favorites
        );
        assert_eq!(
            initial_model_picker_tab(
                &entries,
                &options,
                &ready_favs,
                "codex",
                "codex",
                Some("codex")
            ),
            ModelPickerTab::Instance("codex".into())
        );
    }
}
