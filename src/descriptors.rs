//! Option descriptor contracts and helpers, ported rule-for-rule from KSG Code:
//! - `packages/contracts/src/model.ts`
//! - `packages/shared/src/model.ts`
//! - `packages/client-runtime/src/modelTraits.ts`
//! - `apps/web/src/components/chat/TraitsPicker.tsx`
//! - `apps/web/src/components/chat/ComposerModelEffort.tsx`
//! - `packages/provider-pi/src/server/modelCapabilities.ts`
//! - `apps/server/src/provider/CodexProvider.ts`
//! - `apps/server/src/provider/ClaudeModelCatalog.ts`
//! - `packages/provider-acp/src/server/sessionConfig.ts`

use crate::model_selection::{
    ModelSelection, OptionValue, ProviderOptionSelection, normalize_custom_model_slug,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::LazyLock;

pub const ULTRATHINK_IN_BODY_NOTE: &str =
    "Your prompt contains \"ultrathink\" in the text. Remove it to change this option.";
pub const ACP_SESSION_MODE_OPTION_ID: &str = "_t3/session-mode";
pub const ACP_EMPTY_OPTION_VALUE: &str = "_t3/empty";

const MODEL_MANIFEST_JSON: &str = include_str!("../assets/model-manifest.json");
const REASONING_EFFORT_OPTION_IDS: &[&str] =
    &["reasoningEffort", "effort", "reasoning", "thought_level"];
const PI_THINKING_LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];
const PI_STANDARD_SERVICE_TIER: &str = "default";
const DEFAULT_SERVICE_TIER_ID: &str = "default";
const MAX_ACP_OPTION_DESCRIPTORS: usize = 16;
const MAX_ACP_OPTION_CHOICES: usize = 64;
const MAX_ACP_TEXT_LENGTH: usize = 256;
const MAX_ACP_DESCRIPTION_LENGTH: usize = 1024;

static ULTRATHINK_WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bultrathink\b").expect("valid ultrathink regex"));
static ULTRATHINK_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^Ultrathink:\s*").expect("valid prefix regex"));
static SLASH_COMMAND_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/[^\s/]+(?:\s|$)").expect("valid slash regex"));

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOptionChoice {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_default: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderOptionRole {
    Reasoning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ProviderOptionDescriptor {
    #[serde(rename_all = "camelCase")]
    Select {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        options: Vec<ProviderOptionChoice>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_value: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prompt_injected_values: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<ProviderOptionRole>,
    },
    #[serde(rename_all = "camelCase")]
    Boolean {
        id: String,
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_value: Option<bool>,
    },
}

impl ProviderOptionDescriptor {
    pub fn id(&self) -> &str {
        match self {
            Self::Select { id, .. } | Self::Boolean { id, .. } => id,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Select { label, .. } | Self::Boolean { label, .. } => label,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub option_descriptors: Vec<ProviderOptionDescriptor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptUltrathink {
    None,
    Prefix,
    Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitsTriggerDisplay {
    pub label: String,
    pub speed_label: Option<String>,
    pub ultracode: bool,
    pub context_window: Option<String>,
    pub effort_label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ColumnControls {
    pub effort: Option<ProviderOptionDescriptor>,
    pub others: Vec<ProviderOptionDescriptor>,
    pub speed_select: Option<ProviderOptionDescriptor>,
    pub fast_toggle: Option<ProviderOptionDescriptor>,
    pub ultracode_toggle: Option<ProviderOptionDescriptor>,
}

fn raw_selection_value<'a>(
    selections: &'a [ProviderOptionSelection],
    id: &str,
) -> Option<&'a OptionValue> {
    selections.iter().find(|s| s.id == id).map(|s| &s.value)
}

/// Ported from `packages/shared/src/model.ts` (`withLegacyUltracodeEffort`).
pub fn with_legacy_ultracode_effort(
    descriptors: &[ProviderOptionDescriptor],
    selections: &[ProviderOptionSelection],
) -> Vec<ProviderOptionSelection> {
    let has_legacy_effort = raw_selection_value(selections, "effort").and_then(OptionValue::as_str)
        == Some("ultracode");
    let has_ultracode_toggle = descriptors
        .iter()
        .any(|d| matches!(d, ProviderOptionDescriptor::Boolean { id, .. } if id == "ultracode"));
    if !has_legacy_effort || !has_ultracode_toggle {
        return selections.to_vec();
    }
    let mut out: Vec<ProviderOptionSelection> = selections
        .iter()
        .map(|s| {
            if s.id == "effort" && s.value == OptionValue::Str("ultracode".into()) {
                ProviderOptionSelection {
                    id: "effort".into(),
                    value: OptionValue::Str("xhigh".into()),
                }
            } else {
                s.clone()
            }
        })
        .collect();
    if raw_selection_value(selections, "ultracode").is_none() {
        out.push(ProviderOptionSelection {
            id: "ultracode".into(),
            value: OptionValue::Bool(true),
        });
    }
    out
}

/// Ported from `apps/server/src/provider/ClaudeModelCatalog.ts` (`withUltracodeToggle`).
pub fn with_ultracode_toggle(caps: ModelCapabilities) -> ModelCapabilities {
    let mut descriptors = caps.option_descriptors;
    let Some(effort_index) = descriptors.iter().position(|d| {
        matches!(
            d,
            ProviderOptionDescriptor::Select { id, options, .. }
                if id == "effort" && options.iter().any(|o| o.id == "ultracode")
        )
    }) else {
        return ModelCapabilities {
            option_descriptors: descriptors,
        };
    };
    let ProviderOptionDescriptor::Select {
        id,
        label,
        description,
        options,
        current_value,
        prompt_injected_values,
        role,
    } = descriptors.remove(effort_index)
    else {
        unreachable!()
    };
    let default_id = current_value
        .as_deref()
        .or_else(|| options.iter().find(|o| o.is_default).map(|o| o.id.as_str()));
    let ultracode_by_default = default_id == Some("ultracode");
    let has_xhigh = options.iter().any(|o| o.id == "xhigh");
    let mut next_options = Vec::new();
    for mut option in options {
        let was_default = option.is_default;
        option.is_default = false;
        if option.id == "ultracode" {
            if !has_xhigh {
                next_options.push(ProviderOptionChoice {
                    id: "xhigh".into(),
                    label: "Extra High".into(),
                    description: None,
                    is_default: was_default,
                });
            }
            continue;
        }
        if ultracode_by_default && option.id == "xhigh" {
            option.is_default = true;
        } else if was_default && !ultracode_by_default {
            option.is_default = true;
        }
        next_options.push(option);
    }
    let next_current = if current_value.as_deref() == Some("ultracode") {
        Some("xhigh".to_owned())
    } else {
        current_value
    };
    descriptors.insert(
        effort_index,
        ProviderOptionDescriptor::Select {
            id,
            label,
            description,
            options: next_options,
            current_value: next_current,
            prompt_injected_values,
            role,
        },
    );
    descriptors.insert(
        effort_index + 1,
        ProviderOptionDescriptor::Boolean {
            id: "ultracode".into(),
            label: "Ultracode".into(),
            description: None,
            current_value: ultracode_by_default.then_some(true),
        },
    );
    ModelCapabilities {
        option_descriptors: descriptors,
    }
}

/// Ported from `packages/shared/src/model.ts` (`withImplicitFastModeDefault`).
pub fn with_implicit_fast_mode_default(
    caps: &ModelCapabilities,
    selections: Option<&[ProviderOptionSelection]>,
) -> Vec<ProviderOptionSelection> {
    let base = selections.unwrap_or(&[]);
    if base.iter().any(|s| s.id == "fastMode") {
        return base.to_vec();
    }
    let has_fast_mode = caps
        .option_descriptors
        .iter()
        .any(|d| matches!(d, ProviderOptionDescriptor::Boolean { id, .. } if id == "fastMode"));
    if !has_fast_mode {
        return base.to_vec();
    }
    let mut out = base.to_vec();
    out.push(ProviderOptionSelection {
        id: "fastMode".into(),
        value: OptionValue::Bool(false),
    });
    out
}

fn resolve_descriptor_choice_value(
    options: &[ProviderOptionChoice],
    current_value: Option<&str>,
    prompt_injected_values: &[String],
    raw: Option<&str>,
) -> Option<String> {
    let trimmed = normalize_custom_model_slug(raw);
    let Some(trimmed) = trimmed else {
        return current_value
            .map(str::to_owned)
            .or_else(|| options.iter().find(|o| o.is_default).map(|o| o.id.clone()));
    };
    if options.is_empty() {
        return Some(trimmed);
    }
    if prompt_injected_values.contains(&trimmed) && options.iter().any(|o| o.id == trimmed) {
        return options.iter().find(|o| o.is_default).map(|o| o.id.clone());
    }
    if options.iter().any(|o| o.id == trimmed) {
        return Some(trimmed);
    }
    current_value
        .map(str::to_owned)
        .or_else(|| options.iter().find(|o| o.is_default).map(|o| o.id.clone()))
}

/// Ported from `packages/shared/src/model.ts` (`getProviderOptionDescriptors`).
pub fn get_provider_option_descriptors(
    caps: &ModelCapabilities,
    selections: &[ProviderOptionSelection],
) -> Vec<ProviderOptionDescriptor> {
    let effective_selections = with_legacy_ultracode_effort(&caps.option_descriptors, selections);
    caps.option_descriptors
        .iter()
        .map(|descriptor| match descriptor {
            ProviderOptionDescriptor::Boolean {
                id,
                label,
                description,
                current_value,
            } => {
                let next_val = raw_selection_value(&effective_selections, id)
                    .and_then(OptionValue::as_bool)
                    .or(*current_value);
                ProviderOptionDescriptor::Boolean {
                    id: id.clone(),
                    label: label.clone(),
                    description: description.clone(),
                    current_value: next_val,
                }
            }
            ProviderOptionDescriptor::Select {
                id,
                label,
                description,
                options,
                current_value,
                prompt_injected_values,
                role,
            } => {
                let raw = raw_selection_value(&effective_selections, id)
                    .and_then(OptionValue::as_str)
                    .or(current_value.as_deref());
                let resolved = resolve_descriptor_choice_value(
                    options,
                    current_value.as_deref(),
                    prompt_injected_values,
                    raw,
                );
                ProviderOptionDescriptor::Select {
                    id: id.clone(),
                    label: label.clone(),
                    description: description.clone(),
                    options: options.clone(),
                    current_value: resolved,
                    prompt_injected_values: prompt_injected_values.clone(),
                    role: *role,
                }
            }
        })
        .collect()
}

fn get_reported_option_value(
    id: &str,
    selection: Option<&ModelSelection>,
    reported_selection: Option<&ModelSelection>,
) -> Option<OptionValue> {
    let sel = selection?;
    let rep = reported_selection?;
    if sel.instance_id != rep.instance_id
        || sel.model != rep.model
        || sel.options.iter().any(|o| o.id == id)
    {
        return None;
    }
    raw_selection_value(&rep.options, id).cloned()
}

/// Ported from `packages/shared/src/model.ts` (`getProviderOptionCurrentValue`).
pub fn get_provider_option_current_value(
    descriptor: &ProviderOptionDescriptor,
    selection: Option<&ModelSelection>,
    reported_selection: Option<&ModelSelection>,
) -> Option<OptionValue> {
    let id = descriptor.id();
    let has_explicit = selection.is_some_and(|s| s.options.iter().any(|o| o.id == id));
    if let Some(reported) = get_reported_option_value(id, selection, reported_selection) {
        return Some(reported);
    }
    if id == "variant" && selection.is_some() && !has_explicit {
        return None;
    }
    match descriptor {
        ProviderOptionDescriptor::Boolean { current_value, .. } => {
            current_value.map(OptionValue::Bool)
        }
        ProviderOptionDescriptor::Select {
            options,
            current_value,
            ..
        } => current_value
            .clone()
            .or_else(|| options.iter().find(|o| o.is_default).map(|o| o.id.clone()))
            .map(OptionValue::Str),
    }
}

/// Ported from `packages/shared/src/model.ts` (`getProviderOptionCurrentLabel`).
pub fn get_provider_option_current_label(
    descriptor: &ProviderOptionDescriptor,
    selection: Option<&ModelSelection>,
    reported_selection: Option<&ModelSelection>,
) -> Option<String> {
    match descriptor {
        ProviderOptionDescriptor::Boolean { current_value, .. } => {
            current_value.map(|b| if b { "On" } else { "Off" }.into())
        }
        ProviderOptionDescriptor::Select { id, options, .. } => {
            let current =
                get_provider_option_current_value(descriptor, selection, reported_selection)?;
            let current_str = current.as_str()?;
            options
                .iter()
                .find(|o| o.id == current_str)
                .map(|o| o.label.clone())
                .or_else(|| {
                    let reported = get_reported_option_value(id, selection, reported_selection);
                    if reported.as_ref().and_then(OptionValue::as_str) == Some("default") {
                        Some("Default".into())
                    } else if id == "variant" {
                        Some("Unknown".into())
                    } else {
                        None
                    }
                })
        }
    }
}

#[cfg(test)]
/// Ported from `packages/shared/src/model.ts` (`buildProviderOptionSelectionsFromDescriptors`).
pub fn build_provider_option_selections_from_descriptors(
    descriptors: &[ProviderOptionDescriptor],
) -> Vec<ProviderOptionSelection> {
    let mut out = Vec::new();
    for descriptor in descriptors {
        if let Some(value) = get_provider_option_current_value(descriptor, None, None) {
            out.push(ProviderOptionSelection {
                id: descriptor.id().to_owned(),
                value,
            });
        }
    }
    out
}

#[cfg(test)]
/// Ported from `packages/shared/src/model.ts` (`buildExplicitProviderOptionSelectionsFromDescriptors`).
pub fn build_explicit_provider_option_selections_from_descriptors(
    descriptors: &[ProviderOptionDescriptor],
    selections: &[ProviderOptionSelection],
) -> Vec<ProviderOptionSelection> {
    if selections.is_empty() {
        return Vec::new();
    }
    let explicit_ids: HashSet<String> = with_legacy_ultracode_effort(descriptors, selections)
        .into_iter()
        .map(|s| s.id)
        .collect();
    build_provider_option_selections_from_descriptors(descriptors)
        .into_iter()
        .filter(|s| explicit_ids.contains(&s.id))
        .collect()
}

pub fn is_claude_ultrathink_prompt(text: &str) -> bool {
    ULTRATHINK_WORD_RE.is_match(text)
}

pub fn read_prompt_ultrathink(prompt: &str) -> PromptUltrathink {
    if !is_claude_ultrathink_prompt(prompt) {
        return PromptUltrathink::None;
    }
    let stripped = ULTRATHINK_PREFIX_RE.replace(prompt, "");
    if is_claude_ultrathink_prompt(&stripped) {
        PromptUltrathink::Body
    } else {
        PromptUltrathink::Prefix
    }
}

pub fn strip_ultrathink_prefix(prompt: &str) -> String {
    ULTRATHINK_PREFIX_RE.replace(prompt, "").into_owned()
}

/// Ported from `packages/shared/src/model.ts` (`applyClaudePromptEffortPrefix`).
pub fn apply_claude_prompt_effort_prefix(text: &str, effort: Option<&str>) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return trimmed.to_owned();
    }
    if effort != Some("ultrathink") || SLASH_COMMAND_RE.is_match(trimmed) {
        return trimmed.to_owned();
    }
    if trimmed.starts_with("Ultrathink:") {
        return trimmed.to_owned();
    }
    format!("Ultrathink:\n{trimmed}")
}

/// Ported from `packages/shared/src/model.ts` (`resolvePromptInjectedEffort`).
pub fn resolve_prompt_injected_effort(
    caps: &ModelCapabilities,
    raw_effort: Option<&str>,
) -> Option<String> {
    let trimmed = normalize_custom_model_slug(raw_effort)?;
    for descriptor in &caps.option_descriptors {
        if let ProviderOptionDescriptor::Select {
            prompt_injected_values,
            ..
        } = descriptor
            && prompt_injected_values.contains(&trimmed)
        {
            return Some(trimmed);
        }
    }
    None
}

/// Ported from `packages/client-runtime/src/modelTraits.ts` (`selectableProviderOptionChoices`).
pub fn selectable_provider_option_choices(
    descriptor: &ProviderOptionDescriptor,
) -> Vec<ProviderOptionChoice> {
    match descriptor {
        ProviderOptionDescriptor::Select {
            options,
            prompt_injected_values,
            ..
        } => options
            .iter()
            .filter(|o| !prompt_injected_values.contains(&o.id))
            .cloned()
            .collect(),
        ProviderOptionDescriptor::Boolean { .. } => Vec::new(),
    }
}

/// Ported from `packages/client-runtime/src/modelTraits.ts` (`isReasoningControl`).
pub fn is_reasoning_control(descriptor: &ProviderOptionDescriptor) -> bool {
    match descriptor {
        ProviderOptionDescriptor::Select { id, role, .. } => {
            *role == Some(ProviderOptionRole::Reasoning)
                || REASONING_EFFORT_OPTION_IDS.contains(&id.as_str())
                || id == "thinking"
        }
        ProviderOptionDescriptor::Boolean { id, .. } => id == "thinking",
    }
}

/// Ported from `apps/web/src/components/chat/ComposerModelEffort.tsx` (`splitSpeed`, `columnControls`).
pub fn split_column_and_footer_controls(
    descriptors: &[ProviderOptionDescriptor],
) -> ColumnControls {
    let mut speed_select = None;
    let mut fast_toggle = None;
    let mut ultracode_toggle = None;
    let mut column_all = Vec::new();
    for descriptor in descriptors {
        match descriptor {
            ProviderOptionDescriptor::Select { id, .. } if id == "serviceTier" => {
                if speed_select.is_none() {
                    speed_select = Some(descriptor.clone());
                    continue;
                }
            }
            ProviderOptionDescriptor::Boolean { id, .. } if id == "fastMode" => {
                if fast_toggle.is_none() {
                    fast_toggle = Some(descriptor.clone());
                    continue;
                }
            }
            ProviderOptionDescriptor::Boolean { id, .. } if id == "ultracode" => {
                if ultracode_toggle.is_none() {
                    ultracode_toggle = Some(descriptor.clone());
                    continue;
                }
            }
            _ => {}
        }
        column_all.push(descriptor.clone());
    }
    let effort_index = column_all.iter().position(is_reasoning_control);
    let effort = effort_index.map(|i| column_all.remove(i));
    ColumnControls {
        effort,
        others: column_all,
        speed_select,
        fast_toggle,
        ultracode_toggle,
    }
}

/// Ported from `packages/client-runtime/src/modelTraits.ts` (`buildTraitsTriggerDisplay`).
pub fn build_traits_trigger_display(
    descriptors: &[ProviderOptionDescriptor],
    ultrathink_prompt_controlled: bool,
    selection: Option<&ModelSelection>,
    reported_selection: Option<&ModelSelection>,
) -> TraitsTriggerDisplay {
    let primary_select_id = descriptors
        .iter()
        .find(|d| matches!(d, ProviderOptionDescriptor::Select { .. }))
        .map(|d| d.id().to_owned());
    let mut fast_mode_fallback_label: Option<String> = None;
    let mut speed_label: Option<String> = None;
    let mut reasoning_label_index: Option<usize> = None;
    let mut ultracode = false;
    let mut context_window: Option<String> = None;
    let mut effort_label: Option<String> = None;
    let mut labels: Vec<String> = Vec::new();

    for descriptor in descriptors {
        match descriptor {
            ProviderOptionDescriptor::Boolean {
                id, current_value, ..
            } if id == "fastMode" => {
                if *current_value == Some(true) {
                    speed_label = Some("Fast".into());
                    fast_mode_fallback_label = Some("Fast".into());
                } else {
                    fast_mode_fallback_label = Some("Normal".into());
                }
                continue;
            }
            ProviderOptionDescriptor::Boolean {
                id,
                label,
                current_value,
                ..
            } if id == "ultracode" => {
                if *current_value == Some(true) {
                    ultracode = true;
                    labels.push(label.clone());
                }
                continue;
            }
            ProviderOptionDescriptor::Select { id, options, .. } if id == "serviceTier" => {
                let current_val =
                    get_provider_option_current_value(descriptor, selection, reported_selection);
                let current_id = current_val.as_ref().and_then(OptionValue::as_str);
                let fast_tier = options.iter().find(|o| o.label == "Fast");
                let ultrafast_tier = options.iter().find(|o| o.label == "Ultrafast");
                if ((fast_tier.is_some() || ultrafast_tier.is_some())
                    && current_id == Some("default"))
                    || fast_tier.is_some_and(|t| Some(t.id.as_str()) == current_id)
                    || ultrafast_tier.is_some_and(|t| Some(t.id.as_str()) == current_id)
                {
                    speed_label =
                        if ultrafast_tier.is_some_and(|t| Some(t.id.as_str()) == current_id) {
                            Some("Ultrafast".into())
                        } else if fast_tier.is_some_and(|t| Some(t.id.as_str()) == current_id) {
                            Some("Fast".into())
                        } else {
                            None
                        };
                    fast_mode_fallback_label = Some(
                        options
                            .iter()
                            .find(|o| Some(o.id.as_str()) == current_id)
                            .map(|o| o.label.clone())
                            .unwrap_or_else(|| "Normal".into()),
                    );
                    continue;
                }
            }
            _ => {}
        }

        let item_label = if ultrathink_prompt_controlled
            && primary_select_id.as_deref() == Some(descriptor.id())
        {
            Some("Ultrathink".into())
        } else {
            match descriptor {
                ProviderOptionDescriptor::Boolean {
                    label,
                    current_value,
                    ..
                } => Some(format!(
                    "{label} {}",
                    if *current_value == Some(true) {
                        "On"
                    } else {
                        "Off"
                    }
                )),
                ProviderOptionDescriptor::Select { id, label, .. } => {
                    let lbl = get_provider_option_current_label(
                        descriptor,
                        selection,
                        reported_selection,
                    )
                    .or_else(|| (id == "thinking").then(|| label.clone()));
                    if id == "contextWindow" {
                        context_window = lbl.clone();
                    }
                    lbl
                }
            }
        };

        if let Some(lbl) = item_label
            && !lbl.is_empty()
        {
            if reasoning_label_index.is_none() && is_reasoning_control(descriptor) {
                reasoning_label_index = Some(labels.len());
                effort_label = Some(lbl.clone());
            }
            labels.push(lbl);
        }
    }

    if labels.is_empty()
        && let Some(fallback) = fast_mode_fallback_label
    {
        return TraitsTriggerDisplay {
            label: fallback,
            speed_label,
            ultracode,
            context_window,
            effort_label,
        };
    }

    if let Some(speed) = &speed_label {
        if let Some(idx) = reasoning_label_index {
            labels[idx] = format!("{} {speed}", labels[idx]);
        } else {
            labels.push(speed.clone());
        }
    }

    TraitsTriggerDisplay {
        label: labels.join(" · "),
        speed_label,
        ultracode,
        context_window,
        effort_label,
    }
}

pub fn pi_thinking_label(level: &str) -> &str {
    match level {
        "off" => "Off",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra High",
        "max" => "Max",
        other => other,
    }
}

fn clamp_pi_thinking_level<'a>(input: Option<&str>, available: &[&'a str]) -> Option<&'a str> {
    let input = input?;
    let req_idx = PI_THINKING_LEVELS.iter().position(|&l| l == input)?;
    if let Some(&exact) = available.iter().find(|&&l| l == input) {
        return Some(exact);
    }
    for &candidate in &PI_THINKING_LEVELS[req_idx + 1..] {
        if let Some(&higher) = available.iter().find(|&&l| l == candidate) {
            return Some(higher);
        }
    }
    for &candidate in PI_THINKING_LEVELS[..req_idx].iter().rev() {
        if let Some(&lower) = available.iter().find(|&&l| l == candidate) {
            return Some(lower);
        }
    }
    available.first().copied()
}

/// Ported from `packages/provider-pi/src/server/modelCapabilities.ts` (`piModelCapabilities`).
pub fn pi_model_capabilities(
    model: &Value,
    default_thinking_level: Option<&str>,
) -> ModelCapabilities {
    let mut option_descriptors = Vec::new();
    if model["reasoning"] == true {
        let level_map = model["thinkingLevelMap"].as_object();
        let levels: Vec<&str> = PI_THINKING_LEVELS
            .iter()
            .copied()
            .filter(|&level| {
                let mapped = level_map.and_then(|m| m.get(level));
                if mapped.is_some_and(Value::is_null) {
                    return false;
                }
                if level == "xhigh" || level == "max" {
                    return mapped.is_some();
                }
                true
            })
            .collect();
        if !levels.is_empty() {
            let default_level = clamp_pi_thinking_level(default_thinking_level, &levels);
            option_descriptors.push(ProviderOptionDescriptor::Select {
                id: "thinking".into(),
                label: "Thinking".into(),
                description: None,
                options: levels
                    .into_iter()
                    .map(|lvl| ProviderOptionChoice {
                        id: lvl.into(),
                        label: pi_thinking_label(lvl).into(),
                        description: None,
                        is_default: Some(lvl) == default_level,
                    })
                    .collect(),
                current_value: None,
                prompt_injected_values: vec![],
                role: Some(ProviderOptionRole::Reasoning),
            });
        }
    }

    if let Some(tiers) = model["serviceTiers"].as_array() {
        let mut seen = HashSet::from([PI_STANDARD_SERVICE_TIER.to_owned()]);
        let mut options = Vec::new();
        for tier in tiers {
            let Some(id) = tier["id"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
                continue;
            };
            if !seen.insert(id.to_owned()) {
                continue;
            }
            let name = tier["name"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id);
            let description = tier["description"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            options.push(ProviderOptionChoice {
                id: id.to_owned(),
                label: name.to_owned(),
                description,
                is_default: false,
            });
        }
        if !options.is_empty() {
            let mut all_options = vec![ProviderOptionChoice {
                id: PI_STANDARD_SERVICE_TIER.into(),
                label: "Standard".into(),
                description: None,
                is_default: true,
            }];
            all_options.extend(options);
            option_descriptors.push(ProviderOptionDescriptor::Select {
                id: "serviceTier".into(),
                label: "Service Tier".into(),
                description: None,
                options: all_options,
                current_value: Some(PI_STANDARD_SERVICE_TIER.into()),
                prompt_injected_values: vec![],
                role: None,
            });
        }
    }

    ModelCapabilities { option_descriptors }
}

fn codex_reasoning_effort_label(effort: &str) -> &str {
    match effort {
        "none" => "None",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra High",
        "max" => "Max",
        "ultra" => "Ultra",
        other => other,
    }
}

fn codex_model_family(slug: &str) -> &str {
    slug.strip_prefix("openai.")
        .filter(|s| s.starts_with("gpt-"))
        .unwrap_or(slug)
}

pub fn codex_default_reasoning_effort_override(model: &str) -> Option<&'static str> {
    (codex_model_family(model) == "gpt-6-astra").then_some("medium")
}

/// Ported from `apps/server/src/provider/CodexProvider.ts` (`mapCodexModelCapabilities`).
pub fn map_codex_model_capabilities(
    model: &Value,
    configured_service_tier: Option<&str>,
) -> ModelCapabilities {
    let model_id = model["model"]
        .as_str()
        .or_else(|| model["id"].as_str())
        .unwrap_or("");
    let default_effort = codex_default_reasoning_effort_override(model_id)
        .or_else(|| model["defaultReasoningEffort"].as_str());
    let reasoning_options: Vec<ProviderOptionChoice> = model["supportedReasoningEfforts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let effort = entry["reasoningEffort"].as_str()?;
            Some(ProviderOptionChoice {
                id: effort.into(),
                label: codex_reasoning_effort_label(effort).into(),
                description: None,
                is_default: default_effort == Some(effort),
            })
        })
        .collect();

    let mut option_descriptors = Vec::new();
    if !reasoning_options.is_empty() {
        let default_reasoning = reasoning_options
            .iter()
            .find(|o| o.is_default)
            .map(|o| o.id.clone());
        option_descriptors.push(ProviderOptionDescriptor::Select {
            id: "reasoningEffort".into(),
            label: "Reasoning".into(),
            description: None,
            options: reasoning_options,
            current_value: default_reasoning,
            prompt_injected_values: vec![],
            role: Some(ProviderOptionRole::Reasoning),
        });
    }

    let raw_tiers: Vec<(String, String, String)> = model["serviceTiers"]
        .as_array()
        .filter(|a| !a.is_empty())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let id = t["id"].as_str()?.to_owned();
                    let name = t["name"].as_str().unwrap_or(&id).to_owned();
                    let desc = t["description"].as_str().unwrap_or("").to_owned();
                    Some((id, name, desc))
                })
                .collect()
        })
        .unwrap_or_else(|| {
            model["additionalSpeedTiers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str())
                .map(|id| {
                    (
                        id.to_owned(),
                        if id == "fast" {
                            "Fast".into()
                        } else {
                            id.to_owned()
                        },
                        String::new(),
                    )
                })
                .collect()
        });

    if !raw_tiers.is_empty() {
        let catalog_default = model["defaultServiceTier"]
            .as_str()
            .filter(|id| raw_tiers.iter().any(|(tid, _, _)| tid == id));
        let mut default_tier = catalog_default
            .unwrap_or(DEFAULT_SERVICE_TIER_ID)
            .to_owned();

        let mut options = vec![ProviderOptionChoice {
            id: DEFAULT_SERVICE_TIER_ID.into(),
            label: "Standard".into(),
            description: None,
            is_default: default_tier == DEFAULT_SERVICE_TIER_ID,
        }];
        for (id, name, raw_desc) in &raw_tiers {
            if id == DEFAULT_SERVICE_TIER_ID {
                continue;
            }
            let desc = if id == "ultrafast" {
                "Even faster, more expensive".to_owned()
            } else {
                raw_desc.clone()
            };
            options.push(ProviderOptionChoice {
                id: id.clone(),
                label: name.clone(),
                description: (!desc.is_empty()).then_some(desc),
                is_default: default_tier == *id,
            });
        }
        if let Some(configured) = configured_service_tier
            && let Some(matched) = options
                .iter()
                .find(|o| o.id == configured || (configured == "fast" && o.id == "priority"))
                .map(|o| o.id.clone())
        {
            default_tier = matched.clone();
            for opt in &mut options {
                opt.is_default = opt.id == matched;
            }
        }
        option_descriptors.push(ProviderOptionDescriptor::Select {
            id: "serviceTier".into(),
            label: "Service Tier".into(),
            description: None,
            options,
            current_value: Some(default_tier),
            prompt_injected_values: vec![],
            role: None,
        });
    }

    ModelCapabilities { option_descriptors }
}

#[derive(Clone, Debug)]
pub struct ClaudeManifestEntry {
    pub slug: String,
    pub short_name: Option<String>,
    pub aliases: Vec<String>,
    pub is_legacy: bool,
    pub badge: Option<String>,
    pub capabilities: ModelCapabilities,
}

static CLAUDE_MANIFEST: LazyLock<Vec<ClaudeManifestEntry>> = LazyLock::new(|| {
    let root: Value = serde_json::from_str(MODEL_MANIFEST_JSON).unwrap_or(Value::Null);
    let claude = &root["providers"]["claudeAgent"];
    let profiles = claude["profiles"].as_object();
    let mut entries = Vec::new();
    for model in claude["models"].as_array().into_iter().flatten() {
        let Some(slug) = model["slug"].as_str().map(str::to_owned) else {
            continue;
        };
        let short_name = model["shortName"].as_str().map(str::to_owned);
        let aliases = model["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let is_legacy = model["status"].as_str() == Some("legacy");
        let profile = model["profile"]
            .as_str()
            .and_then(|p| profiles.and_then(|map| map.get(p)));

        let caps: ModelCapabilities = profile
            .and_then(|p| serde_json::from_value(p["capabilities"].clone()).ok())
            .unwrap_or_default();
        let capabilities = with_ultracode_toggle(caps);

        entries.push(ClaudeManifestEntry {
            slug,
            short_name,
            aliases,
            is_legacy,
            badge: None,
            capabilities,
        });
    }
    entries
});

pub fn claude_manifest_model_entry(slug_or_alias: &str) -> Option<ClaudeManifestEntry> {
    let trimmed = slug_or_alias.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();
    CLAUDE_MANIFEST
        .iter()
        .find(|e| e.slug == trimmed)
        .or_else(|| {
            CLAUDE_MANIFEST
                .iter()
                .find(|e| e.aliases.iter().any(|a| a.to_lowercase() == lower))
        })
        .cloned()
}

fn bounded_acp_text(value: Option<&str>, max_len: usize) -> String {
    value.unwrap_or("").trim().chars().take(max_len).collect()
}

fn bounded_acp_opaque_value(value: &str, max_len: usize) -> Option<String> {
    if !value.is_empty() && value == value.trim() && value.chars().count() <= max_len {
        Some(value.to_owned())
    } else {
        None
    }
}

fn acp_choice_id(value: &str) -> Option<String> {
    if value.is_empty() {
        Some(ACP_EMPTY_OPTION_VALUE.into())
    } else {
        bounded_acp_opaque_value(value, MAX_ACP_TEXT_LENGTH)
    }
}

fn acp_select_choices(candidates: Vec<(&str, &str, Option<&str>)>) -> Vec<ProviderOptionChoice> {
    let mut seen = HashSet::new();
    let mut choices = Vec::new();
    for (value, name, desc) in candidates {
        let Some(id) = acp_choice_id(value) else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let description = bounded_acp_text(desc, MAX_ACP_DESCRIPTION_LENGTH);
        let label = bounded_acp_text(Some(name), MAX_ACP_TEXT_LENGTH);
        choices.push(ProviderOptionChoice {
            id: id.clone(),
            label: if label.is_empty() { id } else { label },
            description: (!description.is_empty()).then_some(description),
            is_default: false,
        });
        if choices.len() == MAX_ACP_OPTION_CHOICES {
            break;
        }
    }
    choices
}

/// Ported from `packages/provider-acp/src/server/sessionConfig.ts` (`acpProviderOptionDescriptors`).
pub fn acp_provider_option_descriptors(
    config_options: Option<&[Value]>,
    mode_state: Option<&Value>,
) -> Vec<ProviderOptionDescriptor> {
    let mut descriptors = Vec::new();
    let mut seen = HashSet::new();
    let mut mirrored_mode_choice_sets: Vec<HashSet<String>> = Vec::new();
    let mut has_mode_category = false;

    for option in config_options.unwrap_or(&[]) {
        let category = option["category"].as_str().unwrap_or("");
        if category == "model" || category == "collaboration_mode" {
            continue;
        }
        let Some(raw_id) = option["id"].as_str() else {
            continue;
        };
        let Some(id) = bounded_acp_opaque_value(raw_id, MAX_ACP_TEXT_LENGTH) else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let raw_name = option["name"].as_str();
        let label_text = bounded_acp_text(raw_name, MAX_ACP_TEXT_LENGTH);
        let label = if label_text.is_empty() {
            id.clone()
        } else {
            label_text
        };
        let desc_text =
            bounded_acp_text(option["description"].as_str(), MAX_ACP_DESCRIPTION_LENGTH);
        let description = (!desc_text.is_empty()).then_some(desc_text);

        if option["type"].as_str() == Some("boolean") {
            if category == "mode" {
                has_mode_category = true;
            }
            descriptors.push(ProviderOptionDescriptor::Boolean {
                id,
                label,
                description,
                current_value: option["currentValue"].as_bool(),
            });
            if descriptors.len() == MAX_ACP_OPTION_DESCRIPTORS {
                return descriptors;
            }
            continue;
        }

        let mut flat = Vec::new();
        for candidate in option["options"].as_array().into_iter().flatten() {
            if let Some(group_options) = candidate["options"].as_array() {
                for inner in group_options {
                    if let (Some(val), Some(name)) =
                        (inner["value"].as_str(), inner["name"].as_str())
                    {
                        flat.push((val, name, inner["description"].as_str()));
                    }
                }
            } else if let (Some(val), Some(name)) =
                (candidate["value"].as_str(), candidate["name"].as_str())
            {
                flat.push((val, name, candidate["description"].as_str()));
            }
        }
        let choices = acp_select_choices(flat);
        if choices.is_empty() {
            seen.remove(&id);
            continue;
        }
        if category == "mode" {
            has_mode_category = true;
        }
        if category == "thought_level" {
            mirrored_mode_choice_sets.push(choices.iter().map(|c| c.id.clone()).collect());
        }
        let current_value = option["currentValue"]
            .as_str()
            .and_then(acp_choice_id)
            .filter(|cv| choices.iter().any(|c| &c.id == cv));
        descriptors.push(ProviderOptionDescriptor::Select {
            id,
            label,
            description,
            options: choices,
            current_value,
            prompt_injected_values: vec![],
            role: (category == "thought_level").then_some(ProviderOptionRole::Reasoning),
        });
        if descriptors.len() == MAX_ACP_OPTION_DESCRIPTORS {
            return descriptors;
        }
    }

    if let Some(modes) = mode_state
        && !has_mode_category
        && let Some(available) = modes["availableModes"].as_array()
    {
        let candidates: Vec<(&str, &str, Option<&str>)> = available
            .iter()
            .filter_map(|m| {
                Some((
                    m["id"].as_str()?,
                    m["name"].as_str().unwrap_or(m["id"].as_str()?),
                    m["description"].as_str(),
                ))
            })
            .collect();
        let choices = acp_select_choices(candidates);
        let current_value = modes["currentModeId"]
            .as_str()
            .and_then(acp_choice_id)
            .filter(|cv| choices.iter().any(|c| &c.id == cv));
        let duplicates_known = mirrored_mode_choice_sets
            .iter()
            .any(|set| set.len() == choices.len() && choices.iter().all(|c| set.contains(&c.id)));
        if choices.len() > 1
            && !duplicates_known
            && !seen.contains(ACP_SESSION_MODE_OPTION_ID)
            && descriptors.len() < MAX_ACP_OPTION_DESCRIPTORS
        {
            descriptors.push(ProviderOptionDescriptor::Select {
                id: ACP_SESSION_MODE_OPTION_ID.into(),
                label: "Mode".into(),
                description: Some("Session mode advertised by the ACP agent.".into()),
                options: choices,
                current_value,
                prompt_injected_values: vec![],
                role: None,
            });
        }
    }

    descriptors
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn descriptor_helpers_build_explicit_selections_and_handle_legacy_ultracode() {
        let caps = ModelCapabilities {
            option_descriptors: vec![
                ProviderOptionDescriptor::Select {
                    id: "effort".into(),
                    label: "Reasoning".into(),
                    description: None,
                    options: vec![
                        ProviderOptionChoice {
                            id: "high".into(),
                            label: "High".into(),
                            description: None,
                            is_default: true,
                        },
                        ProviderOptionChoice {
                            id: "xhigh".into(),
                            label: "Extra High".into(),
                            description: None,
                            is_default: false,
                        },
                    ],
                    current_value: None,
                    prompt_injected_values: vec![],
                    role: None,
                },
                ProviderOptionDescriptor::Boolean {
                    id: "ultracode".into(),
                    label: "Ultracode".into(),
                    description: None,
                    current_value: None,
                },
            ],
        };
        let legacy = vec![ProviderOptionSelection {
            id: "effort".into(),
            value: OptionValue::Str("ultracode".into()),
        }];
        let descriptors = get_provider_option_descriptors(&caps, &legacy);
        assert_eq!(
            build_explicit_provider_option_selections_from_descriptors(&descriptors, &legacy),
            vec![
                ProviderOptionSelection {
                    id: "effort".into(),
                    value: OptionValue::Str("xhigh".into()),
                },
                ProviderOptionSelection {
                    id: "ultracode".into(),
                    value: OptionValue::Bool(true),
                },
            ]
        );
    }

    #[test]
    fn claude_ultrathink_prefix_and_body_detection_match_ksg() {
        assert_eq!(
            apply_claude_prompt_effort_prefix("/compact", Some("ultrathink")),
            "/compact"
        );
        assert_eq!(
            apply_claude_prompt_effort_prefix(" /review src/model.ts ", Some("ultrathink")),
            "/review src/model.ts"
        );
        assert_eq!(
            apply_claude_prompt_effort_prefix("Investigate this failure", Some("ultrathink")),
            "Ultrathink:\nInvestigate this failure"
        );
        assert_eq!(read_prompt_ultrathink("plain text"), PromptUltrathink::None);
        assert_eq!(
            read_prompt_ultrathink("Ultrathink:\nSolve this"),
            PromptUltrathink::Prefix
        );
        assert_eq!(
            read_prompt_ultrathink("Please ultrathink about this"),
            PromptUltrathink::Body
        );
    }

    #[test]
    fn pi_capabilities_advertise_thinking_and_service_tiers() {
        let model = json!({
            "provider": "cpa",
            "id": "gpt-6.1-sol",
            "reasoning": true,
            "serviceTiers": [
                {"id": "priority", "name": "Fast", "description": "2x speed"},
                {"id": "ultrafast", "name": "Ultrafast"}
            ]
        });
        let caps = pi_model_capabilities(&model, Some("high"));
        assert_eq!(
            caps.option_descriptors
                .iter()
                .map(|d| d.id())
                .collect::<Vec<_>>(),
            ["thinking", "serviceTier"]
        );
        assert_eq!(
            caps.option_descriptors[1],
            ProviderOptionDescriptor::Select {
                id: "serviceTier".into(),
                label: "Service Tier".into(),
                description: None,
                options: vec![
                    ProviderOptionChoice {
                        id: "default".into(),
                        label: "Standard".into(),
                        description: None,
                        is_default: true,
                    },
                    ProviderOptionChoice {
                        id: "priority".into(),
                        label: "Fast".into(),
                        description: Some("2x speed".into()),
                        is_default: false,
                    },
                    ProviderOptionChoice {
                        id: "ultrafast".into(),
                        label: "Ultrafast".into(),
                        description: None,
                        is_default: false,
                    },
                ],
                current_value: Some("default".into()),
                prompt_injected_values: vec![],
                role: None,
            }
        );
    }

    #[test]
    fn codex_capabilities_map_reasoning_and_service_tiers_matching_ksg() {
        let model = json!({
            "model": "gpt-test",
            "defaultReasoningEffort": "high",
            "defaultServiceTier": "flex",
            "supportedReasoningEfforts": [
                {"reasoningEffort": "low"},
                {"reasoningEffort": "high"}
            ],
            "serviceTiers": [
                {"id": "priority", "name": "Fast", "description": "Lower latency"},
                {"id": "flex", "name": "Flex", "description": "Async routing"}
            ]
        });
        let caps = map_codex_model_capabilities(&model, None);
        assert_eq!(
            caps.option_descriptors
                .iter()
                .map(|d| d.id())
                .collect::<Vec<_>>(),
            ["reasoningEffort", "serviceTier"]
        );
        let astra = json!({
            "model": "gpt-6-astra",
            "defaultReasoningEffort": "high",
            "supportedReasoningEfforts": [
                {"reasoningEffort": "medium"},
                {"reasoningEffort": "high"}
            ]
        });
        let astra_caps = map_codex_model_capabilities(&astra, None);
        assert_eq!(
            get_provider_option_current_value(&astra_caps.option_descriptors[0], None, None),
            Some(OptionValue::Str("medium".into()))
        );
    }

    #[test]
    fn claude_manifest_populates_ultracode_fast_mode_and_ultrathink() {
        let opus = claude_manifest_model_entry("claude-opus-5-5").unwrap();
        let ids: Vec<&str> = opus
            .capabilities
            .option_descriptors
            .iter()
            .map(|d| d.id())
            .collect();
        assert_eq!(ids, ["effort", "ultracode", "fastMode"]);
        let selectable =
            selectable_provider_option_choices(&opus.capabilities.option_descriptors[0]);
        assert!(
            !selectable
                .iter()
                .any(|c| c.id == "ultrathink" || c.id == "ultracode")
        );
    }

    #[test]
    fn acp_descriptors_exclude_model_map_thought_level_and_synthesize_mode() {
        let config_options = vec![
            json!({"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "m1", "options": [{"value": "m1", "name": "M1"}]}),
            json!({"id": "collaboration_mode", "name": "Collab", "category": "collaboration_mode", "type": "select", "currentValue": "default", "options": [{"value": "default", "name": "Default"}]}),
            json!({"id": "thought_level", "name": "Thinking", "category": "thought_level", "type": "select", "currentValue": "medium", "options": [{"value": "low", "name": "Low"}, {"value": "medium", "name": "Medium"}]}),
            json!({"id": "auto_approve", "name": "Auto-approve", "type": "boolean", "currentValue": false}),
        ];
        let mode_state = json!({
            "currentModeId": "safe",
            "availableModes": [
                {"id": "safe", "name": "Safe"},
                {"id": "yolo", "name": "YOLO", "description": "No approvals"}
            ]
        });
        let descriptors = acp_provider_option_descriptors(Some(&config_options), Some(&mode_state));
        assert_eq!(
            descriptors.iter().map(|d| d.id()).collect::<Vec<_>>(),
            ["thought_level", "auto_approve", ACP_SESSION_MODE_OPTION_ID]
        );
        assert!(is_reasoning_control(&descriptors[0]));
    }
}
