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
