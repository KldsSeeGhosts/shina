//! KSG provider-pi/userInput.ts and ComposerPendingUserInputPanel behavior.
//! RPIV uses one select/input RPC per question. Preserve that native response boundary.
use serde_json::{Value, json};

#[cfg(test)]
#[path = "question_attachment_contract_tests.rs"]
mod attachment_contract_tests;
#[cfg(test)]
#[path = "questionnaire_contract_tests.rs"]
mod contract_tests;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct OptionItem {
    pub label: String,
    pub description: String,
    pub value: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub text: String,
    pub multi: bool,
    #[serde(default = "yes")]
    pub allow_custom: bool,
    #[serde(default)]
    pub initial_answer: Option<String>,
    #[serde(default = "yes")]
    pub required: bool,
    pub options: Vec<OptionItem>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Answer {
    pub selected: Vec<String>,
    pub other: String,
    #[serde(default)]
    pub attachment_count: usize,
    #[serde(default)]
    pub attachments_blocked: bool,
    #[serde(default)]
    pub touched: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Questionnaire {
    pub questions: Vec<Question>,
    pub answers: Vec<Answer>,
    pub index: usize,
    #[serde(default)]
    direct: bool,
    #[serde(default = "live")]
    pub capability: String,
    #[serde(default)]
    pub response_mode: Option<String>,
}

fn yes() -> bool {
    true
}
fn live() -> String {
    "live".into()
}

fn text(value: &Value, field: &str) -> Option<String> {
    let text = value[field].as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn questions(value: &Value) -> Option<Vec<Question>> {
    let values = value["questions"].as_array()?;
    if values.is_empty() {
        return None;
    }
    values
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let id = text(q, "id").unwrap_or_else(|| format!("question-{i}"));
            let options = q["options"].as_array()?;
            let options = options
                .iter()
                .map(|option| {
                    let label =
                        text(option, "label").or_else(|| option.as_str().map(str::to_owned))?;
                    let value = option["value"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| label.trim().to_owned());
                    Some(OptionItem {
                        label,
                        value,
                        description: option["description"].as_str().unwrap_or("").to_owned(),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(Question {
                id,
                header: text(q, "header").unwrap_or_else(|| "Question".into()),
                text: text(q, "question")?,
                multi: q["multiSelect"] == true,
                allow_custom: q["allowCustomAnswer"] != false,
                initial_answer: q["initialAnswer"].as_str().map(str::to_owned),
                required: q["required"] != false,
                options,
            })
        })
        .collect()
}

pub fn signature(request: &Value) -> String {
    json!([
        request["method"],
        request["title"],
        if request["method"] == "select" {
            &request["options"]
        } else {
            &Value::Null
        }
    ])
    .to_string()
}

impl Questionnaire {
    pub fn parse(request: &Value) -> Option<Self> {
        let questions = questions(request)?;
        let mut drafts = std::collections::HashMap::new();
        let answers = questions
            .iter()
            .map(|q| {
                drafts
                    .entry(q.id.clone())
                    .or_insert_with(|| Answer {
                        other: if q.allow_custom {
                            q.initial_answer.clone().unwrap_or_default()
                        } else {
                            String::new()
                        },
                        ..Default::default()
                    })
                    .clone()
            })
            .collect();
        Some(Self {
            questions,
            answers,
            index: 0,
            direct: false,
            capability: request["responseCapability"]["type"]
                .as_str()
                .unwrap_or("live")
                .into(),
            response_mode: request["responseMode"].as_str().map(str::to_owned),
        })
    }

    /// KSG provider-pi adapter's piQuestion and piUiResponse.
    pub fn pi(request: &Value) -> Option<Self> {
        let method = request["method"].as_str()?;
        if !matches!(method, "select" | "input" | "editor") {
            return None;
        }
        let title = request["title"].as_str().unwrap_or("Question");
        let detail = ["message", "placeholder"]
            .into_iter()
            .find_map(|k| request[k].as_str().filter(|s| !s.trim().is_empty()));
        let options = if method == "select" {
            request["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|v| {
                    json!({"label":if v.is_empty() {"Empty value"} else {v},
                    "description":if v.is_empty() {"Empty value"} else {v},"value":v})
                })
                .collect()
        } else if method == "input" {
            vec![
                json!({"label":"Submit empty value","description":"Send an empty string to the extension.","value":""}),
            ]
        } else {
            vec![]
        };
        let mut q = json!({"id":request["id"].as_str().unwrap_or("question"),
            "header":if detail.is_some() {title} else {"Question"},
            "question":detail.unwrap_or(title),"options":options});
        if method == "editor" {
            q["initialAnswer"] = json!(request["prefill"].as_str().unwrap_or(""));
        }
        let mut form = Self::parse(
            &json!({"questions":[q],"responseCapability":request["responseCapability"]}),
        )?;
        form.direct = true;
        Some(form)
    }

    pub fn can_respond(&self) -> bool {
        matches!(self.capability.as_str(), "live" | "message")
    }

    pub fn edit(&mut self, value: String) {
        let q = &self.questions[self.index];
        if !q.allow_custom {
            return;
        }
        let answer = &mut self.answers[self.index];
        if q.initial_answer.is_some() || !value.trim().is_empty() {
            answer.selected.clear();
        }
        answer.other = value;
        answer.touched = true;
        self.share_answer();
    }

    pub fn edit_at(&mut self, at: usize, value: String) {
        let previous = self.index;
        self.index = at;
        self.edit(value);
        self.index = previous;
    }

    fn share_answer(&mut self) {
        let id = &self.questions[self.index].id;
        let answer = self.answers[self.index].clone();
        for (question, draft) in self.questions.iter().zip(&mut self.answers) {
            if question.id == *id {
                *draft = answer.clone();
            }
        }
    }

    /// Match every title and serialized option exactly, as KSG's piQuestionnaireQuestion does.
    pub fn rpiv(request: &Value, _tool_id: &str, params: &Value) -> Option<Self> {
        let mut questions = questions(params)?;
        let raw = params["questions"].as_array()?;
        let mut signatures = Vec::new();
        for (q, raw) in questions.iter_mut().zip(raw) {
            if q.options.is_empty() {
                return None;
            }
            let mut lines = Vec::new();
            let mut previews = Vec::new();
            for (i, option) in q.options.iter_mut().enumerate() {
                // Wire format, not interface punctuation.
                let description = text(&raw["options"][i], "description")?;
                let line = format!("{}. {} \u{2014} {description}", i + 1, option.label);
                option.value = if q.multi {
                    (i + 1).to_string()
                } else {
                    line.clone()
                };
                lines.push(line);
                if let Some(preview) = text(&raw["options"][i], "preview") {
                    previews.push(format!(
                        "--- {}. {} preview ---\n{}",
                        i + 1,
                        option.label,
                        preview.chars().take(600).collect::<String>()
                    ));
                }
            }
            let header = text(raw, "header")
                .map(|h| format!("[{h}] "))
                .unwrap_or_default();
            let mut title = format!("{header}{}", q.text);
            let record = if q.multi {
                title.push_str(&format!("\n\n{}\n\nEnter the numbers of all that apply, comma-separated (e.g. \"1,3\"). Add \"; custom answer\" to include your own text, or type only custom text.", lines.join("\n")));
                json!({"method":"input","title":title})
            } else {
                if !previews.is_empty() {
                    let preview = format!("\n\n{}", previews.join("\n\n"));
                    title.push_str(&preview);
                    q.text.push_str(&preview);
                }
                let sentinel = format!("{}. Type something.", lines.len() + 1);
                let supplied = request["options"].as_array().filter(|values| {
                    request["method"] == "select"
                        && request["title"] == title
                        && values.len() == lines.len() + 1
                        && lines.iter().zip(*values).all(|(line, value)| value == line)
                });
                let last = supplied
                    .and_then(|values| values.last()?.as_str())
                    .unwrap_or(&sentinel);
                if last != sentinel {
                    q.options.push(OptionItem {
                        label: last.into(),
                        description: String::new(),
                        value: last.into(),
                    });
                }
                lines.push(last.into());
                json!({"method":"select","title":title,"options":lines})
            };
            signatures.push(signature(&record));
        }
        let at = signatures.iter().position(|s| *s == signature(request))?;
        // Earlier questions have already been answered through RPC.
        questions.drain(..at);
        signatures.drain(..at);
        questions.truncate(1);
        signatures.truncate(1);
        if raw[at]["header"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
        {
            questions[0].header = if questions[0].multi {
                "Select options"
            } else {
                "Select an option"
            }
            .into();
        }
        let answers = vec![Answer::default(); questions.len()];
        Some(Self {
            questions,
            answers,
            index: 0,
            direct: true,
            capability: live(),
            response_mode: None,
        })
    }

    pub fn toggle(&mut self, index: usize) -> bool {
        let q = &self.questions[self.index];
        let Some(option) = q.options.get(index) else {
            return false;
        };
        let answer = &mut self.answers[self.index];
        answer.other.clear();
        if q.multi {
            if let Some(at) = answer.selected.iter().position(|s| s == &option.value) {
                answer.selected.remove(at);
            } else {
                answer.selected.push(option.value.clone());
            }
        } else {
            answer.selected = vec![option.value.clone()];
        }
        self.share_answer();
        true
    }

    pub fn digit_index(&self, digit: &str) -> Option<usize> {
        if digit.len() != 1 {
            return None;
        }
        digit
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=9).contains(n))
            .map(|n| n - 1)
            .filter(|i| *i < self.questions[self.index].options.len())
    }

    #[cfg(test)]
    fn digit(&mut self, digit: &str) -> bool {
        self.digit_index(digit)
            .is_some_and(|index| self.toggle(index))
    }

    pub fn answer(&self, at: usize) -> Option<Value> {
        let q = &self.questions[at];
        let answer = &self.answers[at];
        if answer.attachments_blocked {
            return None;
        }
        let custom = q
            .allow_custom
            .then(|| {
                if q.initial_answer.is_some() {
                    answer.other.as_str()
                } else {
                    answer.other.trim()
                }
            })
            .filter(|s| q.initial_answer.is_some() || !s.is_empty());
        if let Some(custom) = custom.filter(|s| !s.is_empty()) {
            return Some(json!(custom));
        }
        let mut selected = Vec::new();
        for value in &answer.selected {
            if q.options.iter().any(|o| o.value == *value) && !selected.contains(value) {
                selected.push(value.clone());
            }
        }
        if selected.is_empty() {
            custom
                .map(|value| json!(value))
                .or_else(|| (q.allow_custom && answer.attachment_count > 0).then(|| json!("")))
        } else if q.multi {
            Some(json!(selected))
        } else {
            Some(json!(selected[0]))
        }
    }

    pub fn complete(&self) -> bool {
        (0..self.questions.len()).all(|at| self.answer(at).is_some())
    }

    pub fn response(&self) -> Option<Value> {
        if !self.can_respond() || !self.complete() {
            return None;
        }
        self.response_with_answers(&self.resolved_answers()?)
    }

    pub fn resolved_answers(&self) -> Option<serde_json::Map<String, Value>> {
        self.questions
            .iter()
            .enumerate()
            .map(|(i, q)| Some((q.id.clone(), self.answer(i)?)))
            .collect()
    }

    pub fn response_with_answers(&self, answers: &serde_json::Map<String, Value>) -> Option<Value> {
        if !self.can_respond() {
            return None;
        }
        if self.direct {
            let answer = answers.get(&self.questions.first()?.id)?;
            let value = if let Some(values) = answer.as_array() {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                answer.as_str()?.to_owned()
            };
            return Some(json!({"value":value}));
        }
        // Pi input/editor transports text. Keep the structured answer in that string.
        Some(json!({"value": Value::Object(answers.clone()).to_string()}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        json!({"questions":[
            {"id":"one","header":"Storage","question":"Where?","options":[{"label":"Local","description":"On disk"},{"label":"Remote","description":"On server"}]},
            {"id":"two","question":"Which?","multiSelect":true,"options":[{"label":"A","description":"First"},{"label":"B","description":"Second"}]}
        ]})
    }

    #[test]
    fn parse_answer_round_trip_and_keyboard_selection() {
        let mut form = Questionnaire::parse(&fixture()).unwrap();
        assert!(!form.complete());
        for invalid in ["0", "10", "x", "3"] {
            assert!(!form.digit(invalid));
        }
        assert!(form.digit("2"));
        assert_eq!(form.answer(0), Some(json!("Remote")));
        form.index = 1;
        form.digit("2");
        form.digit("1");
        assert_eq!(form.answer(1), Some(json!(["B", "A"])));
        form.digit("2");
        assert_eq!(form.answer(1), Some(json!(["A"])));
        let response = form.response().unwrap();
        let decoded: Value = serde_json::from_str(response["value"].as_str().unwrap()).unwrap();
        assert_eq!(decoded, json!({"one":"Remote","two":["A"]}));
        form.answers[1].other = "Something else".into();
        assert_eq!(form.answer(1), Some(json!("Something else")));
        form.digit("2");
        assert!(form.answers[1].other.is_empty());
    }

    #[test]
    fn rpiv_matches_exact_wire_format_and_answers_only_the_current_native_dialog() {
        let params = fixture();
        let request = json!({"method":"select","title":"[Storage] Where?",
            "options":["1. Local \u{2014} On disk","2. Remote \u{2014} On server","3. Type something."]});
        let mut form = Questionnaire::rpiv(&request, "tool-1", &params).unwrap();
        form.digit("1");
        assert_eq!(form.questions.len(), 1);
        assert_eq!(
            form.response(),
            Some(json!({"value":"1. Local \u{2014} On disk"}))
        );
        let mut unrelated = request.clone();
        unrelated["options"][0] = json!("different");
        assert!(Questionnaire::rpiv(&unrelated, "tool-1", &params).is_none());
        unrelated = request;
        unrelated["title"] = json!("Where?");
        assert!(Questionnaire::rpiv(&unrelated, "tool-1", &params).is_none());
    }

    #[test]
    fn empty_questions_fall_back_and_duplicate_ids_share_their_drafts() {
        assert!(Questionnaire::parse(&json!({"questions":[]})).is_none());
        let mut params = fixture();
        params["questions"][1]["id"] = json!("one");
        let mut form = Questionnaire::parse(&params).unwrap();
        form.edit("shared answer".into());
        assert_eq!(form.answers[1].other, "shared answer");
        params = fixture();
        params["questions"][0]["options"][1]["label"] = json!("Local");
        assert!(Questionnaire::parse(&params).is_some());
    }
}
