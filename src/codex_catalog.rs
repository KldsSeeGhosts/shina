//! Codex model catalog application, ported function by function from KSG Code:
//! - `apps/server/src/provider/codexModelCatalog.ts` (`applyCodexModelCatalog`,
//!   `codexUpdateRequiredModels`, `CodexModelAdapter`)
//! - `apps/server/src/provider/CodexProvider.ts` (the `initialize.userAgent`
//!   version read, `applyPreferredCodexDefaultModel`)
//! - `packages/shared/src/model.ts` (`codexModelFamily`)
//!
//! Shared manifest/semver/catalog mechanics live in `crate::model_catalog`, as
//! KSG shares them through `provider-core`'s `ModelCatalog`.
use crate::model_catalog::{
    BUNDLED_MANIFEST, CatalogMatching, ProviderCatalog, apply_model_catalog,
    compare_semver_versions, parse_semver, resolve_provider_catalog, trimmed_non_empty,
};
use crate::model_picker::UpdateRequiredModel;
use serde_json::{Value, json};
use std::collections::HashSet;

/// KSG's `ProviderDriverKind` key for this provider's manifest section.
const CODEX_PROVIDER_KIND: &str = "codex";

/// Ported from `packages/shared/src/model.ts` (`codexModelFamily`): qualified
/// `openai.gpt-*` wire ids share the manifest slug of their family.
pub fn codex_model_family(slug: &str) -> &str {
    slug.strip_prefix("openai.")
        .filter(|rest| rest.starts_with("gpt-"))
        .unwrap_or(slug)
}

/// KSG `CODEX_MATCHING`.
pub const CODEX_MATCHING: CatalogMatching = CatalogMatching {
    family: Some(|slug| codex_model_family(slug).to_owned()),
};

/// Ported from `CodexProvider.ts`: `initialize.userAgent.match(/\/([^\s]+)/)`
/// names the installed Codex CLI version.
pub fn codex_cli_version(user_agent: &str) -> Option<String> {
    let mut rest = user_agent;
    while let Some(index) = rest.find('/') {
        rest = &rest[index + 1..];
        let version: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
        if !version.is_empty() {
            return Some(version);
        }
    }
    None
}

/// `providers.codex` resolved fresh on each read so its new-model badges can
/// expire; KSG's `ModelCatalog.bundled` fallback. `None` on schema violations.
pub fn bundled_codex_provider_catalog(now_ms: u64) -> Option<ProviderCatalog> {
    resolve_provider_catalog(&BUNDLED_MANIFEST, CODEX_PROVIDER_KIND, now_ms)
}

/// `CodexModelAdapter` (`codexModelCatalog.ts`): `codex.minVersion` is optional
/// and any decode failure means the gate is absent.
fn codex_adapter_min_version(adapter: &Value) -> Option<String> {
    let object = adapter.as_object()?;
    let codex = object.get("codex")?.as_object()?;
    codex.get("minVersion").and_then(trimmed_non_empty)
}

/// Ported from `codexModelCatalog.ts` (`codexUpdateRequiredModels`): current
/// catalog models the installed CLI predates, not already discovered by family.
/// `models` are the discovered `model/list` values (`"id"` carries the slug).
pub fn codex_update_required_models(
    catalog: Option<&ProviderCatalog>,
    version: Option<&str>,
    models: &[Value],
) -> Vec<UpdateRequiredModel> {
    let version = version.map(|version| version.strip_prefix('v').unwrap_or(version));
    let (Some(catalog), Some(version)) = (catalog, version) else {
        return Vec::new();
    };
    if parse_semver(version).is_none() {
        return Vec::new();
    }
    let discovered: HashSet<String> = models
        .iter()
        .filter_map(|model| model["id"].as_str())
        .map(|slug| codex_model_family(slug).to_owned())
        .collect();
    catalog
        .models
        .iter()
        .filter_map(|entry| {
            if entry.status != "current" || discovered.contains(codex_model_family(&entry.slug)) {
                return None;
            }
            let min_version = codex_adapter_min_version(&entry.adapter)?;
            if parse_semver(&min_version).is_none()
                || compare_semver_versions(version, &min_version) >= 0
            {
                return None;
            }
            Some(UpdateRequiredModel {
                slug: entry.slug.clone(),
                name: entry.name.clone(),
                badge: entry.badge.clone(),
                min_version,
            })
        })
        .collect()
}

/// KSG `formatCodexModelName`: preserve the reported name with GPT capitalization.
pub(crate) fn format_codex_model_name(name: &str) -> String {
    let mut name = name.to_owned();
    if name
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("gpt"))
    {
        name.replace_range(..3, "GPT");
    }
    let mut previous = '\0';
    name.chars()
        .map(|ch| {
            let out = if previous == '-' {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            previous = ch;
            out
        })
        .collect()
}

/// Ported from `codexModelCatalog.ts` (`applyCodexModelCatalog`): update-required
/// entries are computed before the catalog pass; `applyModelCatalog` classifies
/// `model/list` rows against the manifest without overwriting their names,
/// order or capabilities.
/// Returns the applied rows and the update-required entries (KSG sets the field
/// only when non-empty; the caller owns emission).
pub fn apply_codex_model_catalog(
    models: Vec<Value>,
    catalog: Option<&ProviderCatalog>,
    version: Option<&str>,
) -> (Vec<Value>, Vec<UpdateRequiredModel>) {
    let update_required = codex_update_required_models(catalog, version, &models);
    let models = apply_model_catalog(models, catalog, &CODEX_MATCHING);
    (models, update_required)
}

/// KSG `PREFERRED_DEFAULT_CODEX_MODELS`
/// (`CodexProvider.ts`): the first listed family present in discovery wins.
pub const PREFERRED_DEFAULT_CODEX_MODELS: [&str; 3] =
    ["gpt-6-astra", "gpt-5.6-sol", "gpt-5.6-terra"];

/// Ported from `CodexProvider.ts` (`applyPreferredCodexDefaultModel`): move
/// `isDefault` onto the first preferred family present in discovery.
pub fn apply_preferred_codex_default_model(models: &mut [Value]) {
    let preferred = PREFERRED_DEFAULT_CODEX_MODELS.iter().find_map(|target| {
        models
            .iter()
            .find(|model| {
                model["isCustom"] != true
                    && codex_model_family(model["id"].as_str().unwrap_or_default()) == *target
            })
            .and_then(|model| model["id"].as_str().map(str::to_owned))
    });
    if let Some(preferred_slug) = preferred {
        for model in models.iter_mut() {
            model["isDefault"] = json!(model["id"].as_str() == Some(preferred_slug.as_str()));
        }
    }
}

/// Ported from `CodexProvider.ts` (`codexRouteBaseUrl`): the base URL Codex's
/// merged config routes model traffic to. `Ok(None)` is Codex's auth-dependent
/// OpenAI default; `Err` is a route that cannot be read.
fn codex_route_base_url(config: &Value) -> Result<Option<String>, ()> {
    if !config.is_object() {
        return Err(());
    }
    let provider = config["model_provider"].as_str().unwrap_or("openai");
    if provider == "openai" {
        return Ok(config["openai_base_url"].as_str().map(str::to_owned));
    }
    let Some(entry) = config["model_providers"].get(provider) else {
        return Err(());
    };
    Ok(entry["base_url"].as_str().map(str::to_owned))
}

/// `(origin, host, scheme)` for an `http(s)` URL, or `None` when the string is
/// not one. The origin keeps the port but drops credentials and the path.
fn url_origin(base: &str) -> Option<(String, String, String)> {
    let (scheme, rest) = base.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
    if authority.is_empty() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            (host, Some(port))
        }
        _ => (authority, None),
    };
    if host.is_empty() {
        return None;
    }
    let origin = match port {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    };
    Some((origin, host.to_owned(), scheme.to_owned()))
}

/// Ported from `CodexProvider.ts` (`routesToNativeOpenai`): only a default
/// route or an `https` URL on OpenAI's own hosts spends native quota.
fn routes_to_native_openai(config: &Value) -> bool {
    match codex_route_base_url(config) {
        Err(()) => false,
        Ok(None) => true,
        Ok(Some(base)) => url_origin(&base).is_some_and(|(_, host, scheme)| {
            scheme == "https" && (host == "api.openai.com" || host == "chatgpt.com")
        }),
    }
}

/// Ported from `CodexProvider.ts` (`codexCustomEndpoint`): the origin of the
/// custom endpoint Codex sends model requests to, if it uses one.
pub(crate) fn codex_custom_endpoint(config: &Value) -> Option<String> {
    let base = codex_route_base_url(config).ok().flatten()?;
    if routes_to_native_openai(config) {
        return None;
    }
    url_origin(&base).map(|(origin, ..)| origin)
}

/// Ported from `CodexProvider.ts` (`codexRoutesSpendNativeQuota`): a stored
/// login does not mean this provider's traffic spends that login's quota.
pub(crate) fn codex_routes_spend_native_quota(account: &Value, config: &Value) -> bool {
    account["requiresOpenaiAuth"] == true
        && account["account"]["type"].as_str() != Some("apiKey")
        && routes_to_native_openai(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_catalog::ProviderCatalogModel;

    /// KSG `codexModelCatalog.test.ts`'s `model` helper, on Shina's model values.
    fn model(overrides: Value) -> Value {
        let mut model = json!({"id":"gpt-test","name":"GPT Test","isCustom":false});
        if let Some(overrides) = overrides.as_object() {
            for (key, value) in overrides {
                model[key] = value.clone();
            }
        }
        model
    }

    /// KSG `codexModelCatalog.test.ts`'s `entry` helper.
    fn catalog_model(
        slug: &str,
        status: &str,
        badge: Option<&str>,
        adapter: Value,
    ) -> ProviderCatalogModel {
        ProviderCatalogModel {
            slug: slug.into(),
            name: slug.into(),
            short_name: None,
            sub_provider: None,
            aliases: vec![],
            badge: badge.map(str::to_owned),
            status: status.into(),
            capabilities: Default::default(),
            adapter,
            profile_adapter: json!({}),
        }
    }

    fn catalog() -> ProviderCatalog {
        let mut gpt_next = catalog_model(
            "gpt-next",
            "current",
            Some("new"),
            json!({"codex":{"minVersion":"1.2.0"}}),
        );
        gpt_next.name = "GPT Next".into();
        ProviderCatalog {
            models: vec![
                gpt_next,
                catalog_model("gpt-unversioned", "current", None, json!({})),
                catalog_model(
                    "gpt-retired",
                    "legacy",
                    None,
                    json!({"codex":{"minVersion":"1.2.0"}}),
                ),
            ],
            default_chat_model: None,
        }
    }

    #[test]
    fn names_current_models_that_need_a_newer_codex_and_are_missing_from_discovery() {
        let catalog = catalog();
        let (models, update_required) =
            apply_codex_model_catalog(vec![], Some(&catalog), Some("1.1.9"));
        assert!(models.is_empty());
        assert_eq!(
            update_required,
            vec![UpdateRequiredModel {
                slug: "gpt-next".into(),
                name: "GPT Next".into(),
                badge: Some("new".into()),
                min_version: "1.2.0".into(),
            }]
        );
        for (version, models) in [
            (Some("1.2.0"), vec![]),
            (Some("1.1.9"), vec![model(json!({"id":"openai.gpt-next"}))]),
            (None, vec![]),
        ] {
            let (_, update_required) =
                apply_codex_model_catalog(models.clone(), Some(&catalog), version);
            assert!(update_required.is_empty(), "{version:?} {models:?}");
        }
        // A catalog slug already qualified still counts as discovered by family.
        let qualified = ProviderCatalog {
            models: vec![ProviderCatalogModel {
                slug: "openai.gpt-next".into(),
                ..catalog.models[0].clone()
            }],
            ..catalog
        };
        assert!(
            apply_codex_model_catalog(
                vec![model(json!({"id":"gpt-next"}))],
                Some(&qualified),
                Some("1.1.9"),
            )
            .1
            .is_empty()
        );
    }

    #[test]
    fn classifies_qualified_codex_families_without_changing_their_wire_ids() {
        let catalog = catalog();
        let (models, _) = apply_codex_model_catalog(
            vec![
                model(json!({"id":"openai.gpt-test","isLegacy":true})),
                model(json!({"id":"openai.gpt-retired"})),
            ],
            Some(&catalog),
            Some("1.2.0"),
        );
        let pairs: Vec<(String, bool)> = models
            .iter()
            .map(|model| {
                (
                    model["id"].as_str().unwrap().to_owned(),
                    model["isLegacy"] == true,
                )
            })
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("openai.gpt-test".to_owned(), false),
                ("openai.gpt-retired".to_owned(), true),
            ]
        );
    }

    #[test]
    fn cli_version_reads_the_first_slash_token_like_ksg() {
        assert_eq!(codex_cli_version("codex/0.167.2"), Some("0.167.2".into()));
        assert_eq!(
            codex_cli_version("fake-codex/0.150.0"),
            Some("0.150.0".into())
        );
        assert_eq!(
            codex_cli_version("codex-cli beta codex/1.2.3"),
            Some("1.2.3".into())
        );
        assert_eq!(codex_cli_version("a/ b/c"), Some("c".into()));
        assert_eq!(codex_cli_version("codex"), None);
        assert_eq!(codex_cli_version(""), None);
    }

    #[test]
    fn manifest_classifies_without_changing_wire_labels_capabilities_or_order() {
        let catalog = bundled_codex_provider_catalog(crate::lifecycle::now()).unwrap();
        let (models, _) = apply_codex_model_catalog(
            vec![
                model(
                    json!({"id":"openai.gpt-6-sol","name":"GPT-6 Sol wire","thinkingLevels":["high"]}),
                ),
                model(json!({"id":"gpt-6-astra","name":"GPT-6 Astra wire","isDefault":true})),
                model(json!({"id":"zzz-fixture","name":"Fixture Unknown"})),
            ],
            Some(&catalog),
            Some("9.9.9"),
        );
        let ids: Vec<&str> = models.iter().filter_map(|m| m["id"].as_str()).collect();
        assert_eq!(ids, ["openai.gpt-6-sol", "gpt-6-astra", "zzz-fixture"]);
        assert_eq!(models[0]["name"], "GPT-6 Sol wire");
        assert_eq!(models[0]["isLegacy"], true);
        assert_eq!(models[0]["thinkingLevels"], json!(["high"]));
        assert_eq!(models[1]["name"], "GPT-6 Astra wire");
        assert!(models[1].get("isLegacy").is_none());
        assert_eq!(models[1]["isDefault"], true);
        // Unknown rows keep their `model/list` data entirely.
        assert_eq!(models[2]["name"], "Fixture Unknown");
        assert!(models[2].get("isLegacy").is_none());
    }

    #[test]
    fn codex_wire_labels_use_ksg_capitalization() {
        assert_eq!(
            format_codex_model_name("gPt-6-astra (CPA)"),
            "GPT-6-Astra (CPA)"
        );
        assert_eq!(
            format_codex_model_name("model-x-4 (élan)"),
            "model-X-4 (élan)"
        );
    }

    #[test]
    fn bundled_catalog_names_update_required_models_from_the_manifest() {
        let catalog = bundled_codex_provider_catalog(crate::lifecycle::now()).unwrap();
        let (_, update_required) = apply_codex_model_catalog(
            vec![model(json!({"id":"gpt-6-astra","isDefault":true}))],
            Some(&catalog),
            Some("0.150.0"),
        );
        let slugs: Vec<&str> = update_required.iter().map(|m| m.slug.as_str()).collect();
        assert_eq!(slugs, ["gpt-6.1-sol", "gpt-6-luna"]);
        assert_eq!(update_required[0].min_version, "0.153.0");
        assert_eq!(update_required[1].name, "GPT-6 Luna");
        // A CLI at or above every minimum has nothing to report.
        let (_, none) = apply_codex_model_catalog(
            vec![model(json!({"id":"gpt-6-astra"}))],
            Some(&catalog),
            Some("0.155.0"),
        );
        assert!(none.is_empty());
    }

    #[test]
    fn badge_expires_on_each_catalog_read() {
        let manifest = json!({"providers":{"codex":{"profiles":{},"models":[
            {"slug":"model-next","name":"Model Next","status":"current",
             "releasedAt":"1970-01-01T00:00:00Z"},
            {"slug":"model-legacy","name":"Model Legacy","status":"legacy",
             "releasedAt":"1970-01-01T00:00:00Z"}
        ]}}});
        let fresh = resolve_provider_catalog(&manifest, "codex", 1).unwrap();
        assert_eq!(fresh.models[0].badge.as_deref(), Some("new"));
        assert_eq!(fresh.models[1].badge, None);
        let week_ms = 7 * 24 * 60 * 60 * 1000;
        let expired = resolve_provider_catalog(&manifest, "codex", week_ms + 1).unwrap();
        assert_eq!(expired.models[0].badge, None);
    }
}
