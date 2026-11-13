//! Git branch, status, worktree, and setup-script operations ported from KSG Code.
//!
//! Reference sources in KSG Code:
//! - `apps/server/src/vcs/GitVcsDriverCore.ts`
//! - `apps/server/src/worktreesDirectory.ts`
//! - `apps/server/src/git/remoteRefs.ts`
//! - `apps/server/src/orchestration-v2/ThreadLaunchService.ts`
//! - `apps/server/src/project/ProjectSetupScriptRunner.ts`
//! - `packages/shared/src/git.ts`
//! - `packages/shared/src/projectScripts.ts`

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[cfg_attr(not(test), allow(dead_code))]
pub const WORKTREE_BRANCH_PREFIX: &str = "t3";
#[cfg_attr(not(test), allow(dead_code))]
pub const GIT_LIST_BRANCHES_DEFAULT_LIMIT: usize = 100;
pub const GIT_LIST_BRANCHES_MAX_LIMIT: usize = 200;
pub const T3_PROJECT_FILE_NAME: &str = "t3.json";

pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const SHORT_TIMEOUT: Duration = Duration::from_secs(5);
const WORKTREE_TIMEOUT: Duration = Duration::from_secs(300);
const SETUP_DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_GIT_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const SETUP_OUTPUT_TAIL_BYTES: usize = 64 * 1024;
const AUTO_FEATURE_BRANCH_FALLBACK: &str = "feature/update";
const DEFAULT_BASE_BRANCH_CANDIDATES: [&str; 2] = ["main", "master"];

const NON_REPOSITORY_STDERR_PHRASES: [&str; 2] = [
    "not a git repository",
    "gitfile does not point to a valid repository",
];

#[derive(Clone, Debug, Default)]
pub struct GitContext {
    pub git_config_global: Option<PathBuf>,
    pub git_config_system: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VcsRefKind {
    All,
    Local,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_remote: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_ref: Option<String>,
    pub current: bool,
    pub is_default: bool,
    pub worktree_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VcsListBranchesInput {
    pub cwd: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub cursor: Option<usize>,
    #[serde(default)]
    pub include_matching_remote_refs: Option<bool>,
    #[serde(default)]
    pub ref_kind: Option<VcsRefKind>,
    #[serde(default)]
    pub refresh: Option<bool>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsListBranchesResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_at_repository_root: Option<bool>,
    pub refs: Vec<VcsRef>,
    pub is_repo: bool,
    pub has_primary_remote: bool,
    pub next_cursor: Option<usize>,
    pub total_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VcsStatusInput {
    pub cwd: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsWorkingTreeFile {
    pub path: String,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsWorkingTree {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_path: Option<String>,
    pub files: Vec<VcsWorkingTreeFile>,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsStatusResult {
    pub is_repo: bool,
    pub has_primary_remote: bool,
    pub is_default_ref: bool,
    pub ref_name: Option<String>,
    pub has_working_tree_changes: bool,
    pub working_tree: VcsWorkingTree,
    pub has_upstream: bool,
    pub ahead_count: usize,
    pub behind_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ahead_of_default_count: Option<usize>,
    pub pr: Option<serde_json::Value>,
}

impl VcsStatusResult {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_detached(&self) -> bool {
        self.is_repo && self.ref_name.is_none()
    }

    fn non_repo() -> Self {
        Self {
            is_repo: false,
            has_primary_remote: false,
            is_default_ref: false,
            ref_name: None,
            has_working_tree_changes: false,
            working_tree: VcsWorkingTree {
                root_path: None,
                files: Vec::new(),
                insertions: 0,
                deletions: 0,
            },
            has_upstream: false,
            ahead_count: 0,
            behind_count: 0,
            ahead_of_default_count: Some(0),
            pr: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VcsCreateWorktreeInput {
    pub cwd: String,
    pub ref_name: String,
    #[serde(default)]
    pub new_ref_name: Option<String>,
    #[serde(default)]
    pub base_ref_name: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub start_from_origin: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsWorktree {
    pub path: String,
    pub ref_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsCreateWorktreeResult {
    pub worktree: VcsWorktree,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VcsRemoveWorktreeInput {
    pub cwd: String,
    pub path: String,
    #[serde(default)]
    pub force: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VcsListWorktreesInput {
    pub cwd: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsWorktreeEntry {
    pub path: String,
    pub ref_name: Option<String>,
    pub head: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsListWorktreesResult {
    pub is_repo: bool,
    pub worktrees: Vec<VcsWorktreeEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VcsRunSetupInput {
    pub cwd: String,
    #[serde(default)]
    pub worktree_path: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VcsRunSetupResult {
    pub status: String,
    pub script_name: Option<String>,
    pub script_command: Option<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub output: String,
    pub duration_ms: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchNamingMode {
    Semantic,
    Static { prefix: String },
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeBaseRemote {
    pub remote_name: String,
    pub branch_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct T3ProjectFile {
    #[serde(default)]
    scripts: Vec<T3ProjectFileScript>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct T3ProjectFileScript {
    name: String,
    command: String,
    #[serde(default)]
    run_on_worktree_create: bool,
}

struct GitOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn git_available() -> bool {
    let mut cmd = Command::new("git");
    cmd.arg("--version");
    crate::process::output(&mut cmd, Duration::from_secs(2), 4096)
        .is_ok_and(|out| out.status.success())
}

pub(crate) fn git_error(
    operation: &str,
    cwd: &Path,
    stderr: &str,
    fallback_detail: &str,
) -> String {
    let trimmed = stderr.trim();
    let detail = if trimmed.is_empty() {
        fallback_detail
    } else {
        trimmed
    };
    format!(
        "Git command failed in {operation} ({}): {detail}",
        cwd.display()
    )
}

pub(crate) fn configure_git_command(cmd: &mut Command, cwd: &Path, ctx: &GitContext) {
    cmd.current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LANGUAGE", "C");
    // Suppress any configured credential helper without altering git's argv.
    let existing_count = std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    cmd.env("GIT_CONFIG_COUNT", (existing_count + 1).to_string())
        .env(
            format!("GIT_CONFIG_KEY_{existing_count}"),
            "credential.helper",
        )
        .env(format!("GIT_CONFIG_VALUE_{existing_count}"), "");
    if let Some(path) = &ctx.git_config_global {
        cmd.env("GIT_CONFIG_GLOBAL", path);
    }
    if let Some(path) = &ctx.git_config_system {
        cmd.env("GIT_CONFIG_SYSTEM", path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
}

fn run_git_raw(
    operation: &str,
    cwd: &Path,
    args: &[&str],
    timeout: Duration,
    ctx: &GitContext,
) -> Result<GitOutput, String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    configure_git_command(&mut cmd, cwd, ctx);
    let output =
        crate::process::output(&mut cmd, timeout, MAX_GIT_OUTPUT_BYTES).map_err(|err| {
            if err.kind() == std::io::ErrorKind::TimedOut {
                git_error(operation, cwd, "", "Git command timed out.")
            } else {
                git_error(
                    operation,
                    cwd,
                    &err.to_string(),
                    "Failed to spawn Git process.",
                )
            }
        })?;
    Ok(GitOutput {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn run_git_checked(
    operation: &str,
    cwd: &Path,
    args: &[&str],
    timeout: Duration,
    fallback_detail: &str,
    ctx: &GitContext,
) -> Result<GitOutput, String> {
    let result = run_git_raw(operation, cwd, args, timeout, ctx)?;
    if result.exit_code == Some(0) {
        Ok(result)
    } else {
        Err(git_error(operation, cwd, &result.stderr, fallback_detail))
    }
}

pub fn validate_existing_directory(raw: &str, label: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} must be an absolute path."));
    }
    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path."));
    }
    let meta =
        std::fs::metadata(path).map_err(|err| format!("{label} could not be accessed: {err}"))?;
    if !meta.is_dir() {
        return Err(format!("{label} is not a directory."));
    }
    Ok(path.to_path_buf())
}

pub(crate) fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_)) | None
                ) {
                    out.pop();
                }
            }
            Component::Normal(s) => out.push(s),
        }
    }
    out
}

fn has_only_normal_or_root_components(path: &Path) -> bool {
    path.components()
        .all(|c| !matches!(c, Component::CurDir | Component::ParentDir))
}

/// Match Git's symlink-following location: nearest real ancestor plus missing path components.
/// Ported from KSG Code's `worktreesDirectory.ts`.
pub(crate) fn real_location(target: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(target) {
        return real;
    }
    let mut missing = Vec::new();
    let mut cur = target;
    while let Some(name) = cur.file_name() {
        let Some(parent) = cur.parent() else {
            break;
        };
        missing.push(name.to_os_string());
        if let Ok(real_parent) = std::fs::canonicalize(parent) {
            let mut out = real_parent;
            for part in missing.into_iter().rev() {
                out.push(part);
            }
            return out;
        }
        cur = parent;
    }
    normalize_lexical(target)
}

pub fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none()
}

/// Ported from KSG Code's `makeIsManagedWorktreePath` in `apps/server/src/worktreesDirectory.ts`.
/// Accepts only normalized absolute paths matching `<worktrees_base_dir>/<repo>/<worktree>`.
pub fn is_managed_worktree_path(worktree_path: &Path, worktrees_base_dir: &Path) -> bool {
    if !worktree_path.is_absolute() || !has_only_normal_or_root_components(worktree_path) {
        return false;
    }
    if normalize_lexical(worktree_path) != worktree_path {
        return false;
    }
    if !worktrees_base_dir.is_absolute() {
        return false;
    }
    let real_base = real_location(worktrees_base_dir);
    if is_filesystem_root(&real_base) {
        return false;
    }
    let real_candidate = real_location(worktree_path);
    let Ok(relative) = real_candidate.strip_prefix(&real_base) else {
        return false;
    };
    let segments: Vec<_> = relative.components().collect();
    segments.len() == 2 && segments.iter().all(|c| matches!(c, Component::Normal(_)))
}

/// Ported from KSG Code's `GitVcsDriverCore.ts` (`createWorktree` path generation).
pub fn build_worktree_path(
    worktrees_base_dir: &Path,
    cwd: &Path,
    git_common_dir: &Path,
    target_branch: &str,
) -> Result<PathBuf, String> {
    if !worktrees_base_dir.is_absolute()
        || is_filesystem_root(&normalize_lexical(worktrees_base_dir))
    {
        return Err(format!(
            "The worktree location \"{}\" must be an absolute folder on this machine, not a drive root.",
            worktrees_base_dir.display()
        ));
    }
    let resolved_cwd = normalize_lexical(cwd);
    let repo_name = resolved_cwd
        .file_name()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Repository path must not be a filesystem root.".to_string())?;
    let identity = format!("{}\0{}", git_common_dir.to_string_lossy(), target_branch);
    let digest = Sha256::digest(identity.as_bytes());
    let hash_hex = hex_lower(&digest)[..16].to_string();
    let mut prefix = String::new();
    for ch in target_branch.replace('/', "-").chars() {
        if prefix.len() + ch.len_utf8() > 23 {
            break;
        }
        prefix.push(ch);
    }
    let directory_name = format!("{prefix}-{hash_hex}");
    Ok(worktrees_base_dir.join(repo_name).join(directory_name))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn is_non_repository_stderr(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    NON_REPOSITORY_STDERR_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
}

pub(crate) fn is_unborn_head_stderr(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("bad revision 'head'")
        || (lower.contains("unknown revision") && lower.contains("path not in the working tree"))
}

fn is_missing_worktree_stderr(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("is not a working tree") || lower.contains("cannot remove working tree")
}

fn is_ref_lock_failure(stderr: &str) -> bool {
    stderr.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with("error: cannot lock ref")
            || lower.starts_with("fatal: cannot lock ref")
            || (lower.starts_with("fatal: unable to create '") && lower.contains(".lock'"))
            || (lower.starts_with("error: fetching ref ")
                && (lower.contains("incorrect old value provided")
                    || lower.contains("reference already exists")))
    })
}

fn refuse_option_like_ref_name(
    operation: &str,
    cwd: &Path,
    ref_name: Option<&str>,
) -> Result<(), String> {
    if let Some(name) = ref_name
        && name != "-"
        && name.starts_with('-')
    {
        return Err(git_error(
            operation,
            cwd,
            "",
            "Ref names cannot start with '-'.",
        ));
    }
    Ok(())
}

fn read_git_common_dir(cwd: &Path, ctx: &GitContext) -> Result<Option<PathBuf>, String> {
    let out = run_git_raw(
        "GitVcsDriver.resolveRepositoryPaths.commonDir",
        cwd,
        &["rev-parse", "--git-common-dir"],
        SHORT_TIMEOUT,
        ctx,
    )?;
    if out.exit_code != Some(0) {
        if is_non_repository_stderr(&out.stderr) {
            return Ok(None);
        }
        return Err(git_error(
            "GitVcsDriver.resolveRepositoryPaths.commonDir",
            cwd,
            &out.stderr,
            "Failed to resolve the Git common directory.",
        ));
    }
    let raw = out.stdout.trim();
    let physical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let resolved = if Path::new(raw).is_absolute() {
        normalize_lexical(Path::new(raw))
    } else {
        normalize_lexical(&physical_cwd.join(raw))
    };
    Ok(Some(std::fs::canonicalize(&resolved).unwrap_or(resolved)))
}

fn read_worktree_root(cwd: &Path, ctx: &GitContext) -> Result<Option<PathBuf>, String> {
    let out = run_git_raw(
        "GitVcsDriver.resolveRepositoryPaths.worktreeRoot",
        cwd,
        &["rev-parse", "--show-toplevel"],
        SHORT_TIMEOUT,
        ctx,
    )?;
    let trimmed = out.stdout.strip_suffix('\n').unwrap_or(&out.stdout);
    #[cfg(windows)]
    let trimmed = trimmed.strip_suffix('\r').unwrap_or(trimmed);
    if out.exit_code == Some(0) && !trimmed.is_empty() {
        let p = Path::new(trimmed);
        let abs = if p.is_absolute() {
            normalize_lexical(p)
        } else {
            normalize_lexical(&cwd.join(p))
        };
        Ok(Some(abs))
    } else {
        Ok(None)
    }
}

pub(crate) fn branch_name_from_head_ref(value: &str) -> Option<String> {
    let trimmed = value.trim();
    trimmed
        .strip_prefix("refs/heads/")
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn fetch_cwd_for_common_dir(git_common_dir: &Path) -> &Path {
    if git_common_dir.file_name().is_some_and(|n| n == ".git") {
        git_common_dir.parent().unwrap_or(git_common_dir)
    } else {
        git_common_dir
    }
}

fn parse_remote_names_in_git_order(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_remote_names(stdout: &str) -> Vec<String> {
    let mut names = parse_remote_names_in_git_order(stdout);
    names.sort_by(|a, b| b.len().cmp(&a.len()));
    names
}

fn parse_remote_ref_with_remote_names(
    ref_name: &str,
    remote_names: &[String],
) -> Option<(String, String, String)> {
    let trimmed = ref_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    for remote in remote_names {
        let prefix = format!("{remote}/");
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            let branch = rest.trim();
            if !branch.is_empty() {
                return Some((trimmed.to_owned(), remote.clone(), branch.to_owned()));
            }
            return None;
        }
    }
    None
}

pub fn derive_local_branch_name_from_remote_ref(branch_name: &str) -> &str {
    match branch_name.find('/') {
        Some(idx) if idx > 0 && idx + 1 < branch_name.len() => &branch_name[idx + 1..],
        _ => branch_name,
    }
}

fn execute_worktree_list(
    operation: &str,
    cwd: &Path,
    git_dir: Option<&Path>,
    ctx: &GitContext,
) -> Result<String, String> {
    let git_dir_str = git_dir.map(|p| p.to_string_lossy().into_owned());
    let mut args_z: Vec<&str> = Vec::new();
    if let Some(gd) = git_dir_str.as_deref() {
        args_z.extend(["--git-dir", gd]);
    }
    args_z.extend(["worktree", "list", "--porcelain", "-z"]);
    let res = run_git_raw(operation, cwd, &args_z, DEFAULT_TIMEOUT, ctx)?;
    if res.exit_code == Some(0) {
        return Ok(res.stdout);
    }
    let mut args_line: Vec<&str> = Vec::new();
    if let Some(gd) = git_dir_str.as_deref() {
        args_line.extend(["--git-dir", gd]);
    }
    args_line.extend(["worktree", "list", "--porcelain"]);
    let line_res = run_git_checked(
        operation,
        cwd,
        &args_line,
        DEFAULT_TIMEOUT,
        "git worktree list failed.",
        ctx,
    )?;
    Ok(line_res.stdout.replace('\n', "\0"))
}

fn parse_worktree_entries(stdout: &str) -> Vec<VcsWorktreeEntry> {
    let mut entries = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_head: Option<String> = None;
    let mut current_branch: Option<String> = None;
    let mut current_detached = false;
    let mut current_bare = false;
    let mut current_locked = false;
    let mut current_prunable = false;

    let flush = |entries: &mut Vec<VcsWorktreeEntry>,
                 current_path: &mut Option<String>,
                 current_head: &mut Option<String>,
                 current_branch: &mut Option<String>,
                 current_detached: &mut bool,
                 current_bare: &mut bool,
                 current_locked: &mut bool,
                 current_prunable: &mut bool| {
        if let Some(path) = current_path.take() {
            entries.push(VcsWorktreeEntry {
                path,
                ref_name: current_branch.take(),
                head: current_head.take(),
                detached: *current_detached,
                bare: *current_bare,
                locked: *current_locked,
                prunable: *current_prunable,
            });
        }
        *current_head = None;
        *current_branch = None;
        *current_detached = false;
        *current_bare = false;
        *current_locked = false;
        *current_prunable = false;
    };

    for field in stdout.split('\0') {
        if field.is_empty() {
            flush(
                &mut entries,
                &mut current_path,
                &mut current_head,
                &mut current_branch,
                &mut current_detached,
                &mut current_bare,
                &mut current_locked,
                &mut current_prunable,
            );
        } else if let Some(path) = field.strip_prefix("worktree ") {
            current_path = Some(path.to_owned());
        } else if let Some(head) = field.strip_prefix("HEAD ") {
            current_head = Some(head.to_owned());
        } else if let Some(branch) = field.strip_prefix("branch refs/heads/") {
            current_branch = Some(branch.to_owned());
        } else if field == "detached" {
            current_detached = true;
        } else if field == "bare" {
            current_bare = true;
        } else if field == "locked" || field.starts_with("locked ") {
            current_locked = true;
        } else if field == "prunable" || field.starts_with("prunable ") {
            current_prunable = true;
        }
    }
    flush(
        &mut entries,
        &mut current_path,
        &mut current_head,
        &mut current_branch,
        &mut current_detached,
        &mut current_bare,
        &mut current_locked,
        &mut current_prunable,
    );
    entries
}

fn parse_worktree_branch_paths(stdout: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entry in parse_worktree_entries(stdout) {
        if !entry.prunable
            && let Some(branch) = entry.ref_name
        {
            map.insert(branch, entry.path);
        }
    }
    map
}

fn parse_default_branch_from_remote_head_ref(value: &str, remote_name: &str) -> Option<String> {
    let trimmed = value.trim();
    let prefix = format!("refs/remotes/{remote_name}/");
    trimmed
        .strip_prefix(&prefix)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Ported from KSG Code's `listRefs` in `GitVcsDriverCore.ts`.
pub fn list_branches(input: &VcsListBranchesInput) -> Result<VcsListBranchesResult, String> {
    list_branches_with(input, &GitContext::default())
}

pub fn list_branches_with(
    input: &VcsListBranchesInput,
    ctx: &GitContext,
) -> Result<VcsListBranchesResult, String> {
    let cwd = validate_existing_directory(&input.cwd, "cwd")?;
    if let Some(limit) = input.limit
        && (limit == 0 || limit > GIT_LIST_BRANCHES_MAX_LIMIT)
    {
        return Err(format!(
            "limit must be between 1 and {GIT_LIST_BRANCHES_MAX_LIMIT}."
        ));
    }

    let Some(git_common_dir) = read_git_common_dir(&cwd, ctx)? else {
        return Ok(VcsListBranchesResult {
            is_at_repository_root: None,
            refs: Vec::new(),
            is_repo: false,
            has_primary_remote: false,
            next_cursor: None,
            total_count: 0,
        });
    };

    let worktree_root = read_worktree_root(&cwd, ctx)?;
    let current_branch_out = run_git_raw(
        "GitVcsDriver.resolveRepositoryPaths.currentBranch",
        &cwd,
        &["symbolic-ref", "--quiet", "HEAD"],
        SHORT_TIMEOUT,
        ctx,
    )?;
    let current_branch = if current_branch_out.exit_code == Some(0) {
        branch_name_from_head_ref(&current_branch_out.stdout)
    } else {
        None
    };
    let is_at_repository_root = worktree_root.as_ref().and_then(|root| {
        std::fs::canonicalize(&cwd).ok().map(|phys| {
            let norm_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
            phys == norm_root
        })
    });

    let fetch_cwd = fetch_cwd_for_common_dir(&git_common_dir);
    let git_dir_str = git_common_dir.to_string_lossy().into_owned();

    let refs_out = run_git_checked(
        "GitVcsDriver.listRefs.snapshotRefs",
        fetch_cwd,
        &[
            "--git-dir",
            &git_dir_str,
            "for-each-ref",
            "--format=%(refname)%09%(committerdate:unix)%09%(symref)%09%(upstream)",
            "refs/heads",
            "refs/remotes",
        ],
        DEFAULT_TIMEOUT,
        "Git ref snapshot enumeration failed.",
        ctx,
    )?;

    let default_ref_out = run_git_raw(
        "GitVcsDriver.listRefs.defaultRef",
        fetch_cwd,
        &[
            "--git-dir",
            &git_dir_str,
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
        ],
        SHORT_TIMEOUT,
        ctx,
    )?;
    let default_branch = if default_ref_out.exit_code == Some(0) {
        parse_default_branch_from_remote_head_ref(&default_ref_out.stdout, "origin")
    } else {
        None
    };

    let worktree_stdout = execute_worktree_list(
        "GitVcsDriver.listRefs.worktreeList",
        fetch_cwd,
        Some(&git_common_dir),
        ctx,
    )
    .unwrap_or_default();
    let normalized_common_dir = normalize_lexical(&git_common_dir);
    let mut worktree_map = HashMap::new();
    for (branch_name, wt_path) in parse_worktree_branch_paths(&worktree_stdout) {
        let normalized = normalize_lexical(Path::new(&wt_path));
        if normalized != normalized_common_dir && normalized.exists() {
            worktree_map.insert(branch_name, normalized.to_string_lossy().into_owned());
        }
    }

    let remote_names_out = run_git_raw(
        "GitVcsDriver.listRefs.remoteNames",
        fetch_cwd,
        &["--git-dir", &git_dir_str, "remote"],
        SHORT_TIMEOUT,
        ctx,
    )?;
    let remote_names = if remote_names_out.exit_code == Some(0) {
        parse_remote_names(&remote_names_out.stdout)
    } else {
        Vec::new()
    };
    let has_primary_remote = !remote_names.is_empty();

    let mut local_branches: Vec<(VcsRef, i64)> = Vec::new();
    let mut remote_branches: Vec<(VcsRef, i64)> = Vec::new();

    for line in refs_out.stdout.split('\n') {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let full_ref_name = parts.next().unwrap_or("");
        let last_commit_raw = parts.next().unwrap_or("0");
        let symbolic_target = parts.next().unwrap_or("");
        let upstream_ref = parts.next().unwrap_or("");
        if full_ref_name.is_empty() || !symbolic_target.is_empty() {
            continue;
        }
        let last_commit = last_commit_raw.parse::<i64>().unwrap_or(0);

        if let Some(name) = full_ref_name.strip_prefix("refs/heads/") {
            local_branches.push((
                VcsRef {
                    name: name.to_owned(),
                    is_remote: Some(false),
                    remote_name: None,
                    upstream_ref: (!upstream_ref.is_empty()).then(|| upstream_ref.to_owned()),
                    current: false,
                    is_default: default_branch.as_deref() == Some(name),
                    worktree_path: worktree_map.get(name).cloned(),
                },
                last_commit,
            ));
            continue;
        }

        if let Some(name) = full_ref_name.strip_prefix("refs/remotes/") {
            let parsed_remote = parse_remote_ref_with_remote_names(name, &remote_names);
            let remote_name = parsed_remote.as_ref().map(|(_, r, _)| r.clone());
            let is_default = default_branch.as_deref().is_some_and(|def| {
                parsed_remote
                    .as_ref()
                    .is_some_and(|(_, r, b)| r == "origin" && b == def)
            });
            remote_branches.push((
                VcsRef {
                    name: name.to_owned(),
                    is_remote: Some(true),
                    remote_name,
                    upstream_ref: None,
                    current: false,
                    is_default,
                    worktree_path: None,
                },
                last_commit,
            ));
        }
    }

    let cmp_recency_then_name =
        |a: &(VcsRef, i64), b: &(VcsRef, i64)| b.1.cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name));
    local_branches.sort_by(cmp_recency_then_name);
    remote_branches.sort_by(cmp_recency_then_name);

    let worktree_root_str = worktree_root
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned());
    let has_current_worktree_branch = worktree_root_str.as_deref().is_some_and(|root| {
        local_branches
            .iter()
            .any(|(r, _)| r.worktree_path.as_deref() == Some(root))
    });

    let mut local_refs: Vec<VcsRef> = local_branches
        .into_iter()
        .map(|(mut r, _)| {
            if r.worktree_path.is_none() && current_branch.as_deref() == Some(r.name.as_str()) {
                r.worktree_path = worktree_root_str.clone();
            }
            r.current = if has_current_worktree_branch {
                r.worktree_path.as_deref() == worktree_root_str.as_deref()
            } else {
                current_branch.as_deref() == Some(r.name.as_str())
            };
            r
        })
        .collect();

    let remote_refs: Vec<VcsRef> = remote_branches.into_iter().map(|(r, _)| r).collect();
    let local_names: HashSet<String> = local_refs.iter().map(|r| r.name.clone()).collect();

    let mut combined = Vec::with_capacity(local_refs.len() + remote_refs.len());
    combined.append(&mut local_refs);
    if input.include_matching_remote_refs == Some(true) {
        combined.extend(remote_refs);
    } else {
        for r in remote_refs {
            if r.remote_name.as_deref() == Some("origin") {
                let local_name = derive_local_branch_name_from_remote_ref(&r.name);
                if !local_name.is_empty() && local_names.contains(local_name) {
                    continue;
                }
            }
            combined.push(r);
        }
    }

    combined.sort_by_key(|r| {
        if r.current {
            0
        } else if r.is_default {
            1
        } else {
            2
        }
    });

    let by_kind: Vec<VcsRef> = match input.ref_kind.unwrap_or(VcsRefKind::All) {
        VcsRefKind::Local => combined
            .into_iter()
            .filter(|r| r.is_remote != Some(true))
            .collect(),
        VcsRefKind::Remote => combined
            .into_iter()
            .filter(|r| r.is_remote == Some(true))
            .collect(),
        VcsRefKind::All => combined,
    };

    let filtered: Vec<VcsRef> = match input
        .query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(q) => {
            let needle = q.to_ascii_lowercase();
            by_kind
                .into_iter()
                .filter(|r| r.name.to_ascii_lowercase().contains(&needle))
                .collect()
        }
        None => by_kind,
    };

    let total_count = filtered.len();
    let cursor = input.cursor.unwrap_or(0).min(total_count);
    let limit = input.limit.unwrap_or(GIT_LIST_BRANCHES_DEFAULT_LIMIT);
    let end = (cursor + limit).min(total_count);
    let refs = filtered[cursor..end].to_vec();
    let next_cursor = (end < total_count).then_some(end);

    Ok(VcsListBranchesResult {
        is_at_repository_root,
        refs,
        is_repo: true,
        has_primary_remote,
        next_cursor,
        total_count,
    })
}

fn read_config_value(cwd: &Path, key: &str, ctx: &GitContext) -> Option<String> {
    let out = run_git_raw(
        "GitVcsDriver.readConfigValue",
        cwd,
        &["config", "--get", key],
        SHORT_TIMEOUT,
        ctx,
    )
    .ok()?;
    if out.exit_code != Some(0) {
        return None;
    }
    let trimmed = out.stdout.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn write_config_value(cwd: &Path, key: &str, value: &str, ctx: &GitContext) -> Result<(), String> {
    run_git_checked(
        "GitVcsDriver.writeConfigValue",
        cwd,
        &["config", key, value],
        SHORT_TIMEOUT,
        "git config failed",
        ctx,
    )?;
    Ok(())
}

pub(crate) fn branch_exists(cwd: &Path, ref_name: &str, ctx: &GitContext) -> bool {
    let full = format!("refs/heads/{ref_name}");
    run_git_raw(
        "GitVcsDriver.branchExists",
        cwd,
        &["show-ref", "--verify", "--quiet", &full],
        SHORT_TIMEOUT,
        ctx,
    )
    .is_ok_and(|out| out.exit_code == Some(0))
}

pub(crate) fn remote_branch_exists(
    cwd: &Path,
    remote_name: &str,
    ref_name: &str,
    ctx: &GitContext,
) -> bool {
    let full = format!("refs/remotes/{remote_name}/{ref_name}");
    run_git_raw(
        "GitVcsDriver.remoteBranchExists",
        cwd,
        &["show-ref", "--verify", "--quiet", &full],
        SHORT_TIMEOUT,
        ctx,
    )
    .is_ok_and(|out| out.exit_code == Some(0))
}

fn resolve_primary_remote_name(cwd: &Path, ctx: &GitContext) -> Option<String> {
    let origin = run_git_raw(
        "GitVcsDriver.remoteExists",
        cwd,
        &["remote", "get-url", "--", "origin"],
        SHORT_TIMEOUT,
        ctx,
    )
    .ok()?;
    if origin.exit_code == Some(0) {
        return Some("origin".into());
    }
    let list = run_git_raw(
        "GitVcsDriver.listRemoteNames",
        cwd,
        &["remote"],
        SHORT_TIMEOUT,
        ctx,
    )
    .ok()?;
    if list.exit_code != Some(0) {
        return None;
    }
    parse_remote_names_in_git_order(&list.stdout)
        .into_iter()
        .next()
}

fn resolve_default_branch_name(cwd: &Path, remote_name: &str, ctx: &GitContext) -> Option<String> {
    let ref_path = format!("refs/remotes/{remote_name}/HEAD");
    let out = run_git_raw(
        "GitVcsDriver.resolveDefaultBranchName",
        cwd,
        &["symbolic-ref", &ref_path],
        SHORT_TIMEOUT,
        ctx,
    )
    .ok()?;
    if out.exit_code != Some(0) {
        return None;
    }
    parse_default_branch_from_remote_head_ref(&out.stdout, remote_name)
}

pub(crate) fn resolve_comparison_base_ref(
    cwd: &Path,
    base_ref: &str,
    ctx: &GitContext,
) -> Option<(String, String)> {
    let comparison_ref =
        if base_ref.starts_with("refs/heads/") || base_ref.starts_with("refs/remotes/") {
            base_ref.to_owned()
        } else if base_ref.contains('/') && !base_ref.starts_with("refs/") {
            format!("refs/remotes/{base_ref}")
        } else {
            return None;
        };
    let ok = run_git_raw(
        "GitVcsDriver.resolveComparisonBaseRef",
        cwd,
        &["show-ref", "--verify", "--quiet", &comparison_ref],
        SHORT_TIMEOUT,
        ctx,
    )
    .is_ok_and(|out| out.exit_code == Some(0));
    ok.then(|| (base_ref.to_owned(), comparison_ref))
}

fn resolve_tracked_remote_base_ref(
    cwd: &Path,
    ref_name: &str,
    ctx: &GitContext,
) -> Option<(String, String)> {
    let query = format!("{ref_name}@{{upstream}}");
    let out = run_git_raw(
        "GitVcsDriver.resolveTrackedRemoteBaseRef",
        cwd,
        &["rev-parse", "--symbolic-full-name", &query],
        SHORT_TIMEOUT,
        ctx,
    )
    .ok()?;
    if out.exit_code != Some(0) {
        return None;
    }
    let comparison = out.stdout.trim();
    comparison
        .strip_prefix("refs/remotes/")
        .map(|short| (short.to_owned(), comparison.to_owned()))
}

pub(crate) fn resolve_base_branch_for_no_upstream(
    cwd: &Path,
    ref_name: &str,
    known_origin_default: Option<&str>,
    allow_remote_of_current: bool,
    ctx: &GitContext,
) -> Option<(String, String)> {
    if let Some(configured_ref) =
        read_config_value(cwd, &format!("branch.{ref_name}.t3-merge-base-ref"), ctx)
        && (configured_ref.starts_with("refs/heads/")
            || configured_ref.starts_with("refs/remotes/"))
        && let Some((base_ref, comparison_ref)) =
            resolve_comparison_base_ref(cwd, &configured_ref, ctx)
    {
        let short_ref = base_ref
            .strip_prefix("refs/remotes/")
            .unwrap_or(&base_ref)
            .to_owned();
        let final_base = if branch_exists(cwd, &short_ref, ctx) {
            base_ref
        } else {
            short_ref
        };
        return Some((final_base, comparison_ref));
    }

    let configured_base = read_config_value(cwd, &format!("branch.{ref_name}.t3-merge-base"), ctx)
        .or_else(|| read_config_value(cwd, &format!("branch.{ref_name}.gh-merge-base"), ctx))
        .unwrap_or_default();

    if let Some((base_ref, comparison_ref)) =
        resolve_comparison_base_ref(cwd, &configured_base, ctx)
    {
        if configured_base.contains('/')
            && !configured_base.starts_with("refs/")
            && branch_exists(cwd, &configured_base, ctx)
        {
            let full = format!("refs/heads/{configured_base}");
            return Some((full.clone(), full));
        }
        return Some((base_ref, comparison_ref));
    }

    let primary_remote = resolve_primary_remote_name(cwd, ctx);
    let mut candidates: Vec<String> = Vec::new();
    if !configured_base.is_empty() {
        candidates.push(configured_base);
    }
    if let Some(remote) = primary_remote.as_deref() {
        let remote_default = if remote == "origin" {
            known_origin_default
                .map(str::to_owned)
                .or_else(|| resolve_default_branch_name(cwd, remote, ctx))
        } else {
            resolve_default_branch_name(cwd, remote, ctx)
        };
        if let Some(def) = remote_default {
            candidates.push(def);
        }
    }
    for cand in DEFAULT_BASE_BRANCH_CANDIDATES {
        candidates.push(cand.to_owned());
    }

    for candidate in candidates {
        if candidate.is_empty() {
            continue;
        }
        let remote_prefix = primary_remote
            .as_deref()
            .filter(|r| *r != "origin")
            .map(|r| format!("{r}/"));
        let normalized = if let Some(rest) = candidate.strip_prefix("origin/") {
            rest
        } else if let Some(prefix) = remote_prefix.as_deref()
            && let Some(rest) = candidate.strip_prefix(prefix)
        {
            rest
        } else {
            candidate.as_str()
        };
        if normalized.is_empty() {
            continue;
        }
        if normalized == ref_name {
            if allow_remote_of_current {
                if let Some(tracked) = resolve_tracked_remote_base_ref(cwd, ref_name, ctx) {
                    return Some(tracked);
                }
                if let Some(remote) = primary_remote.as_deref()
                    && remote_branch_exists(cwd, remote, normalized, ctx)
                {
                    return Some((
                        format!("{remote}/{normalized}"),
                        format!("refs/remotes/{remote}/{normalized}"),
                    ));
                }
            }
            continue;
        }
        if let Some(remote) = primary_remote.as_deref()
            && remote_branch_exists(cwd, remote, normalized, ctx)
        {
            return Some((
                format!("{remote}/{normalized}"),
                format!("refs/remotes/{remote}/{normalized}"),
            ));
        }
        if branch_exists(cwd, normalized, ctx) {
            return Some((normalized.to_owned(), format!("refs/heads/{normalized}")));
        }
    }
    None
}

fn compute_ahead_count_against_base(
    cwd: &Path,
    ref_name: &str,
    known_origin_default: Option<&str>,
    ctx: &GitContext,
) -> usize {
    let Some((_, comparison_ref)) =
        resolve_base_branch_for_no_upstream(cwd, ref_name, known_origin_default, false, ctx)
    else {
        return 0;
    };
    let range = format!("{comparison_ref}..HEAD");
    let Ok(out) = run_git_raw(
        "GitVcsDriver.computeAheadCountAgainstBase",
        cwd,
        &["rev-list", "--count", "--end-of-options", &range],
        SHORT_TIMEOUT,
        ctx,
    ) else {
        return 0;
    };
    if out.exit_code != Some(0) {
        return 0;
    }
    out.stdout.trim().parse::<usize>().unwrap_or(0)
}

fn parse_branch_ab(value: &str) -> (usize, usize) {
    let mut parts = value.split_whitespace();
    let ahead = parts
        .next()
        .and_then(|s| s.strip_prefix('+'))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let behind = parts
        .next()
        .and_then(|s| s.strip_prefix('-'))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    (ahead, behind)
}

fn parse_porcelain_path(record: &str) -> Option<String> {
    let fields_before_path = match record.as_bytes().first().copied() {
        Some(b'1') => 8,
        Some(b'2') => 9,
        Some(b'u') => 10,
        Some(b'?') | Some(b'!') => 1,
        _ => return None,
    };
    let mut parts = record.splitn(fields_before_path + 1, ' ');
    for _ in 0..fields_before_path {
        parts.next()?;
    }
    let path = parts.next()?;
    (!path.is_empty()).then(|| path.to_owned())
}

struct NumstatEntry {
    path: String,
    additions: usize,
    deletions: usize,
}

fn parse_review_numstat(stdout: &str) -> Vec<NumstatEntry> {
    let fields: Vec<&str> = stdout.split('\0').collect();
    let mut files = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        let mut tab_parts = field.splitn(3, '\t');
        if let (Some(add_raw), Some(del_raw), Some(rest)) =
            (tab_parts.next(), tab_parts.next(), tab_parts.next())
            && (add_raw == "-" || add_raw.chars().all(|c| c.is_ascii_digit()))
            && (del_raw == "-" || del_raw.chars().all(|c| c.is_ascii_digit()))
            && !add_raw.is_empty()
            && !del_raw.is_empty()
        {
            let path = if rest.is_empty() {
                index += 1;
                let _prev = fields.get(index).copied().unwrap_or("");
                index += 1;
                fields.get(index).copied().unwrap_or("").to_owned()
            } else {
                rest.to_owned()
            };
            files.push(NumstatEntry {
                path,
                additions: if add_raw == "-" {
                    0
                } else {
                    add_raw.parse().unwrap_or(0)
                },
                deletions: if del_raw == "-" {
                    0
                } else {
                    del_raw.parse().unwrap_or(0)
                },
            });
        }
        index += 1;
    }
    files
}

/// Ported from KSG Code's `readStatusDetailsLocal` / `status` in `GitVcsDriverCore.ts`.
pub fn status(input: &VcsStatusInput) -> Result<VcsStatusResult, String> {
    status_with(input, &GitContext::default())
}

pub fn status_with(input: &VcsStatusInput, ctx: &GitContext) -> Result<VcsStatusResult, String> {
    let cwd = validate_existing_directory(&input.cwd, "cwd")?;
    let index_out = run_git_raw(
        "GitVcsDriver.statusDetails.indexPath",
        &cwd,
        &["rev-parse", "--git-path", "index"],
        SHORT_TIMEOUT,
        ctx,
    )?;
    if index_out.exit_code != Some(0) {
        if is_non_repository_stderr(&index_out.stderr) {
            return Ok(VcsStatusResult::non_repo());
        }
        return Err(git_error(
            "GitVcsDriver.statusDetails.indexPath",
            &cwd,
            &index_out.stderr,
            "Git index lookup failed.",
        ));
    }
    let index_rel = index_out.stdout.trim();
    if !index_rel.is_empty() {
        let index_path = cwd.join(index_rel);
        let lock_path = PathBuf::from(format!("{}.lock", index_path.display()));
        if lock_path.exists() {
            return Err(git_error(
                "GitVcsDriver.statusDetails.indexPath",
                &cwd,
                "",
                "Git index is locked. Status will resume when the index lock is removed.",
            ));
        }
    }

    let status_out = run_git_raw(
        "GitVcsDriver.statusDetails.status",
        &cwd,
        &["status", "--porcelain=2", "--branch", "-z"],
        DEFAULT_TIMEOUT,
        ctx,
    )?;
    if status_out.exit_code != Some(0) {
        if is_non_repository_stderr(&status_out.stderr) {
            return Ok(VcsStatusResult::non_repo());
        }
        return Err(git_error(
            "GitVcsDriver.statusDetails.status",
            &cwd,
            &status_out.stderr,
            "Git status failed.",
        ));
    }

    let worktree_root = read_worktree_root(&cwd, ctx)?;
    let common_dir = read_git_common_dir(&cwd, ctx)?;
    let (default_branch, has_primary_remote) = if let Some(common) = common_dir.as_deref() {
        let fetch_cwd = fetch_cwd_for_common_dir(common);
        let gd = common.to_string_lossy().into_owned();
        let def_out = run_git_raw(
            "GitVcsDriver.statusDetails.defaultBranch",
            fetch_cwd,
            &["--git-dir", &gd, "symbolic-ref", "refs/remotes/origin/HEAD"],
            SHORT_TIMEOUT,
            ctx,
        )?;
        let def = if def_out.exit_code == Some(0) {
            parse_default_branch_from_remote_head_ref(&def_out.stdout, "origin")
        } else {
            None
        };
        let rem_out = run_git_raw(
            "GitVcsDriver.statusDetails.primaryRemoteExists",
            fetch_cwd,
            &["--git-dir", &gd, "remote"],
            SHORT_TIMEOUT,
            ctx,
        )?;
        let has_rem = rem_out.exit_code == Some(0) && !rem_out.stdout.trim().is_empty();
        (def, has_rem)
    } else {
        (None, false)
    };

    let numstat_out = run_git_raw(
        "GitVcsDriver.statusDetails.numstat",
        &cwd,
        &["diff-index", "-M", "--numstat", "-z", "HEAD", "--"],
        DEFAULT_TIMEOUT,
        ctx,
    )?;
    let numstat_entries = if numstat_out.exit_code == Some(0) {
        parse_review_numstat(&numstat_out.stdout)
    } else if is_unborn_head_stderr(&numstat_out.stderr) {
        let unstaged = run_git_checked(
            "GitVcsDriver.statusDetails.numstat.unborn",
            &cwd,
            &["diff-files", "--numstat", "-z"],
            DEFAULT_TIMEOUT,
            "git diff-files failed.",
            ctx,
        )?;
        let staged = run_git_checked(
            "GitVcsDriver.statusDetails.numstat.unborn.staged",
            &cwd,
            &["diff", "--cached", "--numstat", "-z"],
            DEFAULT_TIMEOUT,
            "git diff --cached failed.",
            ctx,
        )?;
        let mut combined = parse_review_numstat(&staged.stdout);
        combined.extend(parse_review_numstat(&unstaged.stdout));
        combined
    } else {
        return Err(git_error(
            "GitVcsDriver.statusDetails.numstat",
            &cwd,
            &numstat_out.stderr,
            "git diff-index HEAD --numstat failed.",
        ));
    };

    let mut ref_name: Option<String> = None;
    let mut upstream_ref: Option<String> = None;
    let mut ahead_count: usize = 0;
    let mut behind_count: usize = 0;
    let mut has_working_tree_changes = false;
    let mut changed_files_without_numstat: HashSet<String> = HashSet::new();

    let records: Vec<&str> = status_out.stdout.split('\0').collect();
    let mut index = 0;
    while index < records.len() {
        let line = records[index];
        if let Some(value) = line.strip_prefix("# branch.head ") {
            let trimmed = value.trim();
            ref_name = if trimmed.starts_with('(') {
                None
            } else {
                Some(trimmed.to_owned())
            };
        } else if let Some(value) = line.strip_prefix("# branch.upstream ") {
            let trimmed = value.trim();
            upstream_ref = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        } else if let Some(value) = line.strip_prefix("# branch.ab ") {
            let (ahead, behind) = parse_branch_ab(value.trim());
            ahead_count = ahead;
            behind_count = behind;
        } else if !line.is_empty() && !line.starts_with('#') {
            has_working_tree_changes = true;
            if let Some(path_value) = parse_porcelain_path(line) {
                changed_files_without_numstat.insert(path_value);
            }
            if line.starts_with("2 ") {
                index += 1;
                if let Some(orig) = records.get(index).copied().filter(|s| !s.is_empty()) {
                    changed_files_without_numstat.insert(orig.to_owned());
                }
            }
        }
        index += 1;
    }

    let is_default_ref = ref_name.as_deref().is_some_and(|r| {
        default_branch.as_deref() == Some(r)
            || (default_branch.is_none() && (r == "main" || r == "master"))
    });
    let needs_fallback_ahead = upstream_ref.is_none() && ref_name.is_some();
    let needs_ahead_of_default = ref_name.is_some() && !is_default_ref;
    let base_ahead_count = if let Some(ref current_branch) = ref_name
        && (needs_fallback_ahead || needs_ahead_of_default)
    {
        compute_ahead_count_against_base(&cwd, current_branch, default_branch.as_deref(), ctx)
    } else {
        0
    };
    if needs_fallback_ahead {
        ahead_count = base_ahead_count;
        behind_count = 0;
    }
    let ahead_of_default_count = if needs_ahead_of_default {
        base_ahead_count
    } else {
        0
    };

    let mut file_stats: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for entry in numstat_entries {
        let stats = file_stats.entry(entry.path).or_insert((0, 0));
        stats.0 += entry.additions;
        stats.1 += entry.deletions;
    }

    let mut insertions = 0;
    let mut deletions = 0;
    for (ins, del) in file_stats.values() {
        insertions += ins;
        deletions += del;
    }
    for file_path in changed_files_without_numstat {
        file_stats.entry(file_path).or_insert((0, 0));
    }

    let files = file_stats
        .into_iter()
        .map(|(path, (ins, del))| VcsWorkingTreeFile {
            path,
            insertions: ins,
            deletions: del,
        })
        .collect();

    Ok(VcsStatusResult {
        is_repo: true,
        has_primary_remote,
        is_default_ref,
        ref_name,
        has_working_tree_changes,
        working_tree: VcsWorkingTree {
            root_path: worktree_root.map(|p| p.to_string_lossy().into_owned()),
            files,
            insertions,
            deletions,
        },
        has_upstream: upstream_ref.is_some(),
        ahead_count,
        behind_count,
        ahead_of_default_count: Some(ahead_of_default_count),
        pr: None,
    })
}

/// Ported from KSG Code's `resolveWorktreeBaseRemote` in `GitVcsDriverCore.ts`.
pub fn resolve_worktree_base_remote_with(
    cwd: &Path,
    ref_name: &str,
    ctx: &GitContext,
) -> Result<Option<WorktreeBaseRemote>, String> {
    let remotes_out = run_git_checked(
        "GitVcsDriver.listRemoteNames",
        cwd,
        &["remote"],
        SHORT_TIMEOUT,
        "git remote failed.",
        ctx,
    )?;
    let remote_names = parse_remote_names(&remotes_out.stdout);
    let full_ref = format!("refs/heads/{ref_name}");
    let local_out = run_git_checked(
        "GitVcsDriver.resolveWorktreeBaseRemote",
        cwd,
        &[
            "for-each-ref",
            "--format=%(refname)\t%(upstream:remotename)\t%(upstream:remoteref)",
            &full_ref,
        ],
        SHORT_TIMEOUT,
        "git for-each-ref failed.",
        ctx,
    )?;
    let local_branch = local_out
        .stdout
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|cols| cols.first().copied() == Some(full_ref.as_str()));

    let target = if let Some(cols) = local_branch {
        let remote_name = cols.get(1).copied().unwrap_or("");
        let merge = cols.get(2).copied().unwrap_or("");
        if remote_names.iter().any(|r| r == remote_name)
            && let Some(branch) = merge.strip_prefix("refs/heads/")
        {
            Some(WorktreeBaseRemote {
                remote_name: remote_name.to_owned(),
                branch_name: branch.to_owned(),
            })
        } else {
            None
        }
    } else {
        parse_remote_ref_with_remote_names(ref_name, &remote_names).map(|(_, r, b)| {
            WorktreeBaseRemote {
                remote_name: r,
                branch_name: b,
            }
        })
    };

    let resolved = target.unwrap_or_else(|| WorktreeBaseRemote {
        remote_name: "origin".to_owned(),
        branch_name: ref_name.to_owned(),
    });
    if remote_names.iter().any(|r| r == &resolved.remote_name) {
        Ok(Some(resolved))
    } else {
        Ok(None)
    }
}

fn fetch_remote(
    cwd: &Path,
    remote_name: &str,
    ref_name: Option<&str>,
    ctx: &GitContext,
) -> Result<bool, String> {
    let run_fetch = |fetch_args: &[&str]| -> Result<GitOutput, String> {
        let first = run_git_raw(
            "GitVcsDriver.fetchRemote",
            cwd,
            fetch_args,
            DEFAULT_TIMEOUT,
            ctx,
        )?;
        if first.exit_code != Some(0) && is_ref_lock_failure(&first.stderr) {
            return run_git_raw(
                "GitVcsDriver.fetchRemote",
                cwd,
                fetch_args,
                DEFAULT_TIMEOUT,
                ctx,
            );
        }
        Ok(first)
    };

    match ref_name {
        None => {
            let res = run_fetch(&["fetch", "--quiet", "--", remote_name])?;
            if res.exit_code == Some(0) {
                Ok(true)
            } else {
                Err(git_error(
                    "GitVcsDriver.fetchRemote",
                    cwd,
                    &res.stderr,
                    &format!("git fetch {remote_name} failed"),
                ))
            }
        }
        Some(ref_spec_name) => {
            let branch =
                parse_remote_ref_with_remote_names(ref_spec_name, &[remote_name.to_owned()])
                    .map(|(_, _, b)| b)
                    .unwrap_or_else(|| ref_spec_name.to_owned());
            let refspec = format!("+refs/heads/{branch}:refs/remotes/{remote_name}/{branch}");
            let res = run_fetch(&["fetch", "--quiet", "--no-tags", "--", remote_name, &refspec])?;
            if res.exit_code == Some(0) {
                return Ok(true);
            }
            let missing_line = format!("fatal: couldn't find remote ref refs/heads/{branch}");
            if res
                .stderr
                .lines()
                .any(|line| line.trim_end() == missing_line)
            {
                return Ok(false);
            }
            Err(git_error(
                "GitVcsDriver.fetchRemote",
                cwd,
                &res.stderr,
                &format!("git fetch {remote_name} failed"),
            ))
        }
    }
}

fn fetch_worktree_base_commit(
    cwd: &Path,
    remote: &WorktreeBaseRemote,
    ctx: &GitContext,
) -> Result<Option<(String, String)>, String> {
    let remote_ref_name = format!("{}/{}", remote.remote_name, remote.branch_name);
    if !fetch_remote(cwd, &remote.remote_name, Some(&remote_ref_name), ctx)? {
        return Ok(None);
    }
    let rev = format!("refs/remotes/{remote_ref_name}^{{commit}}");
    let resolved = run_git_raw(
        "GitVcsDriver.fetchWorktreeBaseCommit",
        cwd,
        &["rev-parse", "--verify", "--quiet", &rev],
        SHORT_TIMEOUT,
        ctx,
    )?;
    if resolved.exit_code == Some(0) {
        Ok(Some((resolved.stdout.trim().to_owned(), remote_ref_name)))
    } else {
        Ok(None)
    }
}

fn resolve_branch_base(cwd: &Path, ref_name: &str, ctx: &GitContext) -> Option<(String, String)> {
    let trimmed = ref_name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(short) = trimmed.strip_prefix("refs/remotes/") {
        let branch_name = short
            .split_once('/')
            .map(|(_, b)| b.trim())
            .filter(|b| !b.is_empty())
            .unwrap_or(short);
        return Some((trimmed.to_owned(), branch_name.to_owned()));
    }
    if let Some(short) = trimmed.strip_prefix("refs/heads/") {
        return Some((trimmed.to_owned(), short.to_owned()));
    }
    let remotes = run_git_raw(
        "GitVcsDriver.listRemoteNames",
        cwd,
        &["remote"],
        SHORT_TIMEOUT,
        ctx,
    )
    .ok()
    .filter(|o| o.exit_code == Some(0))
    .map(|o| parse_remote_names(&o.stdout))
    .unwrap_or_default();
    if let Some((_, _, branch_name)) = parse_remote_ref_with_remote_names(trimmed, &remotes)
        && !branch_exists(cwd, trimmed, ctx)
    {
        return Some((format!("refs/remotes/{trimmed}"), branch_name));
    }
    Some((trimmed.to_owned(), trimmed.to_owned()))
}

/// Ported from KSG Code's `createWorktree` (`GitVcsDriverCore.ts`) and `startFromOrigin`
/// (`ThreadLaunchService.ts`).
pub fn create_worktree(
    input: &VcsCreateWorktreeInput,
    worktrees_base_dir: &Path,
) -> Result<VcsCreateWorktreeResult, String> {
    create_worktree_with(input, worktrees_base_dir, &GitContext::default())
}

pub fn create_worktree_with(
    input: &VcsCreateWorktreeInput,
    worktrees_base_dir: &Path,
    ctx: &GitContext,
) -> Result<VcsCreateWorktreeResult, String> {
    let cwd = validate_existing_directory(&input.cwd, "cwd")?;
    let ref_name = input.ref_name.trim();
    if ref_name.is_empty() {
        return Err("refName must be a non-empty string.".into());
    }
    let new_ref_name = match input.new_ref_name.as_deref() {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err("newRefName must be a non-empty string.".into());
            }
            Some(trimmed)
        }
        None => None,
    };
    let explicit_base_ref_name = match input.base_ref_name.as_deref() {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err("baseRefName must be a non-empty string.".into());
            }
            Some(trimmed)
        }
        None => None,
    };
    refuse_option_like_ref_name("GitVcsDriver.createWorktree", &cwd, Some(ref_name))?;
    refuse_option_like_ref_name("GitVcsDriver.createWorktree", &cwd, new_ref_name)?;

    let target_branch = new_ref_name.unwrap_or(ref_name);
    let git_common_dir = read_git_common_dir(&cwd, ctx)?.ok_or_else(|| {
        git_error(
            "GitVcsDriver.createWorktree",
            &cwd,
            "",
            "Cannot create a worktree outside a Git repository.",
        )
    })?;

    let worktree_path = match input.path.as_deref() {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err("path must be a non-empty string or null.".into());
            }
            let p = Path::new(trimmed);
            if !p.is_absolute() || !has_only_normal_or_root_components(p) {
                return Err("Worktree path must be a normalized absolute path.".into());
            }
            p.to_path_buf()
        }
        None => build_worktree_path(worktrees_base_dir, &cwd, &git_common_dir, target_branch)?,
    };

    let mut start_ref = ref_name.to_owned();
    let mut base_ref_name = explicit_base_ref_name.map(str::to_owned);
    if input.start_from_origin == Some(true)
        && let Some(remote) = resolve_worktree_base_remote_with(&cwd, ref_name, ctx)?
        && let Some((commit_sha, remote_ref_name)) = fetch_worktree_base_commit(&cwd, &remote, ctx)?
    {
        start_ref = commit_sha;
        if base_ref_name.is_none() {
            base_ref_name = Some(format!("refs/remotes/{remote_ref_name}"));
        }
    }

    if let Some(new_branch) = new_ref_name {
        run_git_checked(
            "GitVcsDriver.createWorktree.createBranch",
            &cwd,
            &["branch", "--", new_branch, &start_ref],
            DEFAULT_TIMEOUT,
            "git branch failed",
            ctx,
        )?;
    }

    let checkout_ref = new_ref_name.unwrap_or(&start_ref);
    let checkout_workers =
        read_config_value(&cwd, "checkout.workers", ctx).unwrap_or_else(|| "0".to_owned());
    let workers_arg = format!("checkout.workers={checkout_workers}");
    let worktree_path_str = worktree_path.to_string_lossy().into_owned();

    let add_res = run_git_checked(
        "GitVcsDriver.createWorktree",
        &cwd,
        &[
            "-c",
            &workers_arg,
            "worktree",
            "add",
            "--",
            &worktree_path_str,
            checkout_ref,
        ],
        WORKTREE_TIMEOUT,
        "git worktree add failed",
        ctx,
    );

    if let Err(err) = add_res {
        if let Some(new_branch) = new_ref_name {
            let _ = run_git_raw(
                "GitVcsDriver.deleteLocalBranch",
                &cwd,
                &["branch", "-D", "--", new_branch],
                SHORT_TIMEOUT,
                ctx,
            );
        }
        return Err(err);
    }

    let _ = run_git_raw(
        "GitVcsDriver.createWorktree.refreshIndex",
        &worktree_path,
        &["update-index", "-q", "--refresh", "--force-write-index"],
        WORKTREE_TIMEOUT,
        ctx,
    );

    if let Some(new_branch) = new_ref_name
        && let Some(ref base_raw) = base_ref_name
        && let Some((base_ref, branch_name)) = resolve_branch_base(&cwd, base_raw, ctx)
    {
        let _ = write_config_value(
            &cwd,
            &format!("branch.{new_branch}.t3-merge-base"),
            &branch_name,
            ctx,
        );
        if base_ref.starts_with("refs/remotes/") {
            let _ = write_config_value(
                &cwd,
                &format!("branch.{new_branch}.t3-merge-base-ref"),
                &base_ref,
                ctx,
            );
        }
    }

    Ok(VcsCreateWorktreeResult {
        worktree: VcsWorktree {
            path: worktree_path_str,
            ref_name: target_branch.to_owned(),
        },
    })
}

/// Ported from KSG Code's `removeWorktree` in `GitVcsDriverCore.ts` and `worktreeRemoval.ts`.
pub fn remove_worktree(input: &VcsRemoveWorktreeInput) -> Result<(), String> {
    remove_worktree_with(input, &GitContext::default())
}

pub fn remove_worktree_with(
    input: &VcsRemoveWorktreeInput,
    ctx: &GitContext,
) -> Result<(), String> {
    let cwd = validate_existing_directory(&input.cwd, "cwd")?;
    let raw_path = input.path.trim();
    if raw_path.is_empty() {
        return Err("path must be a non-empty string.".into());
    }
    let target = {
        let p = Path::new(raw_path);
        if p.is_absolute() {
            normalize_lexical(p)
        } else {
            normalize_lexical(&cwd.join(p))
        }
    };
    let target_str = target.to_string_lossy().into_owned();
    let force = input.force == Some(true);

    let mut args = vec![
        "-c",
        "status.showUntrackedFiles=normal",
        "worktree",
        "remove",
    ];
    if force {
        args.push("--force");
    }
    args.push("--");
    args.push(&target_str);

    let res = run_git_raw(
        "GitVcsDriver.removeWorktree",
        &cwd,
        &args,
        WORKTREE_TIMEOUT,
        ctx,
    )?;
    if res.exit_code == Some(0) {
        return Ok(());
    }
    if is_missing_worktree_stderr(&res.stderr) && !target.exists() {
        run_git_checked(
            "GitVcsDriver.pruneWorktrees",
            &cwd,
            &["worktree", "prune"],
            DEFAULT_TIMEOUT,
            "git worktree prune failed",
            ctx,
        )?;
        return Ok(());
    }
    Err(git_error(
        "GitVcsDriver.removeWorktree",
        &cwd,
        &res.stderr,
        "git worktree remove failed",
    ))
}

pub fn list_worktrees(input: &VcsListWorktreesInput) -> Result<VcsListWorktreesResult, String> {
    list_worktrees_with(input, &GitContext::default())
}

pub fn list_worktrees_with(
    input: &VcsListWorktreesInput,
    ctx: &GitContext,
) -> Result<VcsListWorktreesResult, String> {
    let cwd = validate_existing_directory(&input.cwd, "cwd")?;
    let Some(git_common_dir) = read_git_common_dir(&cwd, ctx)? else {
        return Ok(VcsListWorktreesResult {
            is_repo: false,
            worktrees: Vec::new(),
        });
    };
    let fetch_cwd = fetch_cwd_for_common_dir(&git_common_dir);
    let stdout = execute_worktree_list(
        "GitVcsDriver.listWorktrees",
        fetch_cwd,
        Some(&git_common_dir),
        ctx,
    )?;
    Ok(VcsListWorktreesResult {
        is_repo: true,
        worktrees: parse_worktree_entries(&stdout),
    })
}

pub fn is_registered_worktree(cwd: &Path, target: &Path) -> Result<bool, String> {
    let listed = list_worktrees(&VcsListWorktreesInput {
        cwd: cwd.to_string_lossy().into_owned(),
    })?;
    if !listed.is_repo {
        return Ok(false);
    }
    let real_target = real_location(target);
    Ok(listed
        .worktrees
        .iter()
        .any(|entry| real_location(Path::new(&entry.path)) == real_target))
}

/// Ported from KSG Code's `packages/shared/src/git.ts` (`sanitizeBranchFragment`).
pub fn sanitize_branch_fragment(raw: &str) -> String {
    let stripped = strip_combining_marks(raw);
    let trimmed = stripped.trim().to_ascii_lowercase();
    let no_quotes: String = trimmed
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '`'))
        .collect();
    let trimmed_edges = trim_branch_edge_chars(&no_quotes);

    let mut mapped = String::with_capacity(trimmed_edges.len());
    for ch in trimmed_edges.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '/' | '_' | '-') {
            mapped.push(ch);
        } else {
            mapped.push('-');
        }
    }
    let collapsed_slashes = collapse_char(&mapped, '/');
    let collapsed_dashes = collapse_char(&collapsed_slashes, '-');
    let edge_clean = trim_branch_edge_chars(&collapsed_dashes);

    let mut sliced = String::new();
    for ch in edge_clean.chars() {
        if sliced.chars().count() >= 64 {
            break;
        }
        sliced.push(ch);
    }
    let final_clean = trim_branch_edge_chars(&sliced);
    if final_clean.is_empty() {
        "update".to_owned()
    } else {
        final_clean.to_owned()
    }
}

fn strip_combining_marks(input: &str) -> String {
    // Handle common Latin diacritics deterministically without external Unicode tables.
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let replacement = match ch {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => {
                Some('a')
            }
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => Some('e'),
            'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => Some('i'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' | 'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' | 'Ø' => {
                Some('o')
            }
            'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => Some('u'),
            'ñ' | 'Ñ' => Some('n'),
            'ç' | 'Ç' => Some('c'),
            'ý' | 'ÿ' | 'Ý' => Some('y'),
            '\u{0300}'..='\u{036f}' => None,
            other => Some(other),
        };
        if let Some(c) = replacement {
            out.push(c);
        }
    }
    out
}

fn trim_branch_edge_chars(s: &str) -> &str {
    s.trim_matches(|c: char| c == '.' || c == '/' || c == '_' || c == '-' || c.is_whitespace())
}

fn collapse_char(s: &str, target: char) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_match = false;
    for ch in s.chars() {
        if ch == target {
            if !prev_match {
                out.push(ch);
                prev_match = true;
            }
        } else {
            out.push(ch);
            prev_match = false;
        }
    }
    out
}

/// Ported from KSG Code's `formatGeneratedBranchName` in `packages/shared/src/git.ts`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn format_generated_branch_name(raw: &str, mode: BranchNamingMode) -> String {
    match mode {
        BranchNamingMode::Custom => raw.trim().to_owned(),
        BranchNamingMode::Semantic => sanitize_branch_fragment(raw),
        BranchNamingMode::Static { prefix } => {
            let branch = sanitize_branch_fragment(raw);
            let parts: Vec<String> = prefix
                .split('/')
                .map(|part| {
                    let mut mapped = String::new();
                    for ch in part.chars() {
                        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                            mapped.push(ch);
                        } else {
                            mapped.push('-');
                        }
                    }
                    let collapsed = collapse_char(&mapped, '-');
                    collapsed.trim_matches('-').to_owned()
                })
                .filter(|p| !p.is_empty())
                .collect();
            let clean_prefix = parts.join("/");
            if clean_prefix.is_empty() {
                branch
            } else {
                format!("{clean_prefix}/{branch}")
            }
        }
    }
}

/// Ported from KSG Code's `sanitizeFeatureBranchName` in `packages/shared/src/git.ts`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn sanitize_feature_branch_name(raw: &str) -> String {
    let sanitized = sanitize_branch_fragment(raw);
    if sanitized.contains('/') {
        sanitized
    } else {
        format!("feature/{sanitized}")
    }
}

/// Ported from KSG Code's `resolveAutoFeatureBranchName` in `packages/shared/src/git.ts`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_auto_feature_branch_name(
    existing_branch_names: &[&str],
    preferred_branch: Option<&str>,
) -> String {
    let preferred = preferred_branch
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(AUTO_FEATURE_BRANCH_FALLBACK);
    let resolved_base = sanitize_feature_branch_name(preferred);
    let existing_lower: Vec<String> = existing_branch_names
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let existing_refs: HashSet<&str> = existing_lower.iter().map(String::as_str).collect();
    let parts: Vec<&str> = resolved_base.split('/').collect();
    let mut resolved = String::new();

    for (index, part) in parts.iter().enumerate() {
        let base = if resolved.is_empty() {
            (*part).to_owned()
        } else {
            format!("{resolved}/{part}")
        };
        let mut candidate = base.clone();
        let mut suffix = 2usize;
        let is_last = index + 1 == parts.len();
        while existing_refs.contains(candidate.as_str())
            || (is_last
                && existing_lower
                    .iter()
                    .any(|name| name.starts_with(&format!("{candidate}/"))))
        {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        resolved = candidate;
    }

    resolved
}

/// Ported from KSG Code's `resolveAvailableBranchName` in `GitVcsDriverCore.ts`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_available_branch_name_from_list(
    existing_branches: &[String],
    requested_branch: &str,
) -> Result<String, String> {
    let branches: HashSet<String> = existing_branches
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let mut blocked_names = branches.clone();
    for branch in &branches {
        let segments: Vec<&str> = branch.split('/').collect();
        for idx in 1..segments.len() {
            blocked_names.insert(segments[..idx].join("/"));
        }
    }
    let segments: Vec<&str> = requested_branch.split('/').collect();
    let blocked_by_parent = (1..segments.len())
        .any(|idx| branches.contains(&segments[..idx].join("/").to_ascii_lowercase()));
    let desired = if blocked_by_parent {
        segments.join("-")
    } else {
        requested_branch.to_owned()
    };
    if !blocked_names.contains(&desired.to_ascii_lowercase()) {
        return Ok(desired);
    }
    for suffix in 1..=100 {
        let candidate = format!("{desired}-{suffix}");
        if !blocked_names.contains(&candidate.to_ascii_lowercase()) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Could not find an available branch name for '{desired}'."
    ))
}

#[allow(dead_code)]
pub fn resolve_available_branch_name(cwd: &Path, requested_branch: &str) -> Result<String, String> {
    resolve_available_branch_name_with(cwd, requested_branch, &GitContext::default())
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_available_branch_name_with(
    cwd: &Path,
    requested_branch: &str,
    ctx: &GitContext,
) -> Result<String, String> {
    let out = run_git_checked(
        "GitVcsDriver.resolveAvailableBranchName",
        cwd,
        &["for-each-ref", "--format=%(refname:strip=2)", "refs/heads/"],
        DEFAULT_TIMEOUT,
        "git for-each-ref failed.",
        ctx,
    )?;
    let existing: Vec<String> = out
        .stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    resolve_available_branch_name_from_list(&existing, requested_branch)
}

/// Ported from KSG Code's `buildTemporaryWorktreeBranchName` in `packages/shared/src/git.ts`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn build_temporary_worktree_branch_name(random_hex: &str) -> String {
    let token: String = random_hex
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(8)
        .collect();
    format!("{WORKTREE_BRANCH_PREFIX}/{token}")
}

/// Ported from KSG Code's `flattenTemporaryWorktreeBranchName` in `packages/shared/src/git.ts`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn flatten_temporary_worktree_branch_name(ref_name: &str) -> String {
    let normalized = ref_name.trim().to_ascii_lowercase();
    let token_start = normalized.find(['-', '/']).map_or(0, |idx| idx + 1);
    let token: String = normalized[token_start..].chars().take(8).collect();
    format!("{WORKTREE_BRANCH_PREFIX}-{token}")
}

/// Ported from KSG Code's `isTemporaryWorktreeBranch` in `packages/shared/src/git.ts`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn is_temporary_worktree_branch(ref_name: &str) -> bool {
    let trimmed = ref_name.trim().to_ascii_lowercase();
    if let Some(rest) = trimmed
        .strip_prefix("t3/")
        .or_else(|| trimmed.strip_prefix("t3-"))
    {
        return rest.len() == 8 && rest.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if let Some(rest) = trimmed.strip_prefix("t3code-") {
        return rest.len() == 8 && rest.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if let Some(rest) = trimmed.strip_prefix("t3code/") {
        if rest.len() == 8 && rest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return true;
        }
        return is_uuid_v4(rest);
    }
    false
}

fn is_uuid_v4(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, &b) in bytes.iter().enumerate() {
        match idx {
            8 | 13 | 18 | 23 => {
                if b != b'-' {
                    return false;
                }
            }
            14 => {
                if b != b'4' {
                    return false;
                }
            }
            19 => {
                if !matches!(b, b'8' | b'9' | b'a' | b'b') {
                    return false;
                }
            }
            _ => {
                if !b.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

/// Ported from KSG Code's `sanitizeNewRefName` in `packages/shared/src/git.ts`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn sanitize_new_ref_name(raw_name: &str) -> String {
    let mut out = String::new();
    let mut in_ws = false;
    for ch in raw_name.trim().chars() {
        if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}' | '\u{000b}') {
            if !in_ws {
                out.push('-');
                in_ws = true;
            }
        } else {
            out.push(ch);
            in_ws = false;
        }
    }
    out
}

/// Reads the setup script configured in `t3.json` (ported from KSG Code's `t3ProjectFile.ts`
/// and `setupProjectScript` in `packages/shared/src/projectScripts.ts`) and runs it in the
/// worktree directory, returning exit status and a bounded tail of combined output.
pub fn run_setup(input: &VcsRunSetupInput) -> Result<VcsRunSetupResult, String> {
    let project_cwd = validate_existing_directory(&input.cwd, "cwd")?;
    let worktree_path = match input.worktree_path.as_deref() {
        Some(raw) => validate_existing_directory(raw, "worktreePath")?,
        None => project_cwd.clone(),
    };

    let script = find_setup_script(&worktree_path, &project_cwd)?;
    let Some(script) = script else {
        return Ok(VcsRunSetupResult {
            status: "no_script".to_owned(),
            script_name: None,
            script_command: None,
            exit_code: None,
            timed_out: false,
            output: String::new(),
            duration_ms: 0,
        });
    };

    let timeout = input
        .timeout_ms
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(SETUP_DEFAULT_TIMEOUT);

    let started = Instant::now();
    let mut cmd = setup_shell_command(&script.command);
    cmd.current_dir(&worktree_path)
        .env("T3CODE_PROJECT_ROOT", &project_cwd)
        .env("T3CODE_WORKTREE_PATH", &worktree_path)
        .env("COLORTERM", "")
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut process = crate::process::Process::spawn(&mut cmd)
        .map_err(|err| format!("Failed to start setup script: {err}"))?;
    let stdout = process.stdout.take().expect("piped stdout");
    let stderr = process.stderr.take().expect("piped stderr");

    let tail = Arc::new(Mutex::new(BoundedTail::new(SETUP_OUTPUT_TAIL_BYTES)));
    let tail_out = tail.clone();
    let out_handle = thread::spawn(move || read_into_tail(stdout, tail_out));
    let tail_err = tail.clone();
    let err_handle = thread::spawn(move || read_into_tail(stderr, tail_err));

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let exit_code = loop {
        match process.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {}
            Err(err) => {
                process.terminate();
                let _ = process.reap(Duration::from_secs(1));
                return Err(format!("Failed while waiting for setup script: {err}"));
            }
        }
        if Instant::now() >= deadline {
            timed_out = true;
            process.terminate();
            let _ = process.reap(Duration::from_secs(1));
            break None;
        }
        thread::sleep(Duration::from_millis(10));
    };

    if !timed_out {
        process.terminate();
    }
    let _ = crate::process::join_for(out_handle, Duration::from_secs(1));
    let _ = crate::process::join_for(err_handle, Duration::from_secs(1));
    let output = tail.lock().expect("setup output tail").into_string();
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok(VcsRunSetupResult {
        status: "completed".to_owned(),
        script_name: Some(script.name),
        script_command: Some(script.command),
        exit_code,
        timed_out,
        output,
        duration_ms,
    })
}

pub fn find_setup_script_info(
    worktree_path: &Path,
    project_cwd: &Path,
) -> Result<Option<(String, String)>, String> {
    Ok(find_setup_script(worktree_path, project_cwd)?.map(|s| (s.name, s.command)))
}

fn find_setup_script(
    worktree_path: &Path,
    project_cwd: &Path,
) -> Result<Option<T3ProjectFileScript>, String> {
    let mut candidates = vec![worktree_path.join(T3_PROJECT_FILE_NAME)];
    if project_cwd != worktree_path {
        candidates.push(project_cwd.join(T3_PROJECT_FILE_NAME));
    }
    for path in candidates {
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(format!(
                    "Failed to read {} at {}: {err}",
                    T3_PROJECT_FILE_NAME,
                    path.display()
                ));
            }
        };
        let parsed: T3ProjectFile = serde_json_lenient::from_str(&raw).map_err(|err| {
            format!(
                "Invalid {} at {}: {err}",
                T3_PROJECT_FILE_NAME,
                path.display()
            )
        })?;
        return Ok(parsed
            .scripts
            .into_iter()
            .find(|script| script.run_on_worktree_create && !script.command.trim().is_empty()));
    }
    Ok(None)
}

fn setup_shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", command]);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    }
}

struct BoundedTail {
    max_bytes: usize,
    buf: Vec<u8>,
}

impl BoundedTail {
    fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            buf: Vec::new(),
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        if self.max_bytes == 0 {
            return;
        }
        if chunk.len() >= self.max_bytes {
            self.buf.clear();
            self.buf
                .extend_from_slice(&chunk[chunk.len() - self.max_bytes..]);
            return;
        }
        self.buf.extend_from_slice(chunk);
        if self.buf.len() > self.max_bytes {
            let excess = self.buf.len() - self.max_bytes;
            self.buf.drain(..excess);
        }
    }

    fn into_string(&self) -> String {
        let mut start = 0;
        while start < self.buf.len() && (self.buf[start] & 0b1100_0000) == 0b1000_0000 {
            start += 1;
        }
        String::from_utf8_lossy(&self.buf[start..]).into_owned()
    }
}

fn read_into_tail<R: Read>(mut reader: R, tail: Arc<Mutex<BoundedTail>>) {
    let mut buf = [0u8; 4096];
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        tail.lock().expect("tail lock").push(&buf[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("shina-vcs-{label}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    struct TestEnv {
        _root: TempDir,
        empty_config: PathBuf,
        ctx: GitContext,
    }

    impl TestEnv {
        fn new(label: &str) -> Self {
            let root = TempDir::new(label);
            let empty_config = root.path.join("empty.gitconfig");
            std::fs::write(&empty_config, "").unwrap();
            let ctx = GitContext {
                git_config_global: Some(empty_config.clone()),
                git_config_system: Some(empty_config.clone()),
            };
            Self {
                _root: root,
                empty_config,
                ctx,
            }
        }

        fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let mut cmd = Command::new("git");
            cmd.arg("-c")
                .arg("user.name=Test User")
                .arg("-c")
                .arg("user.email=test@example.com")
                .args(args)
                .current_dir(cwd)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_CONFIG_GLOBAL", &self.empty_config)
                .env("GIT_CONFIG_SYSTEM", &self.empty_config);
            let out = crate::process::output(&mut cmd, Duration::from_secs(10), 1024 * 1024)
                .expect("spawn git");
            assert!(
                out.status.success(),
                "git {:?} failed in {}: {}",
                args,
                cwd.display(),
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        }

        fn init_repo(&self, name: &str) -> PathBuf {
            let repo = self._root.path.join(name);
            std::fs::create_dir_all(&repo).unwrap();
            self.git(&repo, &["init", "-b", "main", "--quiet"]);
            std::fs::write(repo.join("README.md"), "hello\n").unwrap();
            self.git(&repo, &["add", "README.md"]);
            self.git(&repo, &["commit", "-m", "initial commit", "--quiet"]);
            repo
        }

        fn worktrees_dir(&self) -> PathBuf {
            let dir = self._root.path.join("worktrees");
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }
    }

    macro_rules! require_git {
        () => {
            if !git_available() {
                eprintln!("skipping test: git is not on PATH");
                return;
            }
        };
    }

    #[test]
    fn list_branches_with_default_and_remote_and_ordering() {
        require_git!();
        let env = TestEnv::new("branches");
        let bare = env._root.path.join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        env.git(&bare, &["init", "--bare", "-b", "main", "--quiet"]);

        let repo = env.init_repo("repo");
        env.git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        env.git(&repo, &["push", "-u", "origin", "main", "--quiet"]);
        env.git(
            &repo,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );

        // Create a remote-only branch via a second clone so it appears in origin/*
        let peer = env._root.path.join("peer");
        env.git(
            &env._root.path,
            &[
                "clone",
                "--quiet",
                bare.to_str().unwrap(),
                peer.to_str().unwrap(),
            ],
        );
        env.git(&peer, &["checkout", "-b", "remote-only", "--quiet"]);
        std::fs::write(peer.join("remote.txt"), "r\n").unwrap();
        env.git(&peer, &["add", "remote.txt"]);
        env.git(&peer, &["commit", "-m", "remote commit", "--quiet"]);
        env.git(&peer, &["push", "origin", "remote-only", "--quiet"]);

        env.git(&repo, &["fetch", "origin", "--quiet"]);
        env.git(&repo, &["checkout", "-b", "feature/active", "--quiet"]);

        let res = list_branches_with(
            &VcsListBranchesInput {
                cwd: repo.to_string_lossy().into_owned(),
                query: None,
                cursor: None,
                include_matching_remote_refs: None,
                ref_kind: None,
                refresh: None,
                limit: None,
            },
            &env.ctx,
        )
        .unwrap();

        assert!(res.is_repo);
        assert!(res.has_primary_remote);
        assert_eq!(res.is_at_repository_root, Some(true));
        // Current branch first, default branch second, remote-only third (origin/main deduped).
        let names: Vec<&str> = res.refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["feature/active", "main", "origin/remote-only"]);
        assert!(res.refs[0].current);
        assert!(!res.refs[0].is_default);
        assert!(res.refs[1].is_default);
        assert!(!res.refs[1].current);
        assert_eq!(res.refs[2].is_remote, Some(true));
        assert_eq!(res.refs[2].remote_name.as_deref(), Some("origin"));

        // includeMatchingRemoteRefs keeps origin/main as well.
        let with_matching = list_branches_with(
            &VcsListBranchesInput {
                cwd: repo.to_string_lossy().into_owned(),
                query: None,
                cursor: None,
                include_matching_remote_refs: Some(true),
                ref_kind: None,
                refresh: None,
                limit: None,
            },
            &env.ctx,
        )
        .unwrap();
        assert!(
            with_matching
                .refs
                .iter()
                .any(|r| r.name == "origin/main" && r.is_default && r.is_remote == Some(true))
        );
    }

    #[test]
    fn status_clean_dirty_and_detached() {
        require_git!();
        let env = TestEnv::new("status");
        let repo = env.init_repo("repo");

        let clean = status_with(
            &VcsStatusInput {
                cwd: repo.to_string_lossy().into_owned(),
            },
            &env.ctx,
        )
        .unwrap();
        assert!(clean.is_repo);
        assert_eq!(clean.ref_name.as_deref(), Some("main"));
        assert!(!clean.is_detached());
        assert!(!clean.has_working_tree_changes);
        assert!(clean.is_default_ref);

        // Dirty: modify tracked file and add untracked file.
        std::fs::write(repo.join("README.md"), "hello\nworld\n").unwrap();
        std::fs::write(repo.join("new.txt"), "new\n").unwrap();
        let dirty = status_with(
            &VcsStatusInput {
                cwd: repo.to_string_lossy().into_owned(),
            },
            &env.ctx,
        )
        .unwrap();
        assert!(dirty.has_working_tree_changes);
        assert_eq!(dirty.working_tree.insertions, 1);
        assert!(
            dirty
                .working_tree
                .files
                .iter()
                .any(|f| f.path == "README.md" && f.insertions == 1)
        );
        assert!(dirty.working_tree.files.iter().any(|f| f.path == "new.txt"));

        // Detached HEAD.
        env.git(&repo, &["checkout", "--detach", "HEAD", "--quiet"]);
        let detached = status_with(
            &VcsStatusInput {
                cwd: repo.to_string_lossy().into_owned(),
            },
            &env.ctx,
        )
        .unwrap();
        assert!(detached.is_repo);
        assert_eq!(detached.ref_name, None);
        assert!(detached.is_detached());
    }

    #[test]
    fn non_repository_folder_returns_distinct_non_repo_result() {
        require_git!();
        let env = TestEnv::new("non-repo");
        let plain = env._root.path.join("plain-dir");
        std::fs::create_dir_all(&plain).unwrap();

        let branches = list_branches_with(
            &VcsListBranchesInput {
                cwd: plain.to_string_lossy().into_owned(),
                query: None,
                cursor: None,
                include_matching_remote_refs: None,
                ref_kind: None,
                refresh: None,
                limit: None,
            },
            &env.ctx,
        )
        .unwrap();
        assert!(!branches.is_repo);
        assert!(branches.refs.is_empty());
        assert_eq!(branches.total_count, 0);

        let st = status_with(
            &VcsStatusInput {
                cwd: plain.to_string_lossy().into_owned(),
            },
            &env.ctx,
        )
        .unwrap();
        assert!(!st.is_repo);
        assert_eq!(st.ref_name, None);
        assert!(!st.is_detached());
    }

    #[test]
    fn create_worktree_from_local_base_and_remove_clean_and_dirty() {
        require_git!();
        let env = TestEnv::new("wt-local");
        let repo = env.init_repo("repo");
        let wt_base = env.worktrees_dir();

        let created = create_worktree_with(
            &VcsCreateWorktreeInput {
                cwd: repo.to_string_lossy().into_owned(),
                ref_name: "main".into(),
                new_ref_name: Some("feature/worktree-test".into()),
                base_ref_name: Some("main".into()),
                path: None,
                start_from_origin: Some(false),
            },
            &wt_base,
            &env.ctx,
        )
        .unwrap();

        let wt_path = PathBuf::from(&created.worktree.path);
        assert!(wt_path.exists());
        assert!(is_managed_worktree_path(&wt_path, &wt_base));
        assert_eq!(created.worktree.ref_name, "feature/worktree-test");

        let listed = list_worktrees_with(
            &VcsListWorktreesInput {
                cwd: repo.to_string_lossy().into_owned(),
            },
            &env.ctx,
        )
        .unwrap();
        assert!(listed.is_repo);
        assert_eq!(listed.worktrees.len(), 2);
        assert!(
            listed
                .worktrees
                .iter()
                .any(|w| w.ref_name.as_deref() == Some("feature/worktree-test"))
        );

        // Remove clean worktree without force.
        remove_worktree_with(
            &VcsRemoveWorktreeInput {
                cwd: repo.to_string_lossy().into_owned(),
                path: created.worktree.path.clone(),
                force: Some(false),
            },
            &env.ctx,
        )
        .unwrap();
        assert!(!wt_path.exists());

        // Create second worktree, dirty it, verify non-force remove fails and force succeeds.
        let created2 = create_worktree_with(
            &VcsCreateWorktreeInput {
                cwd: repo.to_string_lossy().into_owned(),
                ref_name: "main".into(),
                new_ref_name: Some("feature/dirty-wt".into()),
                base_ref_name: Some("main".into()),
                path: None,
                start_from_origin: None,
            },
            &wt_base,
            &env.ctx,
        )
        .unwrap();
        let wt2_path = PathBuf::from(&created2.worktree.path);
        std::fs::write(wt2_path.join("untracked.txt"), "dirty\n").unwrap();

        let non_force_err = remove_worktree_with(
            &VcsRemoveWorktreeInput {
                cwd: repo.to_string_lossy().into_owned(),
                path: created2.worktree.path.clone(),
                force: Some(false),
            },
            &env.ctx,
        )
        .unwrap_err();
        assert!(non_force_err.contains("GitVcsDriver.removeWorktree"));
        assert!(non_force_err.contains(&repo.to_string_lossy().into_owned()));
        assert!(wt2_path.exists());

        remove_worktree_with(
            &VcsRemoveWorktreeInput {
                cwd: repo.to_string_lossy().into_owned(),
                path: created2.worktree.path,
                force: Some(true),
            },
            &env.ctx,
        )
        .unwrap();
        assert!(!wt2_path.exists());
    }

    #[test]
    fn create_worktree_from_origin_after_remote_advanced() {
        require_git!();
        let env = TestEnv::new("wt-origin");
        let bare = env._root.path.join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        env.git(&bare, &["init", "--bare", "-b", "main", "--quiet"]);

        let repo = env.init_repo("repo");
        env.git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        env.git(&repo, &["push", "-u", "origin", "main", "--quiet"]);

        // Advance remote from a peer clone without fetching in `repo`.
        let peer = env._root.path.join("peer");
        env.git(
            &env._root.path,
            &[
                "clone",
                "--quiet",
                bare.to_str().unwrap(),
                peer.to_str().unwrap(),
            ],
        );
        std::fs::write(peer.join("ahead.txt"), "from origin\n").unwrap();
        env.git(&peer, &["add", "ahead.txt"]);
        env.git(&peer, &["commit", "-m", "advance remote", "--quiet"]);
        env.git(&peer, &["push", "origin", "main", "--quiet"]);
        let remote_head = env.git(&peer, &["rev-parse", "HEAD"]).trim().to_owned();

        let wt_base = env.worktrees_dir();
        let created = create_worktree_with(
            &VcsCreateWorktreeInput {
                cwd: repo.to_string_lossy().into_owned(),
                ref_name: "main".into(),
                new_ref_name: Some("feature/from-origin".into()),
                base_ref_name: None,
                path: None,
                start_from_origin: Some(true),
            },
            &wt_base,
            &env.ctx,
        )
        .unwrap();

        let wt_path = PathBuf::from(&created.worktree.path);
        let wt_head = env.git(&wt_path, &["rev-parse", "HEAD"]).trim().to_owned();
        assert_eq!(wt_head, remote_head);
        assert!(wt_path.join("ahead.txt").exists());
    }

    #[test]
    fn branch_name_generation_collision_and_validation_cases_from_ksg() {
        // Copied from KSG Code's packages/shared/src/git.test.ts
        assert!(is_temporary_worktree_branch("t3/deadbeef"));
        assert!(is_temporary_worktree_branch(" t3/deadbeef "));
        assert!(is_temporary_worktree_branch("t3/DEADBEEF"));
        assert_eq!(
            build_temporary_worktree_branch_name("f4ae4e0e-f971-4d48-b4f2-9cf0aa54ab12"),
            "t3/f4ae4e0e"
        );
        assert!(is_temporary_worktree_branch("t3code/deadbeef"));
        assert!(is_temporary_worktree_branch("t3code-deadbeef"));
        assert!(is_temporary_worktree_branch(
            "t3code/f4ae4e0e-f971-4d48-b4f2-9cf0aa54ab12"
        ));
        assert!(!is_temporary_worktree_branch(
            "t3/f4ae4e0e-f971-4d48-b4f2-9cf0aa54ab12"
        ));
        assert!(!is_temporary_worktree_branch(
            "t3code/f4ae4e0e-f971-1d48-b4f2-9cf0aa54ab12"
        ));
        assert!(!is_temporary_worktree_branch(
            "t3code/f4ae4e0e-f971-4d48-c4f2-9cf0aa54ab12"
        ));
        let flat = flatten_temporary_worktree_branch_name("t3/deadbeef");
        assert_eq!(flat, "t3-deadbeef");
        assert!(is_temporary_worktree_branch(&flat));
        assert!(!is_temporary_worktree_branch("t3-deadbeef-extra"));
        assert!(!is_temporary_worktree_branch("t3-feature"));
        assert!(!is_temporary_worktree_branch("t3/feature/demo"));
        assert!(!is_temporary_worktree_branch("main"));
        assert!(!is_temporary_worktree_branch("t3/deadbeef-extra"));

        assert_eq!(
            sanitize_feature_branch_name(" \"FIX//Calendar recruitment filter\" "),
            "fix/calendar-recruitment-filter"
        );
        assert_eq!(
            sanitize_feature_branch_name("refine toolbar"),
            "feature/refine-toolbar"
        );
        assert_eq!(sanitize_feature_branch_name(" /?. / "), "feature/update");
        assert_eq!(sanitize_feature_branch_name("fix///"), "feature/fix");
        assert_eq!(
            sanitize_feature_branch_name("fix/Añadir validación del café"),
            "fix/anadir-validacion-del-cafe"
        );

        assert_eq!(
            resolve_auto_feature_branch_name(
                &["FIX/refine-toolbar", "fix/refine-toolbar-2"],
                Some("fix/refine-toolbar"),
            ),
            "fix/refine-toolbar-3"
        );
        assert_eq!(
            resolve_auto_feature_branch_name(&["fix"], Some("fix/name")),
            "fix-2/name"
        );
        assert_eq!(
            resolve_auto_feature_branch_name(&["fix/name/child"], Some("fix/name")),
            "fix/name-2"
        );
        assert_eq!(
            resolve_auto_feature_branch_name(&["fixes", "fix/name-extra/child"], Some("fix/name")),
            "fix/name"
        );

        assert_eq!(
            format_generated_branch_name(
                "Add Search",
                BranchNamingMode::Static {
                    prefix: " Team / Jules.lock/".into(),
                }
            ),
            "Team/Jules-lock/add-search"
        );
        assert_eq!(
            format_generated_branch_name(
                "Add Search",
                BranchNamingMode::Static {
                    prefix: "-team//feature@{new}".into(),
                }
            ),
            "team/feature-new/add-search"
        );
        assert_eq!(
            format_generated_branch_name(
                "Add Search",
                BranchNamingMode::Static {
                    prefix: " /?. / ".into(),
                }
            ),
            "add-search"
        );
        assert_eq!(
            format_generated_branch_name("feat/Add Search", BranchNamingMode::Semantic),
            "feat/add-search"
        );
        assert_eq!(
            format_generated_branch_name("  custom/Branch-Name  ", BranchNamingMode::Custom),
            "custom/Branch-Name"
        );

        assert_eq!(sanitize_new_ref_name("  my \t branch  "), "my-branch");

        // Collision resolution against existing branches (GitVcsDriverCore.resolveAvailableBranchName)
        assert_eq!(
            resolve_available_branch_name_from_list(
                &["feature/foo".into(), "FEATURE/FOO-1".into()],
                "feature/foo",
            )
            .unwrap(),
            "feature/foo-2"
        );
        assert_eq!(
            resolve_available_branch_name_from_list(&["t3".into()], "t3/deadbeef").unwrap(),
            "t3-deadbeef"
        );
        assert_eq!(
            resolve_available_branch_name_from_list(&["t3-fix/child".into()], "t3-fix").unwrap(),
            "t3-fix-1"
        );

        // Option-like ref names refused.
        let tmp = TempDir::new("refuse-dash");
        assert!(refuse_option_like_ref_name("op", &tmp.path, Some("-bad")).is_err());
        assert!(refuse_option_like_ref_name("op", &tmp.path, Some("-")).is_ok());
    }

    #[test]
    fn name_collision_in_repo_and_setup_script_runner() {
        require_git!();
        let env = TestEnv::new("collision-setup");
        let repo = env.init_repo("repo");
        env.git(&repo, &["branch", "feature/dup"]);
        let resolved = resolve_available_branch_name_with(&repo, "feature/dup", &env.ctx).unwrap();
        assert_eq!(resolved, "feature/dup-1");

        // Setup script from t3.json (lenient JSON) runs in worktree with expected env and bounded tail.
        std::fs::write(
            repo.join("t3.json"),
            r#"{
                // trailing comment supported via lenient json
                "scripts": [
                    { "name": "Dev", "command": "echo dev" },
                    { "name": "Setup", "command": "echo root=$T3CODE_PROJECT_ROOT; echo wt=$T3CODE_WORKTREE_PATH; pwd", "runOnWorktreeCreate": true }
                ]
            }"#,
        )
        .unwrap();
        let wt_base = env.worktrees_dir();
        let wt = create_worktree_with(
            &VcsCreateWorktreeInput {
                cwd: repo.to_string_lossy().into_owned(),
                ref_name: "main".into(),
                new_ref_name: Some(resolved),
                base_ref_name: None,
                path: None,
                start_from_origin: None,
            },
            &wt_base,
            &env.ctx,
        )
        .unwrap();
        std::fs::copy(
            repo.join("t3.json"),
            Path::new(&wt.worktree.path).join("t3.json"),
        )
        .unwrap();

        let setup_res = run_setup(&VcsRunSetupInput {
            cwd: repo.to_string_lossy().into_owned(),
            worktree_path: Some(wt.worktree.path.clone()),
            timeout_ms: Some(10_000),
        })
        .unwrap();
        assert_eq!(setup_res.status, "completed");
        assert_eq!(setup_res.script_name.as_deref(), Some("Setup"));
        assert_eq!(setup_res.exit_code, Some(0));
        assert!(!setup_res.timed_out);
        assert!(
            setup_res
                .output
                .contains(&format!("root={}", repo.display()))
        );
        assert!(
            setup_res
                .output
                .contains(&format!("wt={}", wt.worktree.path))
        );

        // Managed worktree path validation rejects escapes, foreign paths, and wrong depths.
        assert!(is_managed_worktree_path(
            Path::new(&wt.worktree.path),
            &wt_base
        ));
        assert!(!is_managed_worktree_path(
            Path::new("relative/path"),
            &wt_base
        ));
        assert!(!is_managed_worktree_path(&wt_base.join("repo"), &wt_base));
        assert!(!is_managed_worktree_path(
            &wt_base.join("repo").join("wt").join("nested"),
            &wt_base
        ));
        assert!(!is_managed_worktree_path(
            &wt_base.join("repo").join("..").join("repo").join("wt"),
            &wt_base
        ));
        assert!(!is_managed_worktree_path(&repo, &wt_base));

        // Bounded tail keeps the tail of oversized output.
        let mut tail = BoundedTail::new(8);
        tail.push(b"012345");
        tail.push(b"6789abcd");
        assert_eq!(tail.into_string(), "6789abcd");
    }
}
