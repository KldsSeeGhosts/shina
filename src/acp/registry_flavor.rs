//! Local command and registry ID matching from KSG's ACP registry adapter.
use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Flavor {
    #[default]
    Generic,
    Droid,
    Glm,
    Vibe,
}

impl Flavor {
    pub(super) fn detect(id: &str, command: &str) -> Self {
        match id {
            "factory-droid" => return Self::Droid,
            "glm-acp-agent" => return Self::Glm,
            "mistral-vibe" => return Self::Vibe,
            _ => {}
        }
        let command = command
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = command
            .strip_suffix(".exe")
            .or_else(|| command.strip_suffix(".cmd"))
            .unwrap_or(&command);
        match name {
            "droid" | "factory-droid" => Self::Droid,
            "glm" | "glm-acp-agent" => Self::Glm,
            "vibe" | "vibe-acp" | "mistral-vibe" | "mistral-vibe-acp" => Self::Vibe,
            _ => Self::Generic,
        }
    }
}

fn nonempty(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// Droid pairs each select with a free-text companion named `<id>_other`.
/// Recognize only the complete form and leave the companion out of the UI.
pub(super) fn droid_questions(params: &Value) -> Option<Vec<Value>> {
    if params["mode"] != "form" {
        return None;
    }
    let schema = &params["requestedSchema"];
    let properties = schema["properties"].as_object()?;
    let ids = schema["required"].as_array()?;
    if ids.is_empty() || ids.len() > 20 || properties.len() != ids.len() * 2 {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    ids.iter()
        .map(|id| {
            let id = id.as_str()?;
            let number = id.strip_prefix("question_")?;
            if number.starts_with('0')
                || number.is_empty()
                || !number.bytes().all(|c| c.is_ascii_digit())
                || !seen.insert(id)
            {
                return None;
            }
            let field = properties.get(id)?;
            if field["type"] != "string"
                || properties.get(&format!("{id}_other"))?["type"] != "string"
            {
                return None;
            }
            let choices = field["oneOf"].as_array()?;
            if choices.len() > 32 {
                return None;
            }
            let mut saw_other = false;
            let mut options = Vec::new();
            for choice in choices {
                let label = nonempty(&choice["title"])?;
                let value = choice["const"].as_str()?;
                if value == "other" {
                    saw_other = true;
                    continue;
                }
                let number = value.strip_prefix("option_")?;
                if number.is_empty() || !number.bytes().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                options.push(json!({"label":label,"description":label,"value":value}));
            }
            (saw_other && !options.is_empty()).then_some(json!({
                "id":id, "header":nonempty(&field["title"])?,
                "question":nonempty(&field["description"])?,
                "options":options, "allowCustomAnswer":true,
            }))
        })
        .collect()
}

pub(super) fn droid_answer(questions: &[Value], response: &Value) -> Value {
    let cancel = || json!({"action":"cancel"});
    if response["cancelled"] == true {
        return cancel();
    }
    let Some(answers) = response["value"]
        .as_str()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
    else {
        return cancel();
    };
    let mut content = Map::new();
    for question in questions {
        let id = question["id"].as_str().unwrap_or_default();
        let Some(answer) = answers[id]
            .as_str()
            .filter(|answer| !answer.trim().is_empty())
        else {
            return cancel();
        };
        let options = question["options"].as_array().unwrap();
        if options.iter().any(|option| option["value"] == answer) {
            content.insert(id.into(), json!(answer));
        } else {
            let text = if let Some((value, attachment)) = answer.split_once("\n\n")
                && (attachment.starts_with("Attached image \"")
                    || attachment.starts_with("Attached file \""))
                && let Some(choice) = options.iter().find(|option| option["value"] == value)
            {
                format!(
                    "{}\n\n{attachment}",
                    choice["label"].as_str().unwrap_or(value)
                )
            } else {
                answer.to_owned()
            };
            content.insert(id.into(), json!("other"));
            content.insert(format!("{id}_other"), json!(text));
        }
    }
    json!({"action":"accept","content":content})
}
