//! Provider-neutral work details. Structured result keys are documented in docs/harnesses.md.
use crate::model::Tool;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Command,
    FileChange,
    FileSearch,
    WebSearch,
    Read,
    List,
    Subagent,
    Plan,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    #[serde(default)]
    pub old_path: Option<String>,
    pub operation: String,
    #[serde(default)]
    pub diff: String,
    #[serde(default)]
    pub additions: usize,
    #[serde(default)]
    pub deletions: usize,
}

impl FileChange {
    pub fn new(path: &str, operation: &str, diff: &str) -> Self {
        Self {
            path: path.into(),
            old_path: None,
            operation: operation.into(),
            diff: diff.into(),
            additions: diff
                .lines()
                .filter(|l| diff_line_kind(l) == Some(true))
                .count(),
            deletions: diff
                .lines()
                .filter(|l| diff_line_kind(l) == Some(false))
                .count(),
        }
    }
}

pub fn diff_line_kind(line: &str) -> Option<bool> {
    if line.starts_with('+') && !line.starts_with("+++ ") && !line.starts_with("+++\t") {
        Some(true)
    } else if line.starts_with('-') && !line.starts_with("--- ") && !line.starts_with("---\t") {
        Some(false)
    } else {
        None
    }
}

/// Legacy records alone use the name classifier. New provider events set a kind explicitly.
pub fn pi_kind(name: &str) -> ToolKind {
    match name.to_ascii_lowercase().as_str() {
        "read" | "read_file" => ToolKind::Read,
        "edit" | "write" | "patch" | "edit_file" | "write_file" | "apply_patch" => {
            ToolKind::FileChange
        }
        "bash" | "command" | "powershell" | "shell" | "exec" | "exec_command" => ToolKind::Command,
        "grep" | "rg" | "search" | "find" => ToolKind::FileSearch,
        "web_search" | "web_fetch" | "fetch" => ToolKind::WebSearch,
        "ls" | "list" | "list_directory" => ToolKind::List,
        "subagent" | "delegate_task" => ToolKind::Subagent,
        "plan" | "update_plan" | "todo_list" => ToolKind::Plan,
        _ => ToolKind::Other,
    }
}

impl Tool {
    pub fn resolved_kind(&self) -> ToolKind {
        self.kind.unwrap_or_else(|| pi_kind(&self.name))
    }

    pub(crate) fn accept_metadata(&mut self, data: &Value) {
        if let Ok(kind) = serde_json::from_value(data["toolKind"].clone()) {
            self.kind = Some(kind);
        }
        if let Ok(changes) = serde_json::from_value::<Vec<FileChange>>(data["changes"].clone()) {
            self.changes = changes;
        }
    }

    pub(crate) fn accept_work_result(&mut self, result: &Value) {
        self.accept_metadata(result);
        if let Some(input) = result.get("input").filter(|v| !v.is_null()) {
            self.input = input.clone();
        }
        if self.resolved_kind() == ToolKind::FileChange && self.changes.is_empty() {
            let path = self.input["path"]
                .as_str()
                .or_else(|| self.input["file_path"].as_str());
            if let Some(path) = path {
                let patch = result["details"]["patch"]
                    .as_str()
                    .or_else(|| result["details"]["diff"].as_str())
                    .unwrap_or("");
                self.changes.push(FileChange::new(
                    path,
                    if self.name == "write" {
                        "add"
                    } else {
                        "update"
                    },
                    patch,
                ));
            }
        }
        // Preserve native payloads as well as the normalized fields.
        self.structured["input"] = self.input.clone();
        for key in ["fileSearchResults", "webResults"] {
            let results = result
                .get(key)
                .or_else(|| result["structuredContent"].get(key))
                .or_else(|| result["details"].get(key));
            if let Some(results) = results {
                self.structured[key] = results.clone();
            } else if matches!(
                (key, self.resolved_kind()),
                ("fileSearchResults", ToolKind::FileSearch) | ("webResults", ToolKind::WebSearch)
            ) {
                let results = result["structuredContent"]
                    .get("results")
                    .or_else(|| result["details"].get("results"));
                if let Some(results) = results {
                    self.structured[key] = results.clone();
                }
            }
        }
    }
}

/// ACP can provide old/new text instead of a patch. Diff the supplied text only,
/// without reading files or treating unchanged lines as additions and deletions.
pub fn text_diff(old: &str, new: &str) -> String {
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .to_string()
}

pub fn result_metadata(kind: ToolKind, changes: Vec<FileChange>, source: &Value) -> Value {
    json!({
        "toolKind":kind, "changes":changes,
        "fileSearchResults":source.get("results").filter(|_|kind == ToolKind::FileSearch),
        "webResults":source.get("results").filter(|_|kind == ToolKind::WebSearch)
    })
}

pub fn retry_events(thread: &crate::model::Thread) -> &[Value] {
    thread.harness_state["workRetries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub fn compaction_anchor(thread: &crate::model::Thread, item: &Value) -> crate::model::ToolAnchor {
    serde_json::from_value(item["anchor"].clone()).unwrap_or(crate::model::ToolAnchor {
        message: thread.messages.len(),
        offset: 0,
    })
}

/// Codex steps and ACP entries share KSG's pending/running/completed contract.
pub fn plan_data(source: &Value) -> Value {
    let steps = ["steps", "plan", "entries", "items"]
        .iter()
        .find_map(|key| source[*key].as_array())
        .into_iter()
        .flatten()
        .filter_map(|step| {
            let text = step["text"]
                .as_str()
                .or_else(|| step["step"].as_str())
                .or_else(|| step["content"].as_str())
                .or_else(|| step.as_str())?;
            let status = match step["status"].as_str().unwrap_or("") {
                "completed" | "done" => "completed",
                "in_progress" | "inProgress" | "running" => "running",
                _ => "pending",
            };
            Some(json!({"text":text,"status":status}))
        })
        .collect::<Vec<_>>();
    json!({"steps":steps,
        "markdown":source["markdown"].as_str().or_else(||source["text"].as_str()).or_else(||source["plan"].as_str()),
        "explanation":source["explanation"]})
}

/// The first call owns its position. Subsequent updates replace that call's payload.
/// Use ordinary tool history so existing projection and persistence paths carry plans.
pub fn save_plan(history: &mut Vec<Value>, id: &str, source: &Value) {
    let mut found = false;
    for message in history.iter_mut() {
        for block in message["content"].as_array_mut().into_iter().flatten() {
            if block["type"] == "toolCall" && block["id"] == id {
                block["arguments"] = source.clone();
                found = true;
            }
        }
    }
    let result = json!({"role":"toolResult","toolCallId":id,"isError":false,
        "content":[],"toolKind":"plan","structuredContent":{"plan":plan_data(source)}});
    if found {
        if let Some(previous) = history
            .iter_mut()
            .find(|m| m["role"] == "toolResult" && m["toolCallId"] == id)
        {
            *previous = result;
        }
    } else {
        history.push(
            json!({"role":"assistant","content":[{"type":"toolCall","id":id,
            "name":"plan","toolKind":"plan","arguments":source}]}),
        );
        history.push(result);
    }
}

/// KSG's summarizeToolGroup: two highest-priority actions, in first-seen order,
/// plus the number of calls not represented by those actions.
pub fn summarize(tools: &[&Tool]) -> String {
    use ToolKind::*;
    let mut groups: Vec<(ToolKind, Vec<&Tool>)> = vec![];
    for tool in tools {
        let kind = tool.resolved_kind();
        if let Some((_, calls)) = groups.iter_mut().find(|(k, _)| *k == kind) {
            calls.push(tool);
        } else {
            groups.push((kind, vec![tool]));
        }
    }
    let mut selected: Vec<_> = (0..groups.len()).collect();
    selected.sort_by_key(|i| {
        (
            match groups[*i].0 {
                Command | FileChange | Subagent => 0,
                Other | Plan => 2,
                _ => 1,
            },
            *i,
        )
    });
    selected.truncate(2);
    selected.sort_unstable();
    let count = |n, one, many| format!("{n} {}", if n == 1 { one } else { many });
    let mut labels = vec![];
    for i in &selected {
        let (kind, calls) = &groups[*i];
        let running = calls.iter().any(|t| !t.done && !t.failed);
        let n = if *kind == FileChange {
            let mut paths = std::collections::HashSet::new();
            let mut unknown = 0;
            for call in calls {
                if call.changes.is_empty() {
                    let path = call.input["path"]
                        .as_str()
                        .or_else(|| call.input["file_path"].as_str())
                        .unwrap_or(&call.detail);
                    if path.is_empty() {
                        unknown += 1;
                    } else {
                        paths.insert(path.replace('\\', "/"));
                    }
                } else {
                    paths.extend(call.changes.iter().map(|c| c.path.replace('\\', "/")));
                }
            }
            paths.len() + unknown
        } else {
            calls.len()
        };
        labels.push(match kind {
            Command => format!(
                "{} {}",
                if running { "Running" } else { "Ran" },
                count(n, "command", "commands")
            ),
            FileChange => format!(
                "{} {}",
                if running { "Changing" } else { "Changed" },
                count(n, "file", "files")
            ),
            FileSearch => format!(
                "{} code {}",
                if running { "Searching" } else { "Searched" },
                count(n, "time", "times")
            ),
            WebSearch => format!(
                "{} the web {}",
                if running { "Searching" } else { "Searched" },
                count(n, "time", "times")
            ),
            Read => format!(
                "{} {}",
                if running { "Reading" } else { "Read" },
                count(n, "file", "files")
            ),
            List => format!(
                "{} {}",
                if running { "Listing" } else { "Listed" },
                count(n, "directory", "directories")
            ),
            Subagent => format!(
                "{} {}",
                if running { "Delegating" } else { "Delegated" },
                count(n, "task", "tasks")
            ),
            Plan => format!(
                "{} {}",
                if running { "Updating" } else { "Updated" },
                count(n, "plan", "plans")
            ),
            Other => format!(
                "{} {}",
                if running { "Using" } else { "Used" },
                count(n, "tool", "tools")
            ),
        });
    }
    let remaining: Vec<_> = groups
        .iter()
        .enumerate()
        .filter(|(i, _)| !selected.contains(i))
        .flat_map(|(_, (_, calls))| calls)
        .collect();
    if !remaining.is_empty() {
        labels.push(format!(
            "{} {}",
            if remaining.iter().any(|t| !t.done && !t.failed) {
                "Performing"
            } else {
                "Performed"
            },
            count(remaining.len(), "other action", "other actions")
        ));
    }
    for label in labels.iter_mut().skip(1) {
        label.replace_range(..1, &label[..1].to_ascii_lowercase());
    }
    match labels.len() {
        0 => String::new(),
        1 => labels.remove(0),
        2 => labels.join(" and "),
        _ => format!(
            "{}, and {}",
            labels[..labels.len() - 1].join(", "),
            labels.last().unwrap()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Thread;

    #[test]
    fn diff_counts_include_increment_decrement_lines_but_not_file_headers() {
        let change = FileChange::new(
            "a.rs",
            "update",
            "--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n---old;\n+++new;\n",
        );
        assert_eq!((change.additions, change.deletions), (1, 1));
    }

    #[test]
    fn text_diffs_keep_context_zero_length_ranges_and_missing_final_newlines() {
        let diff = text_diff("same\nold\n", "same\nnew\n");
        let change = FileChange::new("a.rs", "update", &diff);
        assert_eq!((change.additions, change.deletions), (1, 1));
        assert!(diff.contains(" same\n"));
        assert!(text_diff("", "new\n").contains("-0,0"));
        assert!(text_diff("old\n", "").contains("+0,0"));
        assert!(text_diff("old", "new").contains("\\ No newline at end of file"));
    }

    #[test]
    fn summaries_prioritize_changes_commands_and_delegations_and_count_distinct_paths() {
        let mut thread = Thread::new(1);
        for (i, kind) in [
            ToolKind::Read,
            ToolKind::FileChange,
            ToolKind::FileChange,
            ToolKind::Command,
            ToolKind::WebSearch,
            ToolKind::Other,
        ]
        .into_iter()
        .enumerate()
        {
            thread.apply(
                json!({"type":"tool_execution_start","toolCallId":i.to_string(),
                "toolName":"opaque","toolKind":kind,"args":{"path":"a.rs"}}),
            );
        }
        for tool in &mut thread.tools {
            tool.done = true;
        }
        thread.tools[1].changes = vec![
            FileChange::new("a.rs", "update", ""),
            FileChange::new("b.rs", "delete", ""),
        ];
        assert_eq!(
            summarize(&thread.tools.iter().collect::<Vec<_>>()),
            "Changed 2 files, ran 1 command, and performed 3 other actions"
        );
        thread.tools[3].done = false;
        assert_eq!(
            summarize(&thread.tools.iter().collect::<Vec<_>>()),
            "Changed 2 files, running 1 command, and performed 3 other actions"
        );
        thread.tools[0].done = false;
        assert_eq!(
            summarize(&thread.tools.iter().collect::<Vec<_>>()),
            "Changed 2 files, running 1 command, and performing 3 other actions"
        );
        assert_eq!(
            summarize(&[&thread.tools[3], &thread.tools[1]]),
            "Running 1 command and changed 2 files"
        );
    }
}
