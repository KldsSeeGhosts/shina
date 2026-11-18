//! Shared provider model catalog, ported function by function from KSG Code:
//! - `apps/server/src/provider/ModelManifest.ts` (`resolveProviderCatalog`)
//! - `packages/provider-core/src/server/ModelCatalog.ts` (`catalogServerModel`,
//!   `classifyCatalogModels`, `applyCatalogDefault`, `applyModelCatalog`,
//!   `CatalogMatching`)
//! - `packages/shared/src/semver.ts` (`parseSemver`, `compareSemverVersions`)
//!
//! The classify/default functions operate on Shina's compact model values, where
//! `"id"` carries KSG's `slug` and `"isCustom"`, `"isLegacy"`, `"isDefault"` and
//! `"aliases"` keep the KSG `ServerProviderModel` names.
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

/// KSG `ModelCatalog.CatalogMatching`: `family` reduces a wire slug to the slug
/// family the manifest catalogues (e.g. `openai.gpt-6-sol` -> `gpt-6-sol`).
#[derive(Clone, Copy, Default)]
pub struct CatalogMatching {
    pub family: Option<fn(&str) -> String>,
}

fn family_of(matching: &CatalogMatching, slug: &str) -> String {
    matching
        .family
        .map(|family| family(slug))
        .unwrap_or_else(|| slug.to_owned())
}

/// The exact-then-family catalog lookup KSG inlines in `classifyCatalogModels`
/// and `applyCatalogDefault`.
pub fn catalog_entry<'a>(
    catalog: Option<&'a ProviderCatalog>,
    matching: &CatalogMatching,
    slug: &str,
) -> Option<&'a ProviderCatalogModel> {
    catalog.and_then(|catalog| {
        catalog
            .models
            .iter()
            .find(|candidate| candidate.slug == slug)
            .or_else(|| {
                let family = family_of(matching, slug);
                catalog
                    .models
                    .iter()
                    .find(|candidate| candidate.slug == family)
            })
    })
}

/// Ported from `ModelCatalog.ts` (`classifyCatalogModels`). Flags known-legacy
/// models and clears stale flags; custom models are never reclassified.
pub fn classify_catalog_models(
    models: &mut Vec<Value>,
    catalog: Option<&ProviderCatalog>,
    matching: &CatalogMatching,
) {
    for model in models.iter_mut() {
        if model["isCustom"] == true {
            continue;
        }
        let slug = model["id"].as_str().unwrap_or_default().to_owned();
        let entry = catalog_entry(catalog, matching, &slug);
        if entry.is_some_and(|entry| entry.status == "legacy") {
            model["isLegacy"] = json!(true);
        } else if let Some(object) = model.as_object_mut() {
            object.remove("isLegacy");
        }
    }
}

/// Ported from `ModelCatalog.ts` (`applyCatalogDefault`). Moves `isDefault` and
/// its aliases from the wire default to the catalog's `defaults.chat` model,
/// resolving the target through the family when no exact slug matches.
pub fn apply_catalog_default(
    models: &mut Vec<Value>,
    catalog: Option<&ProviderCatalog>,
    matching: &CatalogMatching,
) {
    let Some(requested) = catalog.and_then(|catalog| catalog.default_chat_model.as_deref()) else {
        return;
    };
    let slug = models
        .iter()
        .find(|model| model["id"].as_str() == Some(requested))
        .and_then(|model| model["id"].as_str().map(str::to_owned))
        .or_else(|| {
            if matching.family.is_none() {
                return None;
            }
            let wanted = family_of(matching, requested);
            models
                .iter()
                .find(|model| {
                    model["isCustom"] != true
                        && family_of(matching, model["id"].as_str().unwrap_or_default()) == wanted
                })
                .and_then(|model| model["id"].as_str().map(str::to_owned))
        });
    let Some(slug) = slug else {
        return;
    };
    let Some((previous_index, moved_aliases)) = models.iter().enumerate().find_map(|(index, m)| {
        (m["isDefault"] == true && m["id"].as_str() != Some(slug.as_str())).then(|| {
            (
                index,
                m["aliases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a.as_str().map(str::to_owned))
                    .collect::<Vec<_>>(),
            )
        })
    }) else {
        return;
    };
    for (index, model) in models.iter_mut().enumerate() {
        if index == previous_index {
            if let Some(object) = model.as_object_mut() {
                object.remove("isDefault");
                object.remove("aliases");
            }
        } else if model["id"].as_str() == Some(slug.as_str()) {
            let mut aliases: Vec<String> = model["aliases"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a.as_str().map(str::to_owned))
                .collect();
            for alias in &moved_aliases {
                if !aliases.iter().any(|a| a == alias) {
                    aliases.push(alias.clone());
                }
            }
            model["isDefault"] = json!(true);
            if !aliases.is_empty() {
                model["aliases"] = json!(aliases);
            }
        }
    }
}

/// Ported from `ModelCatalog.ts` (`applyModelCatalog`), minus KSG's
/// `updateRequiredModels` field removal: the caller computes those separately,
/// as `applyCodexModelCatalog` does.
pub fn apply_model_catalog(
    mut models: Vec<Value>,
    catalog: Option<&ProviderCatalog>,
    matching: &CatalogMatching,
) -> Vec<Value> {
    classify_catalog_models(&mut models, catalog, matching);
    apply_catalog_default(&mut models, catalog, matching);
    models
}

#[cfg(test)]
mod tests {
    //! Ports of KSG's `ModelCatalog.test.ts` (`classifyCatalogModels`,
    //! `applyCatalogDefault`) and `ModelManifest.test.ts`
    //! (`resolveProviderCatalog`).
    use super::*;

    /// KSG `ModelCatalog.test.ts`'s `model` helper, on Shina's model values.
    fn model(overrides: Value) -> Value {
        let mut model = json!({"id":"model-test","name":"Model Test","isCustom":false});
        if let Some(overrides) = overrides.as_object() {
            for (key, value) in overrides {
                model[key] = value.clone();
            }
        }
        model
    }

    /// KSG `ModelCatalog.test.ts`'s `catalogModel` helper.
    fn catalog_model(slug: &str, status: &str) -> ProviderCatalogModel {
        ProviderCatalogModel {
            slug: slug.into(),
            name: slug.into(),
            short_name: None,
            sub_provider: None,
            aliases: vec![],
            badge: None,
            status: status.into(),
            capabilities: Default::default(),
            adapter: json!({}),
            profile_adapter: json!({}),
        }
    }

    /// KSG `ModelCatalog.test.ts`'s `prefixFamily`.
    const PREFIX_FAMILY: CatalogMatching = CatalogMatching {
        family: Some(|slug| slug.strip_prefix("vendor.").unwrap_or(slug).to_owned()),
    };

    fn catalog(models: Vec<ProviderCatalogModel>, default: Option<&str>) -> ProviderCatalog {
        ProviderCatalog {
            models,
            default_chat_model: default.map(str::to_owned),
        }
    }

    fn legacy_pairs(models: &[Value]) -> Vec<(String, bool)> {
        models
            .iter()
            .map(|model| {
                (
                    model["id"].as_str().unwrap().to_owned(),
                    model["isLegacy"] == true,
                )
            })
            .collect()
    }

    #[test]
    fn flags_only_known_legacy_models_clears_stale_flags_and_skips_custom() {
        let catalog = catalog(vec![catalog_model("old-model", "legacy")], None);
        let mut models = vec![
            model(json!({"id":"current-a"})),
            model(json!({"id":"current-b","isLegacy":true})),
            model(json!({"id":"old-model"})),
            model(json!({"id":"my-own-model","isCustom":true,"isLegacy":true})),
        ];
        classify_catalog_models(&mut models, Some(&catalog), &CatalogMatching::default());
        assert_eq!(
            legacy_pairs(&models),
            vec![
                ("current-a".to_owned(), false),
                ("current-b".to_owned(), false),
                ("old-model".to_owned(), true),
                ("my-own-model".to_owned(), true),
            ]
        );
    }

    #[test]
    fn matches_qualified_slugs_through_the_drivers_family_without_changing_ids() {
        let catalog = catalog(vec![catalog_model("model-old", "legacy")], None);
        let mut models = vec![
            model(json!({"id":"vendor.model-test","isLegacy":true})),
            model(json!({"id":"vendor.model-old"})),
        ];
        classify_catalog_models(&mut models, Some(&catalog), &PREFIX_FAMILY);
        assert_eq!(
            legacy_pairs(&models),
            vec![
                ("vendor.model-test".to_owned(), false),
                ("vendor.model-old".to_owned(), true),
            ]
        );
    }

    #[test]
    fn moves_the_default_flag_and_its_aliases_to_the_catalogs_chat_default() {
        let catalog = catalog(
            vec![catalog_model("model-new", "current")],
            Some("model-new"),
        );
        let mut models = vec![
            model(json!({"id":"model-old","isDefault":true,"aliases":["provider-default"]})),
            model(json!({"id":"model-new"})),
        ];
        apply_catalog_default(&mut models, Some(&catalog), &CatalogMatching::default());
        assert!(models[0].get("isDefault").is_none() && models[0].get("aliases").is_none());
        assert_eq!(models[1]["isDefault"], true);
        assert_eq!(models[1]["aliases"], json!(["provider-default"]));
        // No matching default target leaves the wire default alone.
        let mut partial = models[..1].to_vec();
        let before = partial.clone();
        apply_catalog_default(&mut partial, Some(&catalog), &CatalogMatching::default());
        assert_eq!(partial, before);
    }

    #[test]
    fn resolves_the_catalog_default_to_a_qualified_discovered_model() {
        let catalog = catalog(vec![], Some("model-test"));
        let mut models = vec![
            model(json!({"id":"vendor.model-old","isDefault":true})),
            model(json!({"id":"vendor.model-test"})),
        ];
        apply_catalog_default(&mut models, Some(&catalog), &PREFIX_FAMILY);
        let default = models.iter().find(|m| m["isDefault"] == true).unwrap();
        assert_eq!(default["id"], "vendor.model-test");
    }

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
