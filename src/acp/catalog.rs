//! Port of KSG's `normalizeAcpRegistryLiveConfiguration` and `modelsFromDiscovery`.
use crate::descriptors::{
    ProviderOptionDescriptor, ProviderOptionRole, acp_provider_option_descriptors,
    is_acp_thought_level_option, model_reasoning_levels,
};
use serde_json::{Value, json};
use std::collections::HashSet;

pub(super) fn model_select(options: &[Value]) -> Option<&Value> {
    options.iter().find(|option| {
        option["category"] == "model"
            && option["type"] == "select"
            && option["id"]
                .as_str()
                .is_some_and(|id| !id.trim().is_empty())
    })
}

pub(super) fn choices(option: &Value) -> impl Iterator<Item = &Value> {
    option["options"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|choice| {
            let group = choice["options"].as_array().map(Vec::as_slice);
            if choice.get("value").is_some() {
                std::slice::from_ref(choice).iter()
            } else {
                group.unwrap_or_default().iter()
            }
        })
}

pub(super) fn advertises_models(options: &[Value], legacy: &Value) -> bool {
    model_select(options).is_some_and(|select| choices(select).nth(1).is_some())
        || legacy["availableModels"]
            .as_array()
            .is_some_and(|models| models.len() > 1)
}

fn opaque_id(value: &Value) -> Option<&str> {
    let id = value.as_str()?;
    (!id.is_empty() && id.trim() == id && id.encode_utf16().count() <= 128).then_some(id)
}

fn name(value: &Value, fallback: &str) -> String {
    let mut remaining = 160usize;
    let label: String = value
        .as_str()
        .unwrap_or("")
        .trim()
        .chars()
        .take_while(|ch| {
            if let Some(next) = remaining.checked_sub(ch.len_utf16()) {
                remaining = next;
                true
            } else {
                false
            }
        })
        .collect();
    if label.is_empty() {
        fallback.into()
    } else {
        label
    }
}

fn advertised_reasoning(
    option: Option<&Value>,
    meta: &Value,
) -> Option<Option<ProviderOptionDescriptor>> {
    let mut option = option.filter(|option| option["type"] == "select")?.clone();
    if meta["supportsReasoningEffort"] == false {
        return Some(None);
    }
    option["options"] = Value::Array(
        meta["reasoningEfforts"]
            .as_array()?
            .iter()
            .filter_map(|effort| {
                let value = effort["value"].as_str()?;
                let label = effort["label"].as_str().unwrap_or(value);
                Some(json!({"value":value,"name":label}))
            })
            .collect(),
    );
    option["currentValue"] = meta["reasoningEffort"].clone();
    Some(
        acp_provider_option_descriptors(Some(&[option]), None)
            .into_iter()
            .next(),
    )
}

pub(super) fn model_rows(
    provider: &str,
    options: &[Value],
    legacy: &Value,
    descriptors: &[ProviderOptionDescriptor],
    previous: &[Value],
    input: &[&str],
) -> (Vec<Value>, Option<String>, bool) {
    let explicit = model_select(options);
    let legacy_rows = legacy["availableModels"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let current = if let Some(option) = explicit {
        opaque_id(&option["currentValue"])
    } else {
        opaque_id(&legacy["currentModelId"])
    };
    let candidates: Vec<_> = match explicit {
        Some(option) => choices(option)
            .filter_map(|m| Some((opaque_id(&m["value"])?, &m["name"])))
            .collect(),
        None => legacy_rows
            .iter()
            .filter_map(|m| Some((opaque_id(&m["modelId"])?, &m["name"])))
            .collect(),
    };
    let mut seen = HashSet::new();
    let mut models: Vec<_> = candidates
        .iter()
        .filter(|(id, _)| seen.insert(*id))
        .take(256)
        .copied()
        .collect();
    if models.len() == 256
        && let Some(current) = current
        && !seen.contains(current)
        && let Some(candidate) = candidates.iter().find(|(id, _)| *id == current)
    {
        models[255] = *candidate;
    }
    let use_previous = !previous.is_empty()
        && (models.is_empty() || (models.len() == 1 && explicit.is_none() && previous.len() > 1));
    let advertised_live = !use_previous && !candidates.is_empty();
    let current = if use_previous {
        models = previous
            .iter()
            .filter_map(|m| Some((m["id"].as_str()?, &m["name"])))
            .collect();
        current
            .filter(|id| models.iter().any(|(model, _)| model == id))
            .or_else(|| {
                previous
                    .iter()
                    .find(|m| m["isDefault"] == true)
                    .and_then(|m| m["id"].as_str())
            })
    } else if models.is_empty() {
        models.push(("default", &Value::Null));
        Some("default")
    } else {
        current.filter(|id| models.iter().any(|(model, _)| model == id))
    };
    if use_previous && descriptors.is_empty() {
        let mut rows = previous.to_vec();
        for row in &mut rows {
            row["isDefault"] = json!(row["id"].as_str() == current);
        }
        return (rows, current.map(str::to_owned), false);
    }

    let model_specific: HashSet<_> = options
        .iter()
        .filter(|option| {
            is_acp_thought_level_option(option) || option["category"] == "model_config"
        })
        .filter_map(|option| option["id"].as_str())
        .collect();
    let shared: Vec<_> = descriptors
        .iter()
        .filter(|d| !model_specific.contains(d.id()))
        .cloned()
        .collect();
    let effort = options
        .iter()
        .find(|option| option["category"] == "thought_level")
        .or_else(|| {
            options
                .iter()
                .find(|option| is_acp_thought_level_option(option))
        });
    let rows = models
        .into_iter()
        .map(|(id, label)| {
            let is_default = Some(id) == current;
            let descriptors = if is_default {
                descriptors.to_vec()
            } else {
                let advertised = legacy_rows
                    .iter()
                    .find(|model| model["modelId"] == id)
                    .and_then(|model| advertised_reasoning(effort, &model["_meta"]));
                let replaces_reasoning = advertised.is_some();
                let previous: Vec<ProviderOptionDescriptor> = previous
                    .iter()
                    .find(|m| m["id"] == id)
                    .and_then(|m| {
                        serde_json::from_value(m["capabilities"]["optionDescriptors"].clone()).ok()
                    })
                    .unwrap_or_default();
                shared
                    .iter()
                    .cloned()
                    .chain(previous.into_iter().filter(|d| {
                        !shared.iter().any(|s| s.id() == d.id())
                            && !(replaces_reasoning
                                && matches!(
                                    d,
                                    ProviderOptionDescriptor::Select {
                                        role: Some(ProviderOptionRole::Reasoning),
                                        ..
                                    }
                                ))
                    }))
                    .chain(advertised.flatten())
                    .collect()
            };
            json!({
                "id":id, "name":name(label, if id == "default" { "Default" } else { id }),
                "provider":provider, "isDefault":is_default, "input":input,
                "thinkingLevels":model_reasoning_levels(&descriptors),
                "capabilities":{"optionDescriptors":descriptors},
            })
        })
        .collect();
    (rows, current.map(str::to_owned), advertised_live)
}
