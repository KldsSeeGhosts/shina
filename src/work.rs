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
    Html,
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
    if name.rsplit("__").next() == Some("html_render") {
        return ToolKind::Html;
    }
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
        if self.name.rsplit("__").next() == Some("html_render")
            && let Some(reference) = crate::html_render::find_reference(result)
        {
            self.kind = Some(ToolKind::Html);
            self.structured["htmlRender"] = json!(reference);
        }
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

/// Shells a `-c`-style flag unwraps to its script, as KSG Code's `withoutShellWrapper`
/// (`work-log/commandLabel.ts`) does for `bash -lc "…"` and `powershell -Command "…"`.
const SHELL_PROGRAMS: &[&str] = &["sh", "bash", "zsh", "dash", "ash", "ksh", "fish"];
const WINDOWS_SHELL_PROGRAMS: &[&str] = &["cmd", "powershell", "pwsh"];

/// A quote-aware whitespace split, enough of KSG's `tokenizeShellCommand` for
/// wrapper stripping and program names.
fn shell_tokens(command: &str) -> Vec<String> {
    let mut tokens = vec![];
    let mut current = String::new();
    let mut quote = '\0';
    let mut escape = false;
    for c in command.trim().chars() {
        if escape {
            current.push(c);
            escape = false;
            continue;
        }
        match c {
            '\\' if quote != '\'' => escape = true,
            '"' | '\'' if quote == '\0' => quote = c,
            _ if c == quote => quote = '\0',
            _ if c.is_whitespace() && quote == '\0' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn token_program(token: &str) -> String {
    token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(token)
        .trim_end_matches(".exe")
        .to_ascii_lowercase()
}

/// KSG's `withoutShellWrapper`: a `sh -c`/`cmd /c`/`powershell -Command` invocation's
/// script is what the row reads. Anything else keeps its own text.
fn without_shell_wrapper(command: &str) -> String {
    let tokens = shell_tokens(command);
    if tokens.is_empty() {
        return command.trim().to_owned();
    }
    let program = token_program(&tokens[0]);
    let script_index = if SHELL_PROGRAMS.contains(&program.as_str()) {
        tokens[1..]
            .iter()
            .position(|arg| {
                arg == "--command"
                    || arg
                        .strip_prefix('-')
                        .is_some_and(|flags| !flags.is_empty() && flags.contains('c'))
            })
            .map(|i| i + 2)
    } else if WINDOWS_SHELL_PROGRAMS.contains(&program.as_str()) {
        tokens[1..]
            .iter()
            .position(|arg| {
                arg.eq_ignore_ascii_case("/c")
                    || arg.eq_ignore_ascii_case("-command")
                    || arg.eq_ignore_ascii_case("-c")
            })
            .map(|i| i + 2)
    } else {
        None
    };
    match script_index {
        // The script must be the last token, as KSG requires.
        Some(index) if index == tokens.len() - 1 => tokens[index].clone(),
        _ => command.trim().to_owned(),
    }
}

/// Control characters as their Unicode pictures, as KSG's `withVisibleControlCharacters`.
fn visible_control_characters(command: &str) -> String {
    command
        .chars()
        .map(|c| match c {
            '\n' => '␊',
            '\r' => '␍',
            '\t' => '␉',
            '\u{b}' => '␋',
            '\u{c}' => '␌',
            _ => c,
        })
        .collect()
}

/// Ported from KSG Code's `commandDisplayText`: the command a row shows, shell wrapper
/// stripped and control characters drawn.
pub fn command_display_text(command: &str) -> String {
    visible_control_characters(&without_shell_wrapper(command))
}

/// Ported from KSG Code's `commandProgramName` for the live row ("Running cargo"): the
/// basename of what the command actually runs, unwrapped like the row's own text.
pub fn command_program_name(command: &str) -> Option<String> {
    let tokens = shell_tokens(&without_shell_wrapper(command));
    tokens.first().map(|token| token_program(token))
}

/// The workspace-relative path KSG's `formatWorkspaceRelativePath` shows.
pub fn relative_path(path: &str, root: &std::path::Path) -> String {
    // Projection paths belong to their host. Do not canonicalize or read their files.
    let root = root.to_string_lossy().replace('\\', "/");
    let path = path.replace('\\', "/");
    path.strip_prefix(&format!("{}/", root.trim_end_matches('/')))
        .unwrap_or(&path)
        .to_owned()
}

/// KSG's `SEARCH_QUERY_KEYS`, `SEARCH_GLOB_KEYS` and `SEARCH_TARGET_KEYS`
/// (`packages/shared/src/toolActivity.ts`).
const SEARCH_QUERY_KEYS: &[&str] = &["pattern", "query", "searchTerm", "regex", "grep", "needle"];
const SEARCH_GLOB_KEYS: &[&str] = &[
    "glob",
    "globPattern",
    "glob_pattern",
    "include",
    "filePattern",
    "file_pattern",
];
const SEARCH_TARGET_KEYS: &[&str] = &[
    "path",
    "target_directory",
    "targetDirectory",
    "directory",
    "cwd",
    "root",
];

fn first_input_string<'a>(
    input: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| input.get(*key)?.as_str().map(str::trim))
        .filter(|value| !value.is_empty())
}

/// KSG's `searchTargetName`: the last non-dot segment of the target path.
fn search_target_name(value: &str) -> String {
    value
        .rsplit(['\\', '/'])
        .find(|part| !part.is_empty() && *part != ".")
        .unwrap_or(value)
        .to_owned()
}

/// Ported from KSG's `formatSearchToolLabel`: "Searched {query} in {target}",
/// "Searched files {glob}", "Searched in {target}", in that priority.
pub fn search_label(tool: &Tool) -> Option<String> {
    let input = tool.input.as_object()?;
    let query = first_input_string(input, SEARCH_QUERY_KEYS);
    let glob = first_input_string(input, SEARCH_GLOB_KEYS);
    let target = first_input_string(input, SEARCH_TARGET_KEYS).map(|v| search_target_name(v));
    match (query, glob, target) {
        (Some(query), _, Some(target)) => Some(format!("Searched {query} in {target}")),
        (_, Some(glob), Some(target)) => Some(format!("Searched files {glob} in {target}")),
        (_, Some(glob), None) => Some(format!("Searched files {glob}")),
        (Some(query), _, None) => Some(format!("Searched {query}")),
        (None, None, Some(target)) => Some(format!("Searched in {target}")),
        (None, None, None) => None,
    }
}

/// KSG's `PATH_KEYS` and the nested records `collectToolFilePaths` walks
/// (`packages/shared/src/toolActivity.ts`), depth 4 and at most 8 paths.
const PATH_KEYS: &[&str] = &[
    "path",
    "filePath",
    "file_path",
    "relativePath",
    "filename",
    "fileName",
    "newPath",
    "oldPath",
];
const PATH_NEST_KEYS: &[&str] = &[
    "locations",
    "item",
    "input",
    "result",
    "rawInput",
    "data",
    "changes",
];

fn collect_paths(
    value: &Value,
    paths: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 4 || paths.len() >= 8 {
        return;
    }
    if let Value::Array(items) = value {
        for item in items {
            collect_paths(item, paths, seen, depth + 1);
            if paths.len() >= 8 {
                return;
            }
        }
        return;
    }
    let Some(record) = value.as_object() else {
        return;
    };
    for key in PATH_KEYS {
        if let Some(candidate) = record
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            && seen.insert(candidate.to_owned())
        {
            paths.push(candidate.to_owned());
            if paths.len() >= 8 {
                return;
            }
        }
    }
    for key in PATH_NEST_KEYS {
        if let Some(nested) = record.get(*key) {
            collect_paths(nested, paths, seen, depth + 1);
            if paths.len() >= 8 {
                return;
            }
        }
    }
}

/// KSG's `workEntryReadRawPaths`: the call's changed files first, then the paths its
/// input names, deduplicated.
pub fn read_paths(tool: &Tool) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut paths: Vec<String> = Vec::new();
    for change in &tool.changes {
        if seen.insert(change.path.clone()) {
            paths.push(change.path.clone());
        }
    }
    if paths.is_empty() {
        collect_paths(&tool.input, &mut paths, &mut seen, 0);
    }
    paths
}

/// KSG's `TEST_COMMAND_PATTERN` (`shared/toolOutput.ts` `commandRunsTests`): a failed
/// test run styles its row as a failure, not a failed call.
pub fn command_runs_tests(command: &str) -> bool {
    const RUNNERS: &[&str] = &[
        "vitest", "jest", "pytest", "mocha", "ava", "rspec", "phpunit", "ctest",
    ];
    const WRAPPED: &[&str] = &[
        "bun", "deno", "go", "cargo", "dotnet", "mix", "swift", "npm", "pnpm", "yarn", "vp",
    ];
    const TEST_SHORT: &[&str] = &["npm", "pnpm", "yarn", "bun"];
    let tokens = shell_tokens(&without_shell_wrapper(command));
    let programs: Vec<String> = tokens.iter().map(|t| token_program(t)).collect();
    let test_arg = |arg: &str| arg == "test" || arg.starts_with("test:");
    for (i, program) in programs.iter().enumerate() {
        let next = programs.get(i + 1).map(String::as_str);
        if RUNNERS.contains(&program.as_str())
            || (program == "playwright" && next.is_some_and(test_arg))
        {
            return true;
        }
        if WRAPPED.contains(&program.as_str())
            && (next.is_some_and(test_arg)
                || (TEST_SHORT.contains(&program.as_str()) && next == Some("t"))
                || (next == Some("run")
                    && programs
                        .get(i + 2)
                        .map(String::as_str)
                        .is_some_and(|arg| test_arg(arg) || arg == "t")))
        {
            return true;
        }
    }
    false
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
                Other | Plan | Html => 2,
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
            Command => format!("Ran {}", count(n, "command", "commands")),
            FileChange => format!("Changed {}", count(n, "file", "files")),
            FileSearch => format!("Searched code {}", count(n, "time", "times")),
            WebSearch => format!("Searched the web {}", count(n, "time", "times")),
            Read => format!("Read {}", count(n, "file", "files")),
            List => format!("Listed {}", count(n, "directory", "directories")),
            Subagent => format!("Delegated {}", count(n, "task", "tasks")),
            Plan => format!("Updated {}", count(n, "plan", "plans")),
            Html => format!("Rendered {}", count(n, "page", "pages")),
            Other => format!("Used {}", count(n, "tool", "tools")),
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
            "Performed {}",
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
        // KSG's summary is always past tense, even for calls still running.
        thread.tools[3].done = false;
        thread.tools[0].done = false;
        assert_eq!(
            summarize(&thread.tools.iter().collect::<Vec<_>>()),
            "Changed 2 files, ran 1 command, and performed 3 other actions"
        );
        assert_eq!(
            summarize(&[&thread.tools[3], &thread.tools[1]]),
            "Ran 1 command and changed 2 files"
        );
    }

    #[test]
    fn command_rows_show_the_script_shell_wrappers_run() {
        assert_eq!(command_display_text("cargo test"), "cargo test");
        assert_eq!(
            command_display_text(r#"bash -lc "cargo test -j 4""#),
            "cargo test -j 4"
        );
        assert_eq!(
            command_display_text("powershell -Command Get-ChildItem"),
            "Get-ChildItem"
        );
        // A script that is not the last argument keeps the whole line.
        assert_eq!(
            command_display_text("bash -c 'ls' extra"),
            "bash -c 'ls' extra"
        );
        assert_eq!(command_display_text("printf 'a\nb'"), "printf 'a␊b'");
    }

    #[test]
    fn live_command_rows_name_the_program_and_test_runs_flag_severity() {
        assert_eq!(
            command_program_name("cargo test -j 4").as_deref(),
            Some("cargo")
        );
        assert_eq!(
            command_program_name(r#"bash -lc "cargo test""#).as_deref(),
            Some("cargo")
        );
        assert!(command_runs_tests("cargo test"));
        assert!(command_runs_tests("npm run test"));
        assert!(command_runs_tests(r#"bash -lc "cargo test""#));
        assert!(command_runs_tests("vitest"));
        assert!(!command_runs_tests("cargo build"));
        assert!(!command_runs_tests("cargo testrunner-build"));
    }
}
