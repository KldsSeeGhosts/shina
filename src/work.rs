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
                .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
                .count(),
            deletions: diff
                .lines()
                .filter(|l| l.starts_with('-') && !l.starts_with("---"))
                .count(),
        }
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

/// ACP provides old/new text, not a patch. Keep every line in a valid whole-file hunk.
pub fn text_diff(old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    let mut diff = format!(
        "@@ -1,{} +1,{} @@\n",
        old.lines().count(),
        new.lines().count()
    );
    for (prefix, text) in [('-', old), ('+', new)] {
        for line in text.lines() {
            diff.push(prefix);
            diff.push_str(line);
            diff.push('\n');
        }
    }
    diff
}

pub fn result_metadata(kind: ToolKind, changes: Vec<FileChange>, source: &Value) -> Value {
    json!({
        "toolKind":kind, "changes":changes,
        "fileSearchResults":source.get("results").filter(|_|kind == ToolKind::FileSearch),
        "webResults":source.get("results").filter(|_|kind == ToolKind::WebSearch)
    })
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
