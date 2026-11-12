//! Shared usageLimits contract: {windows:[{id,name,usedPercent,resetsAt}]}.
//! resetsAt is Unix seconds, null when the provider does not report it.
use serde_json::{Value, json};

pub fn authentication_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "authentication",
        "unauthorized",
        "unauthorised",
        "invalid api key",
        "not logged in",
        "not authenticated",
        "login required",
        "please log in",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn codex(snapshot: &Value, previous: &Value) -> Value {
    // KSG keeps model-specific pools out of the main Codex account allowance.
    if snapshot["limitId"].as_str().is_some_and(|id| id != "codex") {
        return previous.clone();
    }
    let mut windows = previous["windows"].as_array().cloned().unwrap_or_default();
    let limit = snapshot["limitId"].as_str().unwrap_or("codex");
    for (key, fallback) in [("primary", "Primary"), ("secondary", "Secondary")] {
        let window = &snapshot[key];
        let Some(percent) = window["usedPercent"].as_f64().filter(|n| n.is_finite()) else {
            continue;
        };
        let id = format!("{limit}:{key}");
        let duration = window["windowDurationMins"].as_u64();
        let name = match duration {
            Some(n) if n >= 1440 && n % 1440 == 0 => format!("{} day", n / 1440),
            Some(n) if n >= 60 && n % 60 == 0 => format!("{} hour", n / 60),
            Some(n) => format!("{n} minute"),
            None => fallback.into(),
        };
        let name = snapshot["limitName"]
            .as_str()
            .map_or(name.clone(), |limit| format!("{limit} {name}"));
        let normalized = json!({"id":id,"name":name,"usedPercent":percent.clamp(0.0,100.0),
            "resetsAt":window["resetsAt"]});
        if let Some(old) = windows.iter_mut().find(|window| window["id"] == id) {
            *old = normalized;
        } else {
            windows.push(normalized);
        }
    }
    if let Some(remaining) = snapshot["individualLimit"]["remainingPercent"]
        .as_f64()
        .filter(|n| n.is_finite())
    {
        let normalized = json!({"id":"codex:individual","name":"Monthly credit limit",
            "usedPercent":(100.0-remaining).clamp(0.0,100.0),
            "resetsAt":snapshot["individualLimit"]["resetsAt"]});
        if let Some(old) = windows
            .iter_mut()
            .find(|window| window["id"] == "codex:individual")
        {
            *old = normalized;
        } else {
            windows.push(normalized);
        }
    } else if snapshot.get("individualLimit") == Some(&Value::Null) {
        windows.retain(|window| window["id"] != "codex:individual");
    }
    json!({"windows":windows})
}

pub fn claude(info: &Value, previous: &Value) -> Value {
    let mut windows = previous["windows"].as_array().cloned().unwrap_or_default();
    let id = info["rateLimitType"].as_str().unwrap_or("usage");
    if let Some(percent) = info["utilization"].as_f64().filter(|n| n.is_finite()) {
        let normalized = json!({"id":id,"name":id.replace('_'," "),
            "usedPercent":(percent * 100.0).clamp(0.0,100.0),"resetsAt":info["resetsAt"]});
        if let Some(old) = windows.iter_mut().find(|window| window["id"] == id) {
            *old = normalized;
        } else {
            windows.push(normalized);
        }
    }
    json!({"windows":windows})
}
