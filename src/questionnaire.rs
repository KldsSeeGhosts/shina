//! KSG provider-pi/userInput.ts and ComposerPendingUserInputPanel behavior.
//! RPIV uses sequential select/input RPCs. Collect answers before replying to the first RPC,
//! then answer only the exact remaining dialogs from that same live tool call.
use serde_json::{Value, json};

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
    pub options: Vec<OptionItem>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Answer {
    pub selected: Vec<String>,
    pub other: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Questionnaire {
    pub questions: Vec<Question>,
    pub answers: Vec<Answer>,
    pub index: usize,
    rpc: Option<(String, Vec<String>)>,
}

fn text(value: &Value, field: &str) -> Option<String> {
    let text = value[field].as_str()?.trim();
    (!text.is_empty() && text.len() <= 32_768).then(|| text.to_owned())
}

fn questions(value: &Value) -> Option<Vec<Question>> {
    let values = value["questions"].as_array()?;
    if values.is_empty() || values.len() > 32 {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    values
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let id = text(q, "id").unwrap_or_else(|| format!("question-{i}"));
            if !seen.insert(id.clone()) {
                return None;
            }
            let options = q["options"].as_array()?;
            if options.len() > 64 {
                return None;
            }
            let mut seen = std::collections::HashSet::new();
            let options = options
                .iter()
                .map(|option| {
                    let label =
                        text(option, "label").or_else(|| option.as_str().map(str::to_owned))?;
                    let value = text(option, "value").unwrap_or_else(|| label.clone());
                    if !seen.insert(value.clone()) {
                        return None;
                    }
                    Some(OptionItem {
                        label,
                        value,
                        description: text(option, "description").unwrap_or_default(),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(Question {
                id,
                header: text(q, "header").unwrap_or_else(|| "Question".into()),
                text: text(q, "question")?,
                multi: q["multiSelect"] == true,
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
        let answers = vec![Answer::default(); questions.len()];
        Some(Self {
            questions,
            answers,
            index: 0,
            rpc: None,
        })
    }

    /// Match every title and serialized option exactly, as KSG's piQuestionnaireQuestion does.
    pub fn rpiv(request: &Value, tool_id: &str, params: &Value) -> Option<Self> {
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
        let answers = vec![Answer::default(); questions.len()];
        Some(Self {
            questions,
            answers,
            index: 0,
            rpc: Some((tool_id.into(), signatures)),
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
        if !answer.other.trim().is_empty() {
            return Some(json!(answer.other.trim()));
        }
        // Option order, not click order.
        let selected: Vec<_> = q
            .options
            .iter()
            .filter(|o| answer.selected.contains(&o.value))
            .map(|o| o.value.clone())
            .collect();
        if selected.is_empty() {
            None
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
        if !self.complete() {
            return None;
        }
        if self.rpc.is_some() {
            return Some(json!({"value": self.rpc_value(0)?}));
        }
        let answers = self
            .questions
            .iter()
            .enumerate()
            .map(|(i, q)| Some((q.id.clone(), self.answer(i)?)))
            .collect::<Option<serde_json::Map<_, _>>>()?;
        // Pi input/editor transports text. Keep the structured answer in that string.
        Some(json!({"value": Value::Object(answers).to_string()}))
    }

    fn rpc_value(&self, at: usize) -> Option<String> {
        let answer = self.answer(at)?;
        Some(if let Some(values) = answer.as_array() {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        } else {
            answer.as_str()?.to_owned()
        })
    }

    pub fn remaining_rpc(&self) -> Vec<(String, String, String)> {
        self.rpc
            .as_ref()
            .map(|(tool, signatures)| {
                signatures
                    .iter()
                    .enumerate()
                    .skip(1)
                    .filter_map(|(i, signature)| {
                        Some((tool.clone(), signature.clone(), self.rpc_value(i)?))
                    })
                    .collect()
            })
            .unwrap_or_default()
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
        assert_eq!(form.answer(1), Some(json!(["A", "B"])));
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
    fn rpiv_matches_exact_wire_format_and_builds_sequential_responses() {
        let params = fixture();
        let request = json!({"method":"select","title":"[Storage] Where?",
            "options":["1. Local \u{2014} On disk","2. Remote \u{2014} On server","3. Type something."]});
        let mut form = Questionnaire::rpiv(&request, "tool-1", &params).unwrap();
        form.digit("1");
        form.index = 1;
        form.digit("2");
        form.digit("1");
        assert_eq!(
            form.response(),
            Some(json!({"value":"1. Local \u{2014} On disk"}))
        );
        let replies = form.remaining_rpc();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].0, "tool-1");
        assert_eq!(replies[0].2, "1,2");
        let mut unrelated = request.clone();
        unrelated["options"][0] = json!("different");
        assert!(Questionnaire::rpiv(&unrelated, "tool-1", &params).is_none());
        unrelated = request;
        unrelated["title"] = json!("Where?");
        assert!(Questionnaire::rpiv(&unrelated, "tool-1", &params).is_none());
    }

    #[test]
    fn malformed_or_duplicate_questions_fall_back_to_native_dialog() {
        assert!(Questionnaire::parse(&json!({"questions":[]})).is_none());
        let mut params = fixture();
        params["questions"][1]["id"] = json!("one");
        assert!(Questionnaire::parse(&params).is_none());
        params = fixture();
        params["questions"][0]["options"][1]["label"] = json!("Local");
        assert!(Questionnaire::parse(&params).is_none());
    }
}
