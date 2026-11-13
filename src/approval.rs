//! KSG ProviderApprovalDecision and ComposerPendingApprovalActions.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionItem {
    pub decision: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

pub fn valid_decision(decision: &str) -> bool {
    matches!(
        decision,
        "accept" | "acceptForSession" | "acceptAlways" | "decline" | "cancel"
    )
}

pub fn defaults() -> Vec<OptionItem> {
    [
        ("cancel", "Cancel"),
        ("decline", "Decline"),
        ("acceptForSession", "Always allow this session"),
        ("accept", "Approve"),
    ]
    .into_iter()
    .map(|(decision, label)| OptionItem {
        decision: decision.into(),
        label: label.into(),
        warning: None,
    })
    .collect()
}

pub fn options(request: &Value) -> Vec<OptionItem> {
    request
        .get("approvalOptions")
        .map(|value| {
            serde_json::from_value::<Vec<OptionItem>>(value.clone())
                .unwrap_or_default()
                .into_iter()
                .filter(|option| {
                    valid_decision(&option.decision) && !option.label.trim().is_empty()
                })
                .collect()
        })
        .unwrap_or_else(defaults)
}

pub fn can_respond(request: &Value) -> bool {
    request["responseCapability"]["type"] != "not_resumable"
        && request["responseCapability"] != "not_resumable"
        && request["canRespond"] != false
}

/// Old clients still send the Pi boolean envelope. New clients send a decision too.
pub fn decision(response: &Value) -> Result<&str, String> {
    if let Some(decision) = response["decision"].as_str() {
        return valid_decision(decision)
            .then_some(decision)
            .ok_or_else(|| "Update required. Unknown approval decision.".into());
    }
    if response["cancelled"] == true {
        Ok("cancel")
    } else if response["confirmed"] == true {
        Ok(if response["allowForSession"] == true {
            "acceptForSession"
        } else {
            "accept"
        })
    } else if response["confirmed"] == false {
        Ok("decline")
    } else {
        Err("The response does not match the pending approval.".into())
    }
}

pub fn response(decision: &str) -> Value {
    match decision {
        "accept" => json!({"decision":decision,"confirmed":true}),
        "acceptForSession" => {
            json!({"decision":decision,"confirmed":true,"allowForSession":true})
        }
        "acceptAlways" => json!({"decision":decision,"confirmed":true}),
        "decline" => json!({"decision":decision,"confirmed":false}),
        _ => json!({"decision":decision,"cancelled":true}),
    }
}

pub fn validate(request: &Value, response: &Value) -> Result<(), String> {
    if !can_respond(request) {
        return Err("This request is not resumable.".into());
    }
    decision(response)?;
    Ok(())
}

fn persistence(value: &str) -> Option<&'static str> {
    let value = value.to_lowercase();
    if value.contains("session") {
        Some("acceptForSession")
    } else if ["always", "permanent", "forever", "persistent"]
        .iter()
        .any(|s| value.contains(s))
    {
        Some("acceptAlways")
    } else {
        None
    }
}

fn field_options(field: &Value) -> Vec<(&str, &str)> {
    if let Some(options) = field["oneOf"].as_array() {
        return options
            .iter()
            .filter_map(|o| Some((o["const"].as_str()?, o["title"].as_str().unwrap_or(""))))
            .collect();
    }
    field["enum"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, o)| Some((o.as_str()?, field["enumNames"][i].as_str().unwrap_or(""))))
        .collect()
}

fn persistence_field(key: &str, field: &Value) -> bool {
    key.eq_ignore_ascii_case("persist")
        || persistence(key).is_some()
        || persistence(field["title"].as_str().unwrap_or("")).is_some()
        || persistence(field["description"].as_str().unwrap_or("")).is_some()
}

fn nullable_strings(record: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().all(|key| {
        record
            .get(*key)
            .is_none_or(|v| v.is_null() || v.is_string())
    })
}

fn mcp_metadata(payload: &Value) -> Option<&Value> {
    let meta = payload.get("_meta")?;
    let record = meta.as_object()?;
    if !nullable_strings(
        record,
        &[
            "app",
            "app_name",
            "appName",
            "connector_name",
            "connectorName",
        ],
    ) || record
        .get("allowPersistentApproval")
        .is_some_and(|v| !v.is_null() && !v.is_boolean())
        || record.get("persist").is_some_and(|v| {
            !v.is_null()
                && !v.is_string()
                && !v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
        })
    {
        return None;
    }
    for (key, fields) in [
        ("target", &["app", "name"][..]),
        ("tool_params", &["app", "app_name"][..]),
    ] {
        if let Some(value) = record.get(key).filter(|v| !v.is_null())
            && !value
                .as_object()
                .is_some_and(|o| nullable_strings(o, fields))
        {
            return None;
        }
    }
    Some(meta)
}

fn mcp_form(payload: &Value) -> Option<&Value> {
    if payload["mode"] == "url" {
        return None;
    }
    let form = payload.get("requestedSchema")?;
    let record = form.as_object()?;
    if record.get("required").is_some_and(|v| {
        !v.is_null() && !v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
    }) {
        return None;
    }
    if let Some(properties) = record.get("properties") {
        for field in properties.as_object()?.values() {
            let field = field.as_object()?;
            if !nullable_strings(field, &["type", "title", "description"]) {
                return None;
            }
            for key in ["enum", "enumNames"] {
                if field.get(key).is_some_and(|v| {
                    !v.is_null() && !v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
                }) {
                    return None;
                }
            }
            if field.get("oneOf").is_some_and(|v| {
                !v.is_null()
                    && !v.as_array().is_some_and(|a| {
                        a.iter().all(|v| {
                            v.as_object().is_some_and(|o| {
                                o.get("const").is_some_and(Value::is_string)
                                    && nullable_strings(o, &["title"])
                            })
                        })
                    })
            }) {
                return None;
            }
        }
    }
    Some(form)
}

/// KSG server/provider/CodexMcpElicitation.ts. The native provider still receives its own schema.
pub fn mcp_response(payload: &Value, decision: &str) -> Value {
    if matches!(decision, "decline" | "cancel") {
        return json!({"action":decision});
    }
    if payload["mode"] == "url" {
        return json!({"action":"decline"});
    }
    let form = mcp_form(payload);
    let mut content = serde_json::Map::new();
    for (key, field) in form.unwrap_or(&Value::Null)["properties"]
        .as_object()
        .into_iter()
        .flatten()
    {
        let chosen = field_options(field).into_iter().find(|(value, _)| {
            if matches!(decision, "acceptForSession" | "acceptAlways") {
                persistence(value) == Some(decision)
            } else {
                let once = regex::Regex::new(r"(?i)^(?:once|accept|approve|allow)(?:[_\s-]*(?:once(?:[_\s-]*only)?|this[_\s-]*time))?$").expect("constant regex");
                once.is_match(value.trim()) && persistence(value).is_none()
            }
        });
        if let Some((value, _)) = chosen {
            content.insert(key.clone(), json!(value));
        } else if field["type"] == "boolean" && persistence_field(key, field) {
            content.insert(key.clone(), json!(decision == "acceptAlways"));
        } else if let Some(default) = field.get("default").filter(|v| !v.is_null()) {
            content.insert(key.clone(), default.clone());
        }
    }
    if form.unwrap_or(&Value::Null)["required"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|key| key.as_str().is_none_or(|key| !content.contains_key(key)))
    {
        return json!({"action":"decline"});
    }
    let mut response = json!({"action":"accept"});
    if form.is_some() {
        response["content"] = json!(content);
    }
    if matches!(decision, "acceptForSession" | "acceptAlways") {
        response["_meta"] =
            json!({"persist":if decision == "acceptForSession" {"session"} else {"always"}});
    }
    response
}

pub fn mcp_options(payload: &Value) -> Vec<OptionItem> {
    let mut labels = std::collections::HashMap::new();
    let metadata = mcp_metadata(payload).unwrap_or(&Value::Null);
    let persist = &metadata["persist"];
    for value in persist.as_str().into_iter().chain(
        persist
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str),
    ) {
        if let Some(decision) = persistence(value) {
            labels.insert(decision, String::new());
        }
    }
    if metadata["allowPersistentApproval"] == true {
        labels.insert("acceptAlways", String::new());
    }
    for (key, field) in mcp_form(payload).unwrap_or(&Value::Null)["properties"]
        .as_object()
        .into_iter()
        .flatten()
    {
        for (value, label) in field_options(field) {
            if let Some(decision) = persistence(value) {
                labels.insert(decision, label.to_owned());
            }
        }
        if field["type"] == "boolean" && persistence_field(key, field) {
            labels.insert(
                "acceptAlways",
                field["title"].as_str().unwrap_or("").to_owned(),
            );
        }
    }
    let mut options = defaults();
    options.retain(|o| o.decision != "acceptForSession");
    for (decision, fallback) in [
        ("acceptForSession", "Always allow this session"),
        ("acceptAlways", "Always allow"),
    ] {
        if let Some(label) = labels.get(decision)
            && mcp_response(payload, decision)["action"] == "accept"
        {
            options.insert(
                options.len() - 1,
                OptionItem {
                    decision: decision.into(),
                    label: if label.is_empty() {
                        fallback.into()
                    } else {
                        label.clone()
                    },
                    warning: None,
                },
            );
        }
    }
    options
}

pub fn mcp_app_name(payload: &Value) -> String {
    let meta = mcp_metadata(payload).unwrap_or(&Value::Null);
    for value in [
        &meta["app_name"],
        &meta["appName"],
        &meta["app"],
        &meta["target"]["app"],
        &meta["target"]["name"],
        &meta["tool_params"]["app_name"],
        &meta["tool_params"]["app"],
    ] {
        if let Some(value) = value.as_str() {
            return value.into();
        }
    }
    let pattern = regex::Regex::new(r"(?i)^Allow ChatGPT to use (.+?)\?$").expect("constant regex");
    if let Some(captures) = payload["message"]
        .as_str()
        .and_then(|s| pattern.captures(s))
    {
        return captures[1].into();
    }
    meta["connector_name"]
        .as_str()
        .or(meta["connectorName"].as_str())
        .or(payload["serverName"].as_str())
        .unwrap_or("")
        .into()
}

/// Claude must not grant a schema default the consent card never showed.
pub fn claude_mcp_acceptance(request: &Value) -> Option<Value> {
    if request["mode"] == "url" || !request["requested_schema"].is_object() {
        return None;
    }
    let mut schema = request["requested_schema"].clone();
    for field in schema["properties"]
        .as_object_mut()
        .into_iter()
        .flat_map(|p| p.values_mut())
    {
        if let Some(field) = field.as_object_mut() {
            field.remove("default");
        }
    }
    let response = mcp_response(&json!({"mode":"form","requestedSchema":schema}), "accept");
    let content = response["content"].as_object()?;
    (response["action"] == "accept"
        && content.values().all(|v| {
            v.is_string()
                || v.is_boolean()
                || v.is_number()
                || v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
        }))
    .then_some(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_persistence_defaults_and_required_fields_match_native_contract() {
        let payload = json!({"mode":"form","requestedSchema":{"properties":{
            "permission":{"oneOf":[{"const":"once","title":"Once"},{"const":"session","title":"This session"},{"const":"always","title":"Forever"}]},
            "extra":{"type":"string","default":"native default"}
        },"required":["permission","extra"]}});
        assert_eq!(
            mcp_options(&payload)
                .iter()
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["Cancel", "Decline", "This session", "Forever", "Approve"]
        );
        assert_eq!(
            mcp_response(&payload, "acceptForSession"),
            json!({"action":"accept","content":{"permission":"session","extra":"native default"},"_meta":{"persist":"session"}})
        );
        assert_eq!(
            mcp_response(&json!({"mode":"url"}), "accept"),
            json!({"action":"decline"})
        );
        assert_eq!(
            mcp_response(
                &json!({"requestedSchema":{"required":["missing"]}}),
                "accept"
            ),
            json!({"action":"decline"})
        );
        assert!(
            claude_mcp_acceptance(&json!({"requested_schema":payload["requestedSchema"]}))
                .is_none()
        );
    }

    #[test]
    fn defaults_empty_options_warnings_and_permission_guards_match_ksg() {
        assert_eq!(
            defaults()
                .iter()
                .map(|o| o.decision.as_str())
                .collect::<Vec<_>>(),
            ["cancel", "decline", "acceptForSession", "accept"]
        );
        assert!(options(&json!({"approvalOptions":[]})).is_empty());
        let request = json!({"approvalOptions":[{"decision":"accept","label":"Allow once","warning":"Check this command."}]});
        assert_eq!(
            options(&request)[0].warning.as_deref(),
            Some("Check this command.")
        );
        // Provider adapters decide how to consume contract decisions a stale client can send.
        assert!(validate(&request, &response("acceptForSession")).is_ok());
        assert!(validate(&json!({"canRespond":false}), &response("accept")).is_err());
        assert!(
            validate(
                &json!({"responseCapability":{"type":"not_resumable"}}),
                &response("accept")
            )
            .is_err()
        );
        assert_eq!(decision(&json!({"confirmed":false})).unwrap(), "decline");
        assert_eq!(decision(&json!({"cancelled":true})).unwrap(), "cancel");
    }
}
