//! Selection resolution order (draft, thread, project, default) and sticky options store,
//! ported rule-for-rule from KSG Code's `composerDraftStore.ts`, `modelSelection.ts`,
//! `providerInstances.ts`, `ChatView.logic.ts`, and `packages/shared/src/model.ts`.

use crate::model_picker::{
    PickerModelOption, ProviderInstanceEntry, ProviderStatusKind,
    is_provider_instance_picker_ready, matches_locked_provider,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OptionValue {
    Bool(bool),
    Str(String),
}

impl OptionValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            Self::Bool(_) => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Str(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOptionSelection {
    pub id: String,
    pub value: OptionValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelection {
    pub instance_id: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ProviderOptionSelection>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftModelState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_selection_by_provider: BTreeMap<String, ModelSelection>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub model_selection_explicit: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_models: Vec<ModelSelection>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StickyComposerState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sticky_model_selection_by_provider: BTreeMap<String, ModelSelection>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sticky_options_by_model_by_provider:
        BTreeMap<String, BTreeMap<String, Vec<ProviderOptionSelection>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticky_active_provider: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveComposerModelState {
    pub selected_model: String,
    pub model_options: BTreeMap<String, Vec<ProviderOptionSelection>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedComposerProviderSelection {
    pub selected_instance_id: Option<String>,
    pub requested_instance_id: Option<String>,
    pub requested_driver_kind: String,
    pub locked_continuation_group_key: Option<String>,
}

pub const NO_PROVIDER_INSTANCE_ID: &str = "t3code_no_provider";

pub fn default_model_by_provider(driver_kind: &str) -> &'static str {
    match driver_kind {
        "codex" => "gpt-6-astra",
        "claude" | "claudeAgent" => "claude-opus-5-5",
        "cursor" => "auto",
        "grok" => "grok-build",
        "opencode" => "openai/gpt-5",
        "antigravity" => "antigravity-default",
        _ => "default",
    }
}

pub fn normalize_custom_model_slug(model: Option<&str>) -> Option<String> {
    let trimmed = model?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Ported from `packages/contracts/src/model.ts` (`MODEL_SLUG_ALIASES_BY_PROVIDER`) and `packages/shared/src/model.ts` (`normalizeModelSlug`).
pub fn normalize_model_slug(model: Option<&str>, driver_kind: &str) -> Option<String> {
    let trimmed = normalize_custom_model_slug(model)?;
    let mapped = match driver_kind {
        "codex" => match trimmed.as_str() {
            "gpt-5-codex" | "5.4" => Some("gpt-5.4"),
            "5.3" | "gpt-5.3" => Some("gpt-5.3-codex"),
            "5.3-spark" | "gpt-5.3-spark" => Some("gpt-5.3-codex-spark"),
            _ => None,
        },
        "cursor" => match trimmed.as_str() {
            "composer" => Some("composer-2"),
            "composer-1.5" | "composer-1" => Some("composer-1.5"),
            "opus-4.6-thinking" | "opus-4.6" => Some("claude-opus-4-6"),
            "sonnet-4.6-thinking" | "sonnet-4.6" => Some("claude-sonnet-4-6"),
            "opus-4.5-thinking" | "opus-4.5" => Some("claude-opus-4-5"),
            _ => None,
        },
        _ => None,
    };
    Some(mapped.map(str::to_owned).unwrap_or(trimmed))
}

/// Ported from `packages/shared/src/model.ts` (`resolveSelectableModel`).
pub fn resolve_selectable_model(
    driver_kind: &str,
    value: Option<&str>,
    options: &[PickerModelOption],
) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(direct) = options.iter().find(|o| o.slug == trimmed) {
        return Some(direct.slug.clone());
    }
    let lower = trimmed.to_lowercase();
    if let Some(by_name) = options.iter().find(|o| o.name.to_lowercase() == lower) {
        return Some(by_name.slug.clone());
    }
    if let Some(by_alias) = options
        .iter()
        .find(|o| o.aliases.iter().any(|a| a.to_lowercase() == lower))
    {
        return Some(by_alias.slug.clone());
    }
    let normalized = normalize_model_slug(Some(trimmed), driver_kind)?;
    options
        .iter()
        .find(|o| o.slug == normalized)
        .map(|o| o.slug.clone())
}

/// Ported from `apps/web/src/modelSelection.ts` (`resolveAppModelSelectionForInstance`).
pub fn resolve_app_model_selection_for_instance(
    instance_id: &str,
    entries: &[ProviderInstanceEntry],
    models_by_instance: &BTreeMap<String, Vec<PickerModelOption>>,
    selected_model: Option<&str>,
    preserve_unavailable_selection: bool,
) -> Option<String> {
    let entry = entries.iter().find(|c| c.instance_id == instance_id)?;
    let options = models_by_instance
        .get(instance_id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if let Some(resolved) = resolve_selectable_model(&entry.driver_kind, selected_model, options) {
        return Some(resolved);
    }
    if preserve_unavailable_selection
        && matches!(entry.driver_kind.as_str(), "opencode" | "antigravity")
        && let Some(unavailable) = normalize_custom_model_slug(selected_model)
        && resolve_selectable_model(&entry.driver_kind, selected_model, options).is_none()
        && (entry.driver_kind != "antigravity" || unavailable != "antigravity-default")
    {
        return Some(unavailable);
    }
    options
        .iter()
        .find(|o| o.is_default)
        .or_else(|| options.first())
        .map(|o| o.slug.clone())
}

fn is_selectable_provider_instance_entry(entry: &ProviderInstanceEntry) -> bool {
    entry.enabled && entry.is_available
}

/// Ported from `apps/web/src/providerInstances.ts` (`resolveSelectableProviderInstanceEntry`).
pub fn resolve_selectable_provider_instance_entry<'a>(
    entries: &'a [ProviderInstanceEntry],
    instance_id: Option<&str>,
) -> Option<&'a ProviderInstanceEntry> {
    if let Some(id) = instance_id
        && let Some(requested) = entries.iter().find(|e| e.instance_id == id)
        && is_selectable_provider_instance_entry(requested)
    {
        return Some(requested);
    }
    entries
        .iter()
        .find(|e| is_provider_instance_picker_ready(e))
        .or_else(|| {
            entries.iter().find(|e| {
                is_selectable_provider_instance_entry(e) && e.status != ProviderStatusKind::Error
            })
        })
}

/// Ported from `apps/web/src/components/ChatView.logic.ts` (`resolveComposerProviderSelection`).
pub fn resolve_composer_provider_selection(
    entries: &[ProviderInstanceEntry],
    candidate_instance_ids: &[Option<&str>],
    locked_provider: Option<&str>,
    locked_instance_id: Option<&str>,
) -> ResolvedComposerProviderSelection {
    let requested_instance_id = candidate_instance_ids
        .iter()
        .flatten()
        .find(|&&c| !c.is_empty() && c != NO_PROVIDER_INSTANCE_ID)
        .map(|&s| s.to_owned());
    let requested_driver_kind = locked_provider
        .map(str::to_owned)
        .or_else(|| {
            requested_instance_id.as_deref().and_then(|id| {
                entries
                    .iter()
                    .find(|e| e.instance_id == id)
                    .map(|e| e.driver_kind.clone())
            })
        })
        .or_else(|| entries.first().map(|e| e.driver_kind.clone()))
        .unwrap_or_else(|| "unconfigured".into());
    let locked_continuation_group_key = if locked_provider.is_some() {
        locked_instance_id.and_then(|id| {
            entries
                .iter()
                .find(|e| e.instance_id == id)
                .and_then(|e| e.continuation_group_key.clone())
        })
    } else {
        None
    };
    let requires_exact_instance = locked_provider == Some("antigravity")
        && locked_instance_id.is_some()
        && locked_continuation_group_key.is_none();

    let compatible_entries: Vec<ProviderInstanceEntry> = entries
        .iter()
        .filter(|entry| {
            matches_locked_provider(
                entry,
                locked_provider,
                locked_continuation_group_key.as_deref(),
            ) && (!requires_exact_instance
                || Some(entry.instance_id.as_str()) == locked_instance_id)
        })
        .cloned()
        .collect();

    let selected_entry = candidate_instance_ids
        .iter()
        .flatten()
        .find_map(|&candidate| {
            compatible_entries
                .iter()
                .find(|e| e.instance_id == candidate && e.enabled && e.is_available)
        })
        .or_else(|| {
            let same_driver: Vec<_> = compatible_entries
                .iter()
                .filter(|e| e.driver_kind == requested_driver_kind)
                .cloned()
                .collect();
            same_driver
                .iter()
                .find(|e| is_provider_instance_picker_ready(e))
                .or_else(|| {
                    same_driver.iter().find(|e| {
                        is_selectable_provider_instance_entry(e)
                            && e.status != ProviderStatusKind::Error
                    })
                })
                .and_then(|matched| {
                    compatible_entries
                        .iter()
                        .find(|e| e.instance_id == matched.instance_id)
                })
        })
        .or_else(|| resolve_selectable_provider_instance_entry(&compatible_entries, None));

    ResolvedComposerProviderSelection {
        selected_instance_id: selected_entry.map(|e| e.instance_id.clone()),
        requested_instance_id,
        requested_driver_kind,
        locked_continuation_group_key,
    }
}

fn get_default_server_model(
    models_by_instance: &BTreeMap<String, Vec<PickerModelOption>>,
    instance_id: Option<&str>,
    driver_kind: &str,
) -> String {
    let models = instance_id
        .and_then(|id| models_by_instance.get(id))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    models
        .iter()
        .find(|m| m.is_default && !m.is_custom)
        .or_else(|| models.iter().find(|m| !m.is_custom))
        .or_else(|| models.first())
        .map(|m| m.slug.clone())
        .unwrap_or_else(|| default_model_by_provider(driver_kind).to_owned())
}

fn selections_map_to_options(
    map: Option<&BTreeMap<String, ModelSelection>>,
) -> BTreeMap<String, Vec<ProviderOptionSelection>> {
    let mut result = BTreeMap::new();
    if let Some(map) = map {
        for (provider, sel) in map {
            if !sel.options.is_empty() {
                result.insert(provider.clone(), sel.options.clone());
            }
        }
    }
    result
}

fn model_identity(
    models_by_instance: &BTreeMap<String, Vec<PickerModelOption>>,
    instance_id: &str,
    driver_kind: &str,
    model: Option<&str>,
) -> Option<String> {
    let is_custom = models_by_instance.get(instance_id).is_some_and(|opts| {
        opts.iter()
            .any(|m| Some(m.slug.as_str()) == model && m.is_custom)
    });
    if is_custom {
        model.map(str::to_owned)
    } else {
        normalize_model_slug(model, driver_kind)
    }
}

/// Ported from `apps/web/src/composerDraftStore.ts` (`deriveEffectiveComposerModelState`).
pub fn derive_effective_composer_model_state(
    draft: Option<&DraftModelState>,
    entries: &[ProviderInstanceEntry],
    models_by_instance: &BTreeMap<String, Vec<PickerModelOption>>,
    selected_provider: &str,
    selected_instance_id: Option<&str>,
    requested_instance_id: Option<&str>,
    thread_model_selection: Option<&ModelSelection>,
    project_model_selection: Option<&ModelSelection>,
) -> EffectiveComposerModelState {
    let base_model_candidate = thread_model_selection
        .map(|s| s.model.as_str())
        .or_else(|| project_model_selection.map(|s| s.model.as_str()));
    let preserve_thread_model = selected_instance_id.is_some()
        && thread_model_selection.map(|s| s.instance_id.as_str()) == selected_instance_id;

    let base_model = selected_instance_id
        .and_then(|id| {
            resolve_app_model_selection_for_instance(
                id,
                entries,
                models_by_instance,
                base_model_candidate,
                preserve_thread_model,
            )
        })
        .or_else(|| {
            if selected_provider == "antigravity" && selected_instance_id.is_some() {
                Some(String::new())
            } else {
                None
            }
        })
        .or_else(|| normalize_model_slug(base_model_candidate, selected_provider))
        .unwrap_or_else(|| {
            get_default_server_model(models_by_instance, selected_instance_id, selected_provider)
        });

    let instance_selection = selected_instance_id
        .and_then(|id| draft.and_then(|d| d.model_selection_by_provider.get(id)));
    let selected_model = if let Some(sel) = instance_selection {
        let sel_instance = selected_instance_id.unwrap_or(selected_provider);
        resolve_app_model_selection_for_instance(
            sel_instance,
            entries,
            models_by_instance,
            Some(&sel.model),
            true,
        )
        .or_else(|| {
            if selected_provider == "antigravity" {
                Some(String::new())
            } else {
                normalize_model_slug(Some(&sel.model), selected_provider)
            }
        })
        .unwrap_or_else(|| sel.model.clone())
    } else {
        base_model
    };

    // Draft options win; otherwise an existing thread's options, even none, outrank the project's.
    let draft_options = selections_map_to_options(draft.map(|d| &d.model_selection_by_provider));
    let mut option_selections_map = BTreeMap::new();
    let mut model_options = if !draft_options.is_empty() {
        if let Some(d) = draft {
            option_selections_map = d.model_selection_by_provider.clone();
        }
        draft_options
    } else {
        let fallback_sel = thread_model_selection.or(project_model_selection);
        if let Some(sel) = fallback_sel {
            option_selections_map.insert(sel.instance_id.clone(), sel.clone());
        }
        selections_map_to_options(Some(&option_selections_map))
    };

    // withFallbackInstanceOptions
    if let (Some(sel_id), Some(req_id)) = (selected_instance_id, requested_instance_id)
        && sel_id != req_id
        && !model_options.contains_key(sel_id)
        && let Some(req_opts) = model_options.get(req_id).cloned()
    {
        let req_driver = entries
            .iter()
            .find(|e| e.instance_id == req_id)
            .map(|e| e.driver_kind.as_str());
        let req_model = option_selections_map.get(req_id).map(|s| s.model.as_str());
        let same_model = req_model == Some(selected_model.as_str())
            || model_identity(models_by_instance, req_id, selected_provider, req_model)
                == model_identity(
                    models_by_instance,
                    sel_id,
                    selected_provider,
                    Some(&selected_model),
                );
        if req_driver == Some(selected_provider) && same_model {
            model_options.insert(sel_id.to_owned(), req_opts);
        }
    }

    EffectiveComposerModelState {
        selected_model,
        model_options,
    }
}

#[cfg(test)]
fn canonical_options(sel: &ModelSelection) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = sel
        .options
        .iter()
        .map(|o| {
            let v = match &o.value {
                OptionValue::Bool(b) => b.to_string(),
                OptionValue::Str(s) => s.clone(),
            };
            (o.id.clone(), v)
        })
        .collect();
    pairs.sort();
    pairs
}

#[cfg(test)]
pub fn model_selections_equal(left: &ModelSelection, right: &ModelSelection) -> bool {
    left.instance_id == right.instance_id
        && left.model == right.model
        && canonical_options(left) == canonical_options(right)
}

#[cfg(test)]
/// Ported from `packages/shared/src/model.ts` (`draftModelPickMatchesThread`).
pub fn draft_model_pick_matches_thread(pick: &ModelSelection, thread: &ModelSelection) -> bool {
    let strip_fast_off = |sel: &ModelSelection| ModelSelection {
        instance_id: sel.instance_id.clone(),
        model: sel.model.clone(),
        options: sel
            .options
            .iter()
            .filter(|o| o.id != "fastMode" || o.value != OptionValue::Bool(false))
            .cloned()
            .collect(),
    };
    model_selections_equal(&strip_fast_off(pick), &strip_fast_off(thread))
}

/// Ported from `apps/web/src/composerDraftStore.ts` (`setModelSelection`).
pub fn set_model_selection(
    draft: &mut DraftModelState,
    selection: Option<ModelSelection>,
    explicit: bool,
    replace_options: bool,
) {
    let Some(sel) = selection else {
        return;
    };
    let instance_id = sel.instance_id.clone();
    if !sel.options.is_empty() || replace_options {
        draft
            .model_selection_by_provider
            .insert(instance_id.clone(), sel);
    } else {
        let existing_opts = draft
            .model_selection_by_provider
            .get(&instance_id)
            .map(|c| c.options.clone())
            .unwrap_or_default();
        draft.model_selection_by_provider.insert(
            instance_id.clone(),
            ModelSelection {
                instance_id: sel.instance_id,
                model: sel.model,
                options: existing_opts,
            },
        );
    }
    draft.active_provider = Some(instance_id);
    draft.model_selection_explicit = explicit;
}

/// Ported from `apps/web/src/composerDraftStore.ts` (`setStickyModelSelection`).
pub fn set_sticky_model_selection(sticky: &mut StickyComposerState, selection: ModelSelection) {
    let instance_id = selection.instance_id.clone();
    let next = if !selection.options.is_empty() {
        selection
    } else {
        let existing_opts = sticky
            .sticky_model_selection_by_provider
            .get(&instance_id)
            .map(|c| c.options.clone())
            .unwrap_or_default();
        ModelSelection {
            instance_id: selection.instance_id,
            model: selection.model,
            options: existing_opts,
        }
    };
    sticky
        .sticky_model_selection_by_provider
        .insert(instance_id.clone(), next);
    sticky.sticky_active_provider = Some(instance_id);
}

/// Ported from `apps/web/src/composerDraftStore.ts` (`applyStickyState`).
pub fn apply_sticky_state(draft: &mut DraftModelState, sticky: &StickyComposerState) {
    draft.model_selection_by_provider = sticky.sticky_model_selection_by_provider.clone();
    draft.active_provider = sticky.sticky_active_provider.clone();
    draft.model_selection_explicit = false;
}

/// Ported from `apps/web/src/composerDraftStore.ts` (`setProviderModelOptions`).
pub fn set_provider_model_options(
    draft: &mut DraftModelState,
    sticky: &mut StickyComposerState,
    instance_id: &str,
    driver_kind: &str,
    model: Option<&str>,
    next_options: Vec<ProviderOptionSelection>,
    persist_sticky: bool,
) {
    let fallback_model = normalize_custom_model_slug(model)
        .unwrap_or_else(|| default_model_by_provider(driver_kind).to_owned());
    let current_model = draft
        .model_selection_by_provider
        .get(instance_id)
        .map(|s| s.model.clone())
        .unwrap_or_else(|| fallback_model.clone());
    draft.model_selection_by_provider.insert(
        instance_id.to_owned(),
        ModelSelection {
            instance_id: instance_id.to_owned(),
            model: current_model,
            options: next_options.clone(),
        },
    );
    draft.active_provider = Some(instance_id.to_owned());
    draft.model_selection_explicit = true;

    if persist_sticky {
        let sticky_base = sticky
            .sticky_model_selection_by_provider
            .get(instance_id)
            .cloned()
            .or_else(|| draft.model_selection_by_provider.get(instance_id).cloned())
            .unwrap_or_else(|| ModelSelection {
                instance_id: instance_id.to_owned(),
                model: fallback_model,
                options: vec![],
            });
        let remembered_model =
            normalize_custom_model_slug(model).unwrap_or_else(|| sticky_base.model.clone());
        if !next_options.is_empty() {
            sticky.sticky_model_selection_by_provider.insert(
                instance_id.to_owned(),
                ModelSelection {
                    instance_id: instance_id.to_owned(),
                    model: sticky_base.model,
                    options: next_options.clone(),
                },
            );
            sticky
                .sticky_options_by_model_by_provider
                .entry(instance_id.to_owned())
                .or_default()
                .insert(remembered_model, next_options);
        } else {
            sticky.sticky_model_selection_by_provider.insert(
                instance_id.to_owned(),
                ModelSelection {
                    instance_id: instance_id.to_owned(),
                    model: sticky_base.model,
                    options: vec![],
                },
            );
            if let Some(by_model) = sticky
                .sticky_options_by_model_by_provider
                .get_mut(instance_id)
            {
                by_model.remove(&remembered_model);
            }
        }
        sticky.sticky_active_provider = Some(instance_id.to_owned());
    }
}

/// Ported from `apps/web/src/composerDraftStore.ts` (`releaseModelSelection`).
pub fn release_model_selection(draft: &mut DraftModelState, confirmed: &ModelSelection) -> bool {
    let Some(active) = &draft.active_provider else {
        return false;
    };
    let Some(pick) = draft.model_selection_by_provider.get(active) else {
        return false;
    };
    if pick != confirmed {
        return false;
    }
    draft.active_provider = None;
    draft.model_selection_by_provider.clear();
    draft.model_selection_explicit = false;
    true
}

/// Ported from `apps/web/src/components/chat/useComposerSelection.ts` (`onProviderModelSelect`).
pub fn select_model_from_picker(
    draft: &mut DraftModelState,
    sticky: &mut StickyComposerState,
    entries: &[ProviderInstanceEntry],
    models_by_instance: &BTreeMap<String, Vec<PickerModelOption>>,
    instance_id: &str,
    model: &str,
) -> Option<ModelSelection> {
    let resolved_model = resolve_app_model_selection_for_instance(
        instance_id,
        entries,
        models_by_instance,
        Some(model),
        false,
    )?;
    let remembered_options = sticky
        .sticky_options_by_model_by_provider
        .get(instance_id)
        .and_then(|by_model| by_model.get(&resolved_model))
        .cloned()
        .unwrap_or_default();
    let next_selection = ModelSelection {
        instance_id: instance_id.to_owned(),
        model: resolved_model,
        options: remembered_options,
    };
    set_model_selection(draft, Some(next_selection.clone()), true, true);
    draft.additional_models.clear();
    set_sticky_model_selection(sticky, next_selection.clone());
    Some(next_selection)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MultiModelToggleResult {
    Unchanged,
    Updated {
        new_primary: Option<ModelSelection>,
        additional: Vec<ModelSelection>,
    },
}

fn resolved_picker_slug_for_selection(
    entries: &[ProviderInstanceEntry],
    models_by_instance: &BTreeMap<String, Vec<PickerModelOption>>,
    instance_id: &str,
    model: &str,
) -> String {
    resolve_app_model_selection_for_instance(
        instance_id,
        entries,
        models_by_instance,
        Some(model),
        true,
    )
    .unwrap_or_else(|| model.to_owned())
}

/// Ordered multi-model selections: primary selection followed by unique additional selections.
pub fn multi_model_selections(
    primary: &ModelSelection,
    additional: &[ModelSelection],
) -> Vec<ModelSelection> {
    let mut out = vec![primary.clone()];
    for sel in additional {
        if !out
            .iter()
            .any(|existing| existing.instance_id == sel.instance_id && existing.model == sel.model)
        {
            out.push(sel.clone());
        }
    }
    out
}

/// Ported from KSG Code's `ComposerFooterControls.tsx` (`toggleModel`):
/// clicking a row's multi-select toggle adds `createModelSelection(instanceId, model)` or removes
/// an already-selected model while keeping at least one model selected.
pub fn toggle_multi_model_selection(
    primary: &ModelSelection,
    additional: &[ModelSelection],
    entries: &[ProviderInstanceEntry],
    models_by_instance: &BTreeMap<String, Vec<PickerModelOption>>,
    instance_id: &str,
    model: &str,
) -> MultiModelToggleResult {
    let resolved_model = resolve_app_model_selection_for_instance(
        instance_id,
        entries,
        models_by_instance,
        Some(model),
        false,
    )
    .unwrap_or_else(|| model.to_owned());
    if resolved_model.is_empty() {
        return MultiModelToggleResult::Unchanged;
    }

    let current = multi_model_selections(primary, additional);
    let matches = |sel: &ModelSelection| {
        sel.instance_id == instance_id
            && resolved_picker_slug_for_selection(
                entries,
                models_by_instance,
                &sel.instance_id,
                &sel.model,
            ) == resolved_model
    };
    if let Some(removed_index) = current.iter().position(matches) {
        if current.len() <= 1 {
            return MultiModelToggleResult::Unchanged;
        }
        let mut remaining: Vec<ModelSelection> =
            current.into_iter().filter(|sel| !matches(sel)).collect();
        if remaining.is_empty() {
            return MultiModelToggleResult::Unchanged;
        }
        if removed_index == 0 {
            let next_primary = remaining.remove(0);
            MultiModelToggleResult::Updated {
                new_primary: Some(next_primary),
                additional: remaining,
            }
        } else {
            MultiModelToggleResult::Updated {
                new_primary: None,
                additional: remaining.into_iter().skip(1).collect(),
            }
        }
    } else {
        let mut next_additional = current.into_iter().skip(1).collect::<Vec<_>>();
        next_additional.push(ModelSelection {
            instance_id: instance_id.to_owned(),
            model: resolved_model,
            options: Vec::new(),
        });
        MultiModelToggleResult::Updated {
            new_primary: None,
            additional: next_additional,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultipleModelSendBlock {
    ServerOutdated,
    NeedsNewGitThread,
}

/// Ported from KSG Code's `components/chat/composerSend.logic.ts` (`multipleModelSendBlock`).
pub fn multiple_model_send_block(
    selection_count: usize,
    server_supports_worktree_bootstrap: bool,
    is_local_draft_thread: bool,
    is_git_repo: bool,
    branch: Option<&str>,
) -> Option<MultipleModelSendBlock> {
    if !server_supports_worktree_bootstrap {
        return Some(MultipleModelSendBlock::ServerOutdated);
    }
    if !is_local_draft_thread
        || !is_git_repo
        || branch.is_none_or(|b| b.trim().is_empty())
        || selection_count == 0
    {
        return Some(MultipleModelSendBlock::NeedsNewGitThread);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_picker::{ProviderAuthStatus, ProviderStatusKind};

    fn opt_str(id: &str, val: &str) -> ProviderOptionSelection {
        ProviderOptionSelection {
            id: id.into(),
            value: OptionValue::Str(val.into()),
        }
    }

    fn opt_bool(id: &str, val: bool) -> ProviderOptionSelection {
        ProviderOptionSelection {
            id: id.into(),
            value: OptionValue::Bool(val),
        }
    }

    fn entry(instance_id: &str, driver_kind: &str, available: bool) -> ProviderInstanceEntry {
        ProviderInstanceEntry {
            instance_id: instance_id.into(),
            driver_kind: driver_kind.into(),
            display_name: instance_id.into(),
            continuation_group_key: None,
            enabled: available,
            installed: true,
            status: if available {
                ProviderStatusKind::Ready
            } else {
                ProviderStatusKind::Disabled
            },
            auth_status: ProviderAuthStatus::Authenticated,
            can_setup: false,
            is_available: available,
            is_acp: false,
            message: None,
            compatibility_broken_message: None,
            update_required_models: vec![],
        }
    }

    fn model_opt(slug: &str, is_default: bool) -> PickerModelOption {
        PickerModelOption {
            slug: slug.into(),
            name: slug.into(),
            short_name: None,
            sub_provider: None,
            aliases: vec![],
            badge: None,
            is_default,
            is_custom: false,
            is_legacy: false,
            is_unavailable: false,
        }
    }

    #[test]
    fn selection_resolves_in_ksg_order_draft_thread_project_default() {
        let entries = vec![
            entry("pi", "pi", true),
            entry("codex", "codex", true),
            entry("claude", "claude", true),
        ];
        let mut models = BTreeMap::new();
        models.insert("pi".into(), vec![model_opt("cpa/default", true)]);
        models.insert(
            "codex".into(),
            vec![
                model_opt("gpt-6-astra", true),
                model_opt("gpt-5.6-sol", false),
            ],
        );
        models.insert("claude".into(), vec![model_opt("claude-opus-5-5", true)]);

        let project_sel = ModelSelection {
            instance_id: "codex".into(),
            model: "gpt-5.6-sol".into(),
            options: vec![opt_str("reasoningEffort", "medium")],
        };
        let thread_sel = ModelSelection {
            instance_id: "codex".into(),
            model: "gpt-6-astra".into(),
            options: vec![],
        };

        let prov_default =
            resolve_composer_provider_selection(&entries, &[None, None, None], None, None);
        assert_eq!(prov_default.selected_instance_id.as_deref(), Some("pi"));
        let eff_default = derive_effective_composer_model_state(
            None,
            &entries,
            &models,
            "pi",
            Some("pi"),
            None,
            None,
            None,
        );
        assert_eq!(eff_default.selected_model, "cpa/default");

        let prov_project =
            resolve_composer_provider_selection(&entries, &[None, None, Some("codex")], None, None);
        assert_eq!(prov_project.selected_instance_id.as_deref(), Some("codex"));
        let eff_project = derive_effective_composer_model_state(
            None,
            &entries,
            &models,
            "codex",
            Some("codex"),
            Some("codex"),
            None,
            Some(&project_sel),
        );
        assert_eq!(eff_project.selected_model, "gpt-5.6-sol");
        assert_eq!(
            eff_project.model_options.get("codex"),
            Some(&vec![opt_str("reasoningEffort", "medium")])
        );

        let eff_thread = derive_effective_composer_model_state(
            None,
            &entries,
            &models,
            "codex",
            Some("codex"),
            Some("codex"),
            Some(&thread_sel),
            Some(&project_sel),
        );
        assert_eq!(eff_thread.selected_model, "gpt-6-astra");
        assert!(eff_thread.model_options.is_empty());

        let mut draft = DraftModelState::default();
        set_model_selection(
            &mut draft,
            Some(ModelSelection {
                instance_id: "claude".into(),
                model: "claude-opus-5-5".into(),
                options: vec![opt_str("effort", "high")],
            }),
            true,
            true,
        );
        let prov_draft = resolve_composer_provider_selection(
            &entries,
            &[
                draft.active_provider.as_deref(),
                Some("codex"),
                Some("codex"),
            ],
            None,
            None,
        );
        assert_eq!(prov_draft.selected_instance_id.as_deref(), Some("claude"));
        let eff_draft = derive_effective_composer_model_state(
            Some(&draft),
            &entries,
            &models,
            "claude",
            Some("claude"),
            Some("claude"),
            Some(&thread_sel),
            Some(&project_sel),
        );
        assert_eq!(eff_draft.selected_model, "claude-opus-5-5");
        assert_eq!(
            eff_draft.model_options.get("claude"),
            Some(&vec![opt_str("effort", "high")])
        );
    }

    #[test]
    fn fallback_instance_inherits_options_when_showing_same_model() {
        let entries = vec![
            entry("codex_secondary", "codex", false),
            entry("codex", "codex", true),
        ];
        let mut models = BTreeMap::new();
        models.insert("codex".into(), vec![model_opt("gpt-5.4", true)]);
        let mut draft = DraftModelState::default();
        set_model_selection(
            &mut draft,
            Some(ModelSelection {
                instance_id: "codex_secondary".into(),
                model: "5.4".into(),
                options: vec![opt_str("reasoningEffort", "high")],
            }),
            true,
            true,
        );
        let eff = derive_effective_composer_model_state(
            Some(&draft),
            &entries,
            &models,
            "codex",
            Some("codex"),
            Some("codex_secondary"),
            None,
            None,
        );
        assert_eq!(eff.selected_model, "gpt-5.4");
        assert_eq!(
            eff.model_options.get("codex"),
            Some(&vec![opt_str("reasoningEffort", "high")])
        );
    }

    // Ported from KSG Code `composerDraftStore.test.ts` (`per-model sticky options`, `sticky composer settings`).
    #[test]
    fn sticky_options_per_model_and_sticky_state_match_ksg() {
        let entries = vec![
            entry("codex", "codex", true),
            entry("cursor", "cursor", true),
        ];
        let mut models = BTreeMap::new();
        models.insert(
            "codex".into(),
            vec![
                model_opt("gpt-5.3-codex", false),
                model_opt("gpt-5.4", true),
            ],
        );

        let mut draft = DraftModelState::default();
        let mut sticky = StickyComposerState::default();

        set_provider_model_options(
            &mut draft,
            &mut sticky,
            "codex",
            "codex",
            Some("gpt-5.3-codex"),
            vec![opt_str("reasoningEffort", "xhigh")],
            true,
        );
        set_provider_model_options(
            &mut draft,
            &mut sticky,
            "codex",
            "codex",
            Some("gpt-5.4"),
            vec![opt_str("reasoningEffort", "high")],
            true,
        );
        assert_eq!(
            sticky
                .sticky_options_by_model_by_provider
                .get("codex")
                .and_then(|m| m.get("gpt-5.3-codex")),
            Some(&vec![opt_str("reasoningEffort", "xhigh")])
        );
        assert_eq!(
            sticky
                .sticky_options_by_model_by_provider
                .get("codex")
                .and_then(|m| m.get("gpt-5.4")),
            Some(&vec![opt_str("reasoningEffort", "high")])
        );

        let picked = select_model_from_picker(
            &mut draft,
            &mut sticky,
            &entries,
            &models,
            "codex",
            "gpt-5.3-codex",
        )
        .unwrap();
        assert_eq!(picked.options, vec![opt_str("reasoningEffort", "xhigh")]);
        assert_eq!(
            draft
                .model_selection_by_provider
                .get("codex")
                .unwrap()
                .options,
            vec![opt_str("reasoningEffort", "xhigh")]
        );

        set_sticky_model_selection(
            &mut sticky,
            ModelSelection {
                instance_id: "cursor".into(),
                model: "composer-2".into(),
                options: vec![opt_bool("fastMode", false)],
            },
        );
        set_sticky_model_selection(
            &mut sticky,
            ModelSelection {
                instance_id: "cursor".into(),
                model: "composer-2.5".into(),
                options: vec![],
            },
        );
        assert_eq!(
            sticky.sticky_model_selection_by_provider.get("cursor"),
            Some(&ModelSelection {
                instance_id: "cursor".into(),
                model: "composer-2.5".into(),
                options: vec![opt_bool("fastMode", false)],
            })
        );

        let mut new_draft = DraftModelState {
            active_provider: Some("codex".into()),
            model_selection_by_provider: BTreeMap::from([(
                "codex".into(),
                ModelSelection {
                    instance_id: "codex".into(),
                    model: "stale-model".into(),
                    options: vec![opt_str("reasoningEffort", "low")],
                },
            )]),
            model_selection_explicit: false,
            additional_models: vec![],
        };
        apply_sticky_state(&mut new_draft, &sticky);
        assert_eq!(new_draft.active_provider.as_deref(), Some("cursor"));
        assert!(!new_draft.model_selection_explicit);

        let empty_sticky = StickyComposerState::default();
        apply_sticky_state(&mut new_draft, &empty_sticky);
        assert!(new_draft.active_provider.is_none());
        assert!(new_draft.model_selection_by_provider.is_empty());
    }

    #[test]
    fn release_model_selection_treats_fast_mode_false_as_unset_and_ignores_superseded_picks() {
        let mut draft = DraftModelState::default();
        let pick = ModelSelection {
            instance_id: "codex".into(),
            model: "gpt-5.5".into(),
            options: vec![opt_bool("fastMode", false)],
        };
        set_model_selection(&mut draft, Some(pick.clone()), true, true);
        let server_echo = ModelSelection {
            instance_id: "codex".into(),
            model: "gpt-5.5".into(),
            options: vec![],
        };
        assert!(draft_model_pick_matches_thread(&pick, &server_echo));

        let older = ModelSelection {
            instance_id: "codex".into(),
            model: "gpt-5.4".into(),
            options: vec![],
        };
        assert!(!release_model_selection(&mut draft, &older));
        assert!(draft.model_selection_explicit);

        assert!(release_model_selection(&mut draft, &pick));
        assert!(draft.active_provider.is_none());
        assert!(draft.model_selection_by_provider.is_empty());
        assert!(!draft.model_selection_explicit);
    }

    #[test]
    fn multi_model_toggle_adds_removes_keeps_last_and_roundtrips_json() {
        let entries = vec![
            entry("codex", "codex", true),
            entry("claude", "claudeAgent", true),
        ];
        let mut models = BTreeMap::new();
        models.insert(
            "codex".into(),
            vec![
                model_opt("gpt-6-astra", true),
                model_opt("gpt-6-sol", false),
            ],
        );
        models.insert(
            "claude".into(),
            vec![
                model_opt("claude-opus-5-5", true),
                model_opt("claude-sonnet-5-5", false),
            ],
        );

        let primary = ModelSelection {
            instance_id: "codex".into(),
            model: "gpt-6-astra".into(),
            options: vec![opt_str("reasoningEffort", "high")],
        };

        // Toggling the sole primary model is a no-op.
        assert_eq!(
            toggle_multi_model_selection(&primary, &[], &entries, &models, "codex", "gpt-6-astra",),
            MultiModelToggleResult::Unchanged
        );

        // Adding additional models preserves primary options and starts additions with empty options.
        let MultiModelToggleResult::Updated {
            new_primary,
            additional,
        } = toggle_multi_model_selection(
            &primary,
            &[],
            &entries,
            &models,
            "claude",
            "claude-opus-5-5",
        )
        else {
            panic!("expected Updated");
        };
        assert!(new_primary.is_none());
        assert_eq!(
            additional,
            vec![ModelSelection {
                instance_id: "claude".into(),
                model: "claude-opus-5-5".into(),
                options: vec![],
            }]
        );

        // Removing the primary model promotes the next model to primary.
        let MultiModelToggleResult::Updated {
            new_primary,
            additional,
        } = toggle_multi_model_selection(
            &primary,
            &additional,
            &entries,
            &models,
            "codex",
            "gpt-6-astra",
        )
        else {
            panic!("expected Updated");
        };
        assert_eq!(
            new_primary,
            Some(ModelSelection {
                instance_id: "claude".into(),
                model: "claude-opus-5-5".into(),
                options: vec![],
            })
        );
        assert!(additional.is_empty());

        // Round-trips DraftModelState with additionalModels through JSON.
        let draft = DraftModelState {
            active_provider: Some("codex".into()),
            model_selection_by_provider: BTreeMap::from([("codex".into(), primary)]),
            model_selection_explicit: true,
            additional_models: vec![ModelSelection {
                instance_id: "claude".into(),
                model: "claude-opus-5-5".into(),
                options: vec![opt_str("effort", "medium")],
            }],
        };
        let encoded = serde_json::to_value(&draft).unwrap();
        let decoded: DraftModelState = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, draft);
    }

    // Ported from KSG Code `components/chat/composerSend.logic.test.ts` (`multipleModelSendBlock`).
    #[test]
    fn multiple_model_send_block_matches_ksg_test_cases() {
        assert_eq!(
            multiple_model_send_block(2, true, true, true, Some("main")),
            None
        );
        assert_eq!(
            multiple_model_send_block(2, false, true, false, Some("main")),
            Some(MultipleModelSendBlock::ServerOutdated)
        );
        assert_eq!(
            multiple_model_send_block(2, true, false, true, Some("main")),
            Some(MultipleModelSendBlock::NeedsNewGitThread)
        );
        assert_eq!(
            multiple_model_send_block(2, true, true, false, Some("main")),
            Some(MultipleModelSendBlock::NeedsNewGitThread)
        );
        assert_eq!(
            multiple_model_send_block(2, true, true, true, None),
            Some(MultipleModelSendBlock::NeedsNewGitThread)
        );
        assert_eq!(
            multiple_model_send_block(0, true, true, true, Some("main")),
            Some(MultipleModelSendBlock::NeedsNewGitThread)
        );
    }
}
