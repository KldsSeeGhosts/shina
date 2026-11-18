//! Shared provider model catalog, ported function by function from KSG Code:
//! - `apps/server/src/provider/ModelManifest.ts` (`resolveProviderCatalog`)
//! - `packages/provider-core/src/server/ModelCatalog.ts` (`catalogServerModel`)
//! - `packages/shared/src/semver.ts` (`parseSemver`, `compareSemverVersions`)
use crate::descriptors::ModelCapabilities;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::LazyLock;

const MODEL_MANIFEST_JSON: &str = include_str!("../assets/model-manifest.json");
const MODEL_NEW_BADGE_DURATION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

pub static BUNDLED_MANIFEST: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(MODEL_MANIFEST_JSON).unwrap_or(Value::Null));

/// `TrimmedNonEmptyString`: the manifest schema rejects empty and padded values.
pub fn trimmed_non_empty(value: &Value) -> Option<String> {
    let trimmed = value.as_str()?.trim();
    (!trimmed.is_empty() && trimmed.len() == value.as_str()?.len()).then(|| trimmed.to_owned())
}

/// Ported from `packages/shared/src/semver.ts` (`parseSemver` /
/// `normalizeSemverVersion`): strips build metadata and one leading `v`, pads
/// `major.minor` to three segments, and splits the prerelease on `.`.
pub fn parse_semver(version: &str) -> Option<(u64, u64, u64, Vec<String>)> {
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
    let Some((lm, ln, lp, lpre)) = parse_semver(left) else {
        return 1;
    };
    let Some((rm, rn, rp, rpre)) = parse_semver(right) else {
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
        let time = time
            .trim_end_matches('Z')
            .split(['+', '-'])
            .next()
            .unwrap_or("");
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

/// KSG `ServerProviderModel`.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogModelRow {
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

/// One `models[]` row joined to its profile, as `resolveProviderCatalog` yields it.
#[derive(Clone, Debug)]
pub struct ProviderCatalogModel {
    pub slug: String,
    pub name: String,
    pub short_name: Option<String>,
    pub sub_provider: Option<String>,
    pub aliases: Vec<String>,
    pub badge: Option<String>,
    pub status: String,
    pub capabilities: ModelCapabilities,
    pub adapter: Value,
    pub profile_adapter: Value,
}

/// KSG `ModelCatalog.ProviderCatalog`.
#[derive(Clone, Debug)]
pub struct ProviderCatalog {
    pub models: Vec<ProviderCatalogModel>,
    pub default_chat_model: Option<String>,
}

/// Ported from `apps/server/src/provider/ModelManifest.ts` (`resolveProviderCatalog`).
/// Returns `None` on any schema or reference violation, matching KSG's `null`.
/// Badge expiry is evaluated on every call so rereads age out "new" badges.
pub fn resolve_provider_catalog(
    manifest: &Value,
    provider: &str,
    now_ms: u64,
) -> Option<ProviderCatalog> {
    let catalog = manifest["providers"][provider].as_object()?;
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
            && released_ms
                .is_some_and(|ms| now_ms >= ms && now_ms - ms < MODEL_NEW_BADGE_DURATION_MS))
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
            short_name: entry.get("shortName").and_then(|v| trimmed_non_empty(v)),
            sub_provider: entry.get("subProvider").and_then(|v| trimmed_non_empty(v)),
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
pub fn catalog_server_model(
    catalog: &ProviderCatalog,
    entry: &ProviderCatalogModel,
) -> CatalogModelRow {
    CatalogModelRow {
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

#[cfg(test)]
mod tests {
    //! Ports of KSG's `ModelManifest.test.ts` (`resolveProviderCatalog`).
    use super::*;

    /// KSG `ModelManifest.test.ts`'s badge matrix: `releasedAt` yields "new" only
    /// for the week after release, on a `current` model.
    #[test]
    fn derives_the_new_badge_from_released_at() {
        let day_ms = 86_400_000;
        for (released_at, now_ms, badge) in [
            (Some("2026-01-01"), 1767225600000, Some("new")), // 2026-01-01T00:00:00Z
            (
                Some("2026-01-01"),
                1767225600000 + 7 * day_ms - 1,
                Some("new"),
            ), // 2026-01-07T23:59:59.999Z
            (Some("2026-01-01"), 1767225600000 + 7 * day_ms, None), // 2026-01-08T00:00:00Z
            (Some("2026-01-01"), 1767225600000 + 31 * day_ms, None), // 2026-02-01T00:00:00Z
            (Some("2026-01-02T00:00:00Z"), 1767225600000, None), // released in the future
            (None, 1767225600000, None),
            (Some("invalid"), 1767225600000, None),
        ] {
            let mut next = json!({"slug":"model-next","name":"Model Next","status":"current"});
            if let Some(released) = released_at {
                next["releasedAt"] = json!(released);
            }
            let manifest = json!({"providers":{"synthetic":{"profiles":{},"models":[next,
                {"slug":"model-legacy","name":"Model Legacy","status":"legacy",
                 "releasedAt":"2026-01-01"}]}}});
            let catalog = resolve_provider_catalog(&manifest, "synthetic", now_ms).unwrap();
            assert_eq!(
                catalog.models[0].badge.as_deref(),
                badge,
                "{released_at:?} {now_ms}"
            );
            assert_eq!(catalog.models[1].badge, None);
        }
    }

    #[test]
    fn rejects_invalid_catalog_references() {
        let manifest = |models: Value, defaults: Option<&str>| {
            let mut catalog = json!({"profiles":{},"models":models});
            if let Some(default) = defaults {
                catalog["defaults"] = json!({"chat":default});
            }
            json!({"providers":{"synthetic":catalog}})
        };
        for invalid in [
            manifest(
                json!([
                    {"slug":"duplicate","name":"First","status":"current"},
                    {"slug":"duplicate","name":"Second","status":"current"},
                ]),
                None,
            ),
            manifest(
                json!([{"slug":"missing-profile","name":"Missing Profile",
                    "status":"current","profile":"missing"}]),
                None,
            ),
            manifest(
                json!([{"slug":"present","name":"Present","status":"current"}]),
                Some("absent"),
            ),
        ] {
            assert!(resolve_provider_catalog(&invalid, "synthetic", 0).is_none());
        }
    }
}
