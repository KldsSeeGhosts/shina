//! Review diff backend ported from KSG Code.
//!
//! Reference sources in KSG Code:
//! - `packages/contracts/src/review.ts` (wire inputs, results, source kinds, limits)
//! - `apps/server/src/review/ReviewService.ts` (non-repository result and error shapes)
//! - `apps/server/src/vcs/GitVcsDriverCore.ts` (`getReviewBranchCommits`,
//!   `getReviewDiffPreview`, `getReviewDiffFileContents` and their helpers)

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    thread,
    time::{Duration, Instant},
};

use crate::vcs::{DEFAULT_TIMEOUT, SHORT_TIMEOUT};
use crate::vcs::{
    GitContext, branch_exists, branch_name_from_head_ref, configure_git_command, git_error,
    is_non_repository_stderr, is_unborn_head_stderr, normalize_lexical,
    resolve_base_branch_for_no_upstream, resolve_comparison_base_ref, validate_existing_directory,
};

const REVIEW_DIFF_PATCH_MAX_OUTPUT_BYTES: usize = 120_000;
const REVIEW_METADATA_MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const REVIEW_UNTRACKED_MAX_FILES: usize = 5_000;
const REVIEW_DIFF_FILE_MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const REVIEW_BRANCH_COMMIT_LIMIT: usize = 100;
const REVIEW_BRANCH_SUBJECT_LIMIT: usize = 300;
const REVIEW_BRANCH_COMMIT_MAX_OUTPUT_BYTES: usize = 128 * 1024;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 1_000_000;

// Client patch parsers require a/ and b/ prefixes; the -c override wins over user diff config.
const REVIEW_DIFF_ARGS: [&str; 10] = [
    "-c",
    "diff.suppressBlankEmpty=false",
    "diff",
    "--find-renames",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--submodule=short",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

// KSG blocks its secrets dir, `*.sqlite` directly under the state dir, and /dev/fd.
// Shina's secrets are these files anywhere under the state dir.
const SERVER_STATE_FILE_NAMES: [&str; 6] = [
    "server.json",
    "server.json.tmp",
    "credential.json",
    "credential.json.tmp",
    "client-credential.json",
    "client-credential.json.tmp",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewSourceKind {
    WorkingTree,
    BranchRange,
    Commit,
}

/// The top-level `sourceKind` of `review.getDiffPreview` accepts no "commit".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewPreviewSourceKind {
    WorkingTree,
    BranchRange,
}

impl From<ReviewPreviewSourceKind> for ReviewSourceKind {
    fn from(kind: ReviewPreviewSourceKind) -> Self {
        match kind {
            ReviewPreviewSourceKind::WorkingTree => ReviewSourceKind::WorkingTree,
            ReviewPreviewSourceKind::BranchRange => ReviewSourceKind::BranchRange,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewChangeType {
    Change,
    RenamePure,
    RenameChanged,
    New,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDiffFileArg {
    pub path: String,
    #[serde(default)]
    pub previous_path: Option<String>,
    pub source_kind: ReviewSourceKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDiffPreviewInput {
    pub cwd: String,
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default)]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub ignore_whitespace: Option<bool>,
    #[serde(default)]
    pub source_kind: Option<ReviewPreviewSourceKind>,
    #[serde(default)]
    pub tracked_only: Option<bool>,
    #[serde(default)]
    pub metadata_only: Option<bool>,
    #[serde(default)]
    pub file: Option<ReviewDiffFileArg>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBranchCommitsInput {
    pub cwd: String,
    #[serde(default)]
    pub base_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDiffFileContentsInput {
    pub cwd: String,
    pub source_kind: ReviewSourceKind,
    pub change_type: ReviewChangeType,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
    pub old_path: String,
    pub new_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBranchCommit {
    pub sha: String,
    pub subject: String,
    pub committed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBranchCommitsResult {
    pub commits: Vec<ReviewBranchCommit>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDiffFileStat {
    pub path: String,
    pub previous_path: Option<String>,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDiffPreviewSource {
    pub id: String,
    pub kind: ReviewSourceKind,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_subject: Option<String>,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
    pub diff: String,
    pub diff_hash: String,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patches_deferred: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<ReviewDiffFileStat>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDiffPreviewResult {
    pub cwd: String,
    pub generated_at: String,
    pub sources: Vec<ReviewDiffPreviewSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDiffFileContentsResult {
    pub old_contents: String,
    pub new_contents: String,
}

/// `isMissingGitCwdError`: the cwd disappeared between validation and spawn.
enum RunError {
    MissingCwd,
    Failed(String),
}

impl RunError {
    fn into_string(self) -> String {
        match self {
            RunError::MissingCwd => "Git working directory is missing.".into(),
            RunError::Failed(message) => message,
        }
    }
}

type R<T> = Result<T, RunError>;

fn failed<T>(message: String) -> R<T> {
    Err(RunError::Failed(message))
}

fn git_fail<T>(operation: &str, cwd: &Path, detail: &str) -> R<T> {
    Err(RunError::Failed(git_error(operation, cwd, "", detail)))
}

fn review_commit_error<T>(cwd: &Path, detail: &str) -> R<T> {
    git_fail("GitVcsDriver.reviewCommit", cwd, detail)
}

fn review_file_error<T>(cwd: &Path, detail: &str) -> R<T> {
    git_fail("GitVcsDriver.getReviewDiffFileContents", cwd, detail)
}

fn working_tree_file_error<T>(cwd: &Path, stage: &str, detail: &str) -> R<T> {
    git_fail(
        &format!("GitVcsDriver.getReviewDiffFileContents.workingTree.{stage}"),
        cwd,
        detail,
    )
}

struct GitResult {
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
}

impl GitResult {
    fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// KSG `executeGit`: bounded output that keeps the first `max_output` bytes and
/// flags truncation instead of failing when `allow_truncate` is set.
#[allow(clippy::too_many_arguments)]
fn run_git(
    operation: &str,
    cwd: &Path,
    args: &[String],
    timeout: Duration,
    max_output: usize,
    allow_truncate: bool,
    env: &[(&str, &str)],
    stdin: Option<Vec<u8>>,
    ctx: &GitContext,
) -> R<GitResult> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    configure_git_command(&mut cmd, cwd, ctx);
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut process = crate::process::Process::spawn(&mut cmd).map_err(|err| {
        if matches!(
            err.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
        ) && !cwd.is_dir()
        {
            RunError::MissingCwd
        } else {
            RunError::Failed(git_error(
                operation,
                cwd,
                &err.to_string(),
                "Failed to spawn Git process.",
            ))
        }
    })?;
    let stdout_pipe = process.stdout.take().expect("piped stdout");
    let stderr_pipe = process.stderr.take().expect("piped stderr");
    let out_thread = thread::spawn(move || capped_read(Box::new(stdout_pipe), max_output));
    let err_thread = thread::spawn(move || capped_read(Box::new(stderr_pipe), max_output));
    let stdin_thread = stdin.map(|data| {
        let mut pipe = process.stdin.take().expect("piped stdin");
        thread::spawn(move || {
            let _ = pipe.write_all(&data);
            let _ = pipe.flush();
        })
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match process.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(err) => {
                process.terminate();
                return failed(git_error(
                    operation,
                    cwd,
                    &err.to_string(),
                    "Failed to read Git process exit code.",
                ));
            }
        }
        if Instant::now() >= deadline {
            process.terminate();
            let _ = process.reap(Duration::from_secs(1));
            return git_fail(operation, cwd, "Git command timed out.");
        }
        thread::sleep(Duration::from_millis(10));
    };
    // Descendants must not keep the output pipes open.
    process.terminate();
    let stdout = crate::process::join_for(out_thread, Duration::from_secs(1));
    let stderr = crate::process::join_for(err_thread, Duration::from_secs(1));
    if let Some(writer) = stdin_thread {
        let _ = writer.join();
    }
    let (Some((stdout, stdout_truncated)), Some((stderr, stderr_truncated))) = (stdout, stderr)
    else {
        return git_fail(operation, cwd, "Failed to read Git process output.");
    };
    if !allow_truncate && (stdout_truncated || stderr_truncated) {
        return git_fail(
            operation,
            cwd,
            &format!("Git output exceeded {max_output} bytes and was truncated."),
        );
    }
    Ok(GitResult {
        exit_code: status.code(),
        stdout,
        stderr,
        stdout_truncated,
    })
}

/// KSG `executeGit` without `allowNonZeroExit`: a non-zero exit is an error.
#[allow(clippy::too_many_arguments)]
fn run_git_checked(
    operation: &str,
    cwd: &Path,
    args: &[String],
    timeout: Duration,
    max_output: usize,
    allow_truncate: bool,
    env: &[(&str, &str)],
    stdin: Option<Vec<u8>>,
    ctx: &GitContext,
) -> R<GitResult> {
    let result = run_git(
        operation,
        cwd,
        args,
        timeout,
        max_output,
        allow_truncate,
        env,
        stdin,
        ctx,
    )?;
    if result.exit_code == Some(0) {
        Ok(result)
    } else {
        failed(git_error(
            operation,
            cwd,
            &result.stderr_text(),
            "Git command exited with a non-zero status.",
        ))
    }
}

/// Drains a stream, keeping the first `cap` bytes and flagging truncation.
fn capped_read(mut pipe: Box<dyn Read + Send>, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buf = [0u8; 16 * 1024];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = cap.saturating_sub(kept.len());
                let take = n.min(room);
                if take < n {
                    truncated = true;
                }
                kept.extend_from_slice(&buf[..take]);
            }
        }
    }
    (kept, truncated)
}

fn str_args(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn is_commit_sha(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// `diffHash` is the SHA-256 of `JSON.stringify([diff, files])`, so the tuple
/// serializes the same way KSG's array does.
fn hash_diff(diff: &str, files: &[ReviewDiffFileStat]) -> String {
    let payload = serde_json::to_string(&(diff, files)).unwrap_or_default();
    format!("{:x}", Sha256::digest(payload.as_bytes()))
}

/// KSG `parseReviewNumstat`: `add\tdel\tpath`, or `add\tdel\t\0prev\0path` for renames.
fn parse_review_numstat(stdout: &str) -> Vec<ReviewDiffFileStat> {
    let fields: Vec<&str> = stdout.split('\0').collect();
    let mut files = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        let mut parts = field.splitn(3, '\t');
        if let (Some(add), Some(del), Some(rest)) = (parts.next(), parts.next(), parts.next())
            && (add == "-" || (!add.is_empty() && add.bytes().all(|b| b.is_ascii_digit())))
            && (del == "-" || (!del.is_empty() && del.bytes().all(|b| b.is_ascii_digit())))
        {
            let (previous_path, path) = if rest.is_empty() {
                index += 1;
                let previous = fields.get(index).copied().map(str::to_owned);
                index += 1;
                (
                    previous,
                    fields.get(index).copied().unwrap_or("").to_owned(),
                )
            } else {
                (None, rest.to_owned())
            };
            files.push(ReviewDiffFileStat {
                path,
                previous_path,
                additions: if add == "-" {
                    0
                } else {
                    add.parse().unwrap_or(0)
                },
                deletions: if del == "-" {
                    0
                } else {
                    del.parse().unwrap_or(0)
                },
            });
        }
        index += 1;
    }
    files
}

/// KSG `splitNullSeparatedGitStdoutPaths`: a truncated tail record is dropped.
fn split_nul_paths(text: &str, truncated: bool) -> Vec<String> {
    let mut parts: Vec<&str> = text.split('\0').collect();
    if truncated && parts.last().is_some_and(|part| !part.is_empty()) {
        parts.pop();
    }
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

struct ReviewRepository {
    worktree_root: Option<PathBuf>,
    current_branch: Option<String>,
}

/// KSG `readGitCommonDir`: null for a non-repository, an error otherwise.
fn read_git_common_dir(cwd: &Path, ctx: &GitContext) -> R<Option<PathBuf>> {
    let operation = "GitVcsDriver.resolveRepositoryPaths.commonDir";
    let out = run_git(
        operation,
        cwd,
        &str_args(&["rev-parse", "--git-common-dir"]),
        SHORT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    if out.exit_code != Some(0) {
        if is_non_repository_stderr(&out.stderr_text()) {
            return Ok(None);
        }
        return git_fail(
            operation,
            cwd,
            "Failed to resolve the Git common directory.",
        );
    }
    let raw = out.stdout_text().trim().to_owned();
    let physical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let resolved = if Path::new(&raw).is_absolute() {
        normalize_lexical(Path::new(&raw))
    } else {
        normalize_lexical(&physical_cwd.join(&raw))
    };
    Ok(Some(std::fs::canonicalize(&resolved).unwrap_or(resolved)))
}

/// KSG `readWorktreeRoot`: null on any non-zero exit or empty output.
fn read_worktree_root(cwd: &Path, ctx: &GitContext) -> R<Option<PathBuf>> {
    let out = run_git(
        "GitVcsDriver.resolveRepositoryPaths.worktreeRoot",
        cwd,
        &str_args(&["rev-parse", "--show-toplevel"]),
        SHORT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    let stdout = out.stdout_text();
    // Git terminates --show-toplevel with exactly one "\n" on every platform.
    let trimmed = stdout.strip_suffix('\n').unwrap_or(stdout.as_str());
    if out.exit_code == Some(0) && !trimmed.is_empty() {
        let path = Path::new(trimmed);
        Ok(Some(if path.is_absolute() {
            normalize_lexical(path)
        } else {
            normalize_lexical(&cwd.join(path))
        }))
    } else {
        Ok(None)
    }
}

/// KSG `resolveRepositoryPathsUncached`, minus its unused common-dir result.
fn resolve_repository_paths(cwd: &Path, ctx: &GitContext) -> R<Option<ReviewRepository>> {
    if read_git_common_dir(cwd, ctx)?.is_none() {
        return Ok(None);
    }
    let worktree_root = read_worktree_root(cwd, ctx)?;
    let branch_out = run_git(
        "GitVcsDriver.resolveRepositoryPaths.currentBranch",
        cwd,
        &str_args(&["symbolic-ref", "--quiet", "HEAD"]),
        SHORT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    let current_branch = if branch_out.exit_code == Some(0) {
        branch_name_from_head_ref(&branch_out.stdout_text())
    } else {
        None
    };
    Ok(Some(ReviewRepository {
        worktree_root,
        current_branch,
    }))
}

/// KSG `resolveReviewWorktreeRoot`: a found root skips full discovery.
fn resolve_review_worktree_root(cwd: &Path, ctx: &GitContext) -> R<Option<ReviewRepository>> {
    if let Some(root) = read_worktree_root(cwd, ctx)? {
        return Ok(Some(ReviewRepository {
            worktree_root: Some(root),
            current_branch: None,
        }));
    }
    resolve_repository_paths(cwd, ctx)
}

struct ReviewMergeBase {
    base_ref: Option<String>,
    merge_base: String,
}

/// KSG `resolveReviewMergeBase` with `knownOriginDefaultBranch` unset.
fn resolve_review_merge_base(
    cwd: &Path,
    branch: Option<&str>,
    explicit_base_ref: Option<&str>,
    ctx: &GitContext,
) -> R<ReviewMergeBase> {
    let base = match explicit_base_ref {
        Some(explicit) => Some(
            resolve_comparison_base_ref(cwd, explicit, ctx).unwrap_or_else(|| {
                let comparison =
                    if !explicit.starts_with("refs/") && branch_exists(cwd, explicit, ctx) {
                        format!("refs/heads/{explicit}")
                    } else {
                        explicit.to_owned()
                    };
                (explicit.to_owned(), comparison)
            }),
        ),
        None => {
            branch.and_then(|name| resolve_base_branch_for_no_upstream(cwd, name, None, true, ctx))
        }
    };
    let Some((base_ref, comparison_ref)) = base else {
        return Ok(ReviewMergeBase {
            base_ref: None,
            merge_base: "HEAD".into(),
        });
    };
    let result = run_git(
        "GitVcsDriver.resolveReviewMergeBase",
        cwd,
        &str_args(&["merge-base", "--end-of-options", &comparison_ref, "HEAD"]),
        DEFAULT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    let merge_base = result.stdout_text().trim().to_owned();
    if result.exit_code != Some(0) || merge_base.is_empty() {
        if explicit_base_ref.is_none() {
            let head = run_git(
                "GitVcsDriver.resolveReviewMergeBase.head",
                cwd,
                &str_args(&["rev-parse", "--verify", "--quiet", "HEAD"]),
                DEFAULT_TIMEOUT,
                DEFAULT_MAX_OUTPUT_BYTES,
                false,
                &[],
                None,
                ctx,
            )?;
            if head.exit_code != Some(0) {
                return Ok(ReviewMergeBase {
                    base_ref: None,
                    merge_base: "HEAD".into(),
                });
            }
        }
        return git_fail(
            "GitVcsDriver.resolveReviewMergeBase",
            cwd,
            &format!("Could not find a common commit between '{base_ref}' and HEAD."),
        );
    }
    Ok(ReviewMergeBase {
        base_ref: Some(base_ref),
        merge_base,
    })
}

#[cfg(windows)]
const EMPTY_TREE_INPUT: &str = "NUL";
#[cfg(not(windows))]
const EMPTY_TREE_INPUT: &str = "/dev/null";

/// KSG `readEmptyTreeHash`: computed so SHA-256 repositories work too.
fn read_empty_tree_hash(cwd: &Path, ctx: &GitContext) -> R<String> {
    let out = run_git_checked(
        "GitVcsDriver.review.emptyTree",
        cwd,
        &str_args(&["hash-object", "-t", "tree", EMPTY_TREE_INPUT]),
        DEFAULT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    Ok(out.stdout_text().trim().to_owned())
}

struct ResolvedCommit {
    sha: String,
    parent: String,
}

/// KSG `resolveReviewCommit`: `cat-file` checks keep lazy fetches off.
fn resolve_review_commit(cwd: &Path, sha: &str, ctx: &GitContext) -> R<ResolvedCommit> {
    if !is_commit_sha(sha) {
        return review_commit_error(cwd, "Selected commit is invalid or no longer available.");
    }
    let present = run_git(
        "GitVcsDriver.reviewCommit.present",
        cwd,
        &["cat-file".into(), "-e".into(), format!("{sha}^{{commit}}")],
        DEFAULT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[("GIT_NO_LAZY_FETCH", "1")],
        None,
        ctx,
    )?;
    if present.exit_code != Some(0) {
        return review_commit_error(cwd, "Selected commit is invalid or no longer available.");
    }
    // The raw object keeps the parents a shallow clone grafts away.
    let object = run_git_checked(
        "GitVcsDriver.reviewCommit.parents",
        cwd,
        &str_args(&["cat-file", "commit", sha]),
        DEFAULT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?
    .stdout_text();
    let first_parent = object
        .split('\n')
        .nth(1)
        .and_then(|line| line.strip_prefix("parent "));
    if let Some(parent) = first_parent {
        let check = run_git(
            "GitVcsDriver.reviewCommit.parent",
            cwd,
            &[
                "cat-file".into(),
                "-e".into(),
                format!("{parent}^{{commit}}"),
            ],
            DEFAULT_TIMEOUT,
            DEFAULT_MAX_OUTPUT_BYTES,
            false,
            &[("GIT_NO_LAZY_FETCH", "1")],
            None,
            ctx,
        )?;
        if check.exit_code != Some(0) {
            return review_commit_error(cwd, "Selected commit's parent is not available locally.");
        }
    }
    let parent = match first_parent {
        Some(parent) => parent.to_owned(),
        None => read_empty_tree_hash(cwd, ctx)?,
    };
    Ok(ResolvedCommit {
        sha: sha.to_lowercase(),
        parent,
    })
}

/// KSG `readReviewCommitMessage`: subject is the first body line, capped at 300.
fn read_review_commit_message(cwd: &Path, sha: &str, ctx: &GitContext) -> R<String> {
    let out = run_git_checked(
        "GitVcsDriver.readReviewCommitMessage",
        cwd,
        &str_args(&[
            "show",
            "-s",
            "--no-notes",
            "--no-show-signature",
            "--format=%B",
            sha,
        ]),
        DEFAULT_TIMEOUT,
        8_192,
        true,
        &[],
        None,
        ctx,
    )?;
    let text = out.stdout_text();
    Ok(text
        .split('\n')
        .next()
        .unwrap_or("")
        .chars()
        .take(300)
        .collect())
}

/// Temporary GIT_INDEX_FILE holding untracked files as intent-to-add, so
/// `git diff` reports them like KSG's `prepareReviewIndex`.
struct TempIndex {
    path: PathBuf,
}

impl TempIndex {
    fn new() -> R<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "shina-review-index-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        std::fs::File::create(&path)
            .map_err(|_| RunError::Failed("Could not prepare the review index.".to_owned()))?;
        Ok(Self { path })
    }
    fn env(&self) -> (String, String) {
        (
            "GIT_INDEX_FILE".to_owned(),
            self.path.to_string_lossy().into_owned(),
        )
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// KSG `prepareReviewIndex`. Untracked paths listed as staged deletions stay out.
fn prepare_review_index(
    cwd: &Path,
    untracked_paths: &[String],
    ctx: &GitContext,
) -> R<Option<TempIndex>> {
    let staged_out = run_git(
        "GitVcsDriver.readUnifiedWorkingTreeReviewDiff.stagedDeletions",
        cwd,
        &str_args(&[
            "diff",
            "--cached",
            "--no-renames",
            "--name-only",
            "--diff-filter=D",
            "-z",
            "HEAD",
            "--",
        ]),
        DEFAULT_TIMEOUT,
        REVIEW_METADATA_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    let staged: HashSet<String> = staged_out
        .stdout_text()
        .split('\0')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    let index_out = run_git_checked(
        "GitVcsDriver.readUnifiedWorkingTreeReviewDiff.indexPath",
        cwd,
        &str_args(&["rev-parse", "--git-path", "index"]),
        DEFAULT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    let index_rel = index_out.stdout_text().trim().to_owned();
    let index_path = if Path::new(&index_rel).is_absolute() {
        PathBuf::from(index_rel)
    } else {
        cwd.join(index_rel)
    };
    let paths_to_add: Vec<String> = untracked_paths
        .iter()
        .filter(|path| !staged.contains(*path))
        .cloned()
        .collect();
    if paths_to_add.is_empty() {
        return Ok(None);
    }
    let temp = TempIndex::new()?;
    let env = temp.env();
    let env_pairs = [(env.0.as_str(), env.1.as_str())];
    if index_path.exists() {
        std::fs::copy(&index_path, &temp.path)
            .map_err(|_| RunError::Failed("Could not prepare the review index.".into()))?;
        // Seconds are floored so a copied index is never newer than the real one.
        if let Ok(modified) = std::fs::metadata(&index_path).and_then(|m| m.modified())
            && let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            let floored = std::time::UNIX_EPOCH + Duration::from_secs(since_epoch.as_secs());
            let times = std::fs::FileTimes::new()
                .set_accessed(floored)
                .set_modified(floored);
            if let Ok(file) = std::fs::File::options().write(true).open(&temp.path) {
                let _ = file.set_times(times);
            }
        }
    } else {
        run_git_checked(
            "GitVcsDriver.review.emptyIndex",
            cwd,
            &str_args(&["read-tree", "--empty"]),
            DEFAULT_TIMEOUT,
            DEFAULT_MAX_OUTPUT_BYTES,
            false,
            &env_pairs,
            None,
            ctx,
        )?;
    }
    run_git_checked(
        "GitVcsDriver.readUnifiedWorkingTreeReviewDiff.expandSplitIndex",
        cwd,
        &str_args(&[
            "-c",
            "core.splitIndex=false",
            "-c",
            "splitIndex.sharedIndexExpire=never",
            "update-index",
            "--no-split-index",
        ]),
        DEFAULT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &env_pairs,
        None,
        ctx,
    )?;
    let stdin = format!("{}\0", paths_to_add.join("\0")).into_bytes();
    run_git_checked(
        "GitVcsDriver.readUnifiedWorkingTreeReviewDiff.addUntracked",
        cwd,
        &str_args(&[
            "-c",
            "core.splitIndex=false",
            "-c",
            "splitIndex.sharedIndexExpire=never",
            "-c",
            "core.preloadIndex=false",
            "-c",
            "core.sparseCheckout=false",
            "--literal-pathspecs",
            "add",
            "--intent-to-add",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ]),
        DEFAULT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &env_pairs,
        Some(stdin),
        ctx,
    )?;
    Ok(Some(temp))
}

/// `unavailable` keeps the review's untracked totals out like KSG's null result.
enum UntrackedPrep {
    Unavailable,
    Ready(Option<TempIndex>),
}

/// KSG `prepareUntrackedReviewIndex`: listing failures and >5000 files are
/// unavailable; index-prep failures are errors.
fn prepare_untracked_index(
    cwd: &Path,
    path_args: &[String],
    only_path: Option<&str>,
    ctx: &GitContext,
) -> R<UntrackedPrep> {
    let mut args = str_args(&["ls-files", "--others", "--exclude-standard", "-z", "--"]);
    args.extend(path_args.iter().cloned());
    let untracked = run_git(
        "GitVcsDriver.review.listUntracked",
        cwd,
        &args,
        DEFAULT_TIMEOUT,
        REVIEW_METADATA_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    );
    let out = match untracked {
        Ok(out) if out.exit_code == Some(0) => out,
        _ => return Ok(UntrackedPrep::Unavailable),
    };
    // ls-files lists a nested repository with a trailing slash; its diff path has none.
    let paths: Vec<String> = split_nul_paths(&out.stdout_text(), out.stdout_truncated)
        .into_iter()
        .filter(|candidate| {
            only_path
                .is_none_or(|path| candidate.as_str() == path || candidate == &format!("{path}/"))
        })
        .collect();
    if paths.is_empty() {
        return Ok(UntrackedPrep::Ready(None));
    }
    if paths.len() > REVIEW_UNTRACKED_MAX_FILES {
        return Ok(UntrackedPrep::Unavailable);
    }
    Ok(UntrackedPrep::Ready(prepare_review_index(
        cwd, &paths, ctx,
    )?))
}

#[derive(Clone)]
struct TrackedDiff {
    stdout: String,
    truncated: bool,
    files: Option<Vec<ReviewDiffFileStat>>,
}

/// KSG `getReviewBranchCommits`.
fn get_branch_commits_impl(
    input: &ReviewBranchCommitsInput,
    ctx: &GitContext,
) -> R<ReviewBranchCommitsResult> {
    let empty = || ReviewBranchCommitsResult {
        commits: Vec::new(),
        truncated: false,
    };
    let cwd = validate_existing_directory(&input.cwd, "cwd").map_err(RunError::Failed)?;
    let Some(repository) = resolve_repository_paths(&cwd, ctx)? else {
        return Ok(empty());
    };
    let Some(cwd) = repository.worktree_root else {
        return Ok(empty());
    };
    let head = run_git(
        "GitVcsDriver.getReviewBranchCommits.head",
        &cwd,
        &str_args(&["rev-parse", "--verify", "--quiet", "HEAD"]),
        DEFAULT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    if head.exit_code != Some(0) {
        if head.exit_code == Some(1) && head.stderr_text().trim().is_empty() {
            return Ok(empty());
        }
        return review_commit_error(&cwd, "Could not resolve the current branch commit.");
    }
    let base = resolve_review_merge_base(
        &cwd,
        repository.current_branch.as_deref(),
        input
            .base_ref
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        ctx,
    )?;
    let result = run_git_checked(
        "GitVcsDriver.getReviewBranchCommits",
        &cwd,
        &[
            "log".into(),
            "-z".into(),
            "--no-notes".into(),
            "--no-show-signature".into(),
            "--format=%H%x00%s%x00%cI".into(),
            format!("--max-count={}", REVIEW_BRANCH_COMMIT_LIMIT + 1),
            format!("{}..HEAD", base.merge_base),
        ],
        DEFAULT_TIMEOUT,
        REVIEW_BRANCH_COMMIT_MAX_OUTPUT_BYTES,
        true,
        &[],
        None,
        ctx,
    )?;
    let stdout = result.stdout_text();
    if !result.stdout_truncated && !stdout.is_empty() && !stdout.ends_with('\0') {
        return review_commit_error(&cwd, "Could not read branch commit metadata.");
    }
    let fields: Vec<&str> = stdout.split('\0').collect();
    let complete_field_count = fields.len().saturating_sub(1);
    let mut commits: Vec<ReviewBranchCommit> = Vec::new();
    let mut index = 0;
    while index + 2 < complete_field_count && commits.len() <= REVIEW_BRANCH_COMMIT_LIMIT {
        let sha = fields[index];
        let subject = fields[index + 1];
        let committed_at = fields[index + 2];
        if sha.is_empty() || committed_at.is_empty() || !is_commit_sha(sha) {
            return review_commit_error(&cwd, "Could not read branch commit metadata.");
        }
        commits.push(ReviewBranchCommit {
            sha: sha.to_owned(),
            subject: subject.chars().take(REVIEW_BRANCH_SUBJECT_LIMIT).collect(),
            committed_at: committed_at.to_owned(),
        });
        index += 3;
    }
    if result.stdout_truncated && commits.is_empty() {
        return review_commit_error(&cwd, "Branch commit metadata exceeds the response limit.");
    }
    let truncated = result.stdout_truncated || commits.len() > REVIEW_BRANCH_COMMIT_LIMIT;
    commits.truncate(REVIEW_BRANCH_COMMIT_LIMIT);
    Ok(ReviewBranchCommitsResult { commits, truncated })
}

/// KSG `getReviewDiffPreview`.
fn get_diff_preview_impl(
    input: &ReviewDiffPreviewInput,
    ctx: &GitContext,
) -> R<ReviewDiffPreviewResult> {
    let cwd = validate_existing_directory(&input.cwd, "cwd").map_err(RunError::Failed)?;
    let result_cwd = input.cwd.trim().to_owned();
    let empty_result = |sources: Vec<ReviewDiffPreviewSource>| ReviewDiffPreviewResult {
        cwd: result_cwd.clone(),
        generated_at: utc_now(),
        sources,
        incomplete: None,
    };
    let path_args: Vec<String> = input
        .file
        .iter()
        .flat_map(|file| {
            std::iter::once(file.path.clone()).chain(
                file.previous_path
                    .iter()
                    .filter(|path| !path.is_empty())
                    .cloned(),
            )
        })
        .map(|path| format!(":(top,literal){path}"))
        .collect();
    let patch_limit = if input.file.is_some() {
        REVIEW_DIFF_FILE_MAX_OUTPUT_BYTES
    } else {
        REVIEW_DIFF_PATCH_MAX_OUTPUT_BYTES
    };
    let source_kind = input
        .file
        .as_ref()
        .map(|file| file.source_kind)
        .or_else(|| input.source_kind.map(ReviewSourceKind::from));
    let needs_current_branch = input.commit_sha.is_none()
        && input.file.as_ref().map(|file| file.source_kind) != Some(ReviewSourceKind::Commit)
        && source_kind != Some(ReviewSourceKind::WorkingTree);
    let repository = match if needs_current_branch {
        resolve_repository_paths(&cwd, ctx)
    } else {
        resolve_review_worktree_root(&cwd, ctx)
    } {
        Err(RunError::MissingCwd) => None,
        other => other?,
    };
    let Some(cwd) = repository
        .as_ref()
        .and_then(|repo| repo.worktree_root.clone())
    else {
        return Ok(empty_result(Vec::new()));
    };

    if input.commit_sha.is_some()
        || input.file.as_ref().map(|f| f.source_kind) == Some(ReviewSourceKind::Commit)
    {
        let commit_sha = input.commit_sha.as_deref().map(str::trim).unwrap_or("");
        if commit_sha.is_empty()
            || input
                .file
                .as_ref()
                .is_some_and(|file| file.source_kind != ReviewSourceKind::Commit)
        {
            return review_commit_error(&cwd, "Selected commit request is invalid.");
        }
        let commit = resolve_review_commit(&cwd, commit_sha, ctx)?;
        let subject = read_review_commit_message(&cwd, &commit.sha, ctx)?;
        let mut diff_args: Vec<String> = REVIEW_DIFF_ARGS.iter().map(|a| (*a).to_owned()).collect();
        if input.ignore_whitespace == Some(true) {
            diff_args.push("--ignore-all-space".into());
        }
        let mut stat_args = diff_args.clone();
        stat_args.extend(str_args(&[
            "--numstat",
            "-z",
            &commit.parent,
            &commit.sha,
            "--",
        ]));
        stat_args.extend(path_args.iter().cloned());
        let stat = run_git_checked(
            "GitVcsDriver.getReviewDiffPreview.commitStat",
            &cwd,
            &stat_args,
            DEFAULT_TIMEOUT,
            REVIEW_METADATA_MAX_OUTPUT_BYTES,
            false,
            &[],
            None,
            ctx,
        )?;
        let files = parse_review_numstat(&stat.stdout_text());
        let patch = if input.metadata_only == Some(true) && input.file.is_none() {
            (String::new(), false)
        } else {
            let mut patch_args = diff_args;
            patch_args.extend(str_args(&["--patch", &commit.parent, &commit.sha, "--"]));
            patch_args.extend(path_args.iter().cloned());
            let out = run_git_checked(
                "GitVcsDriver.getReviewDiffPreview.commitPatch",
                &cwd,
                &patch_args,
                DEFAULT_TIMEOUT,
                patch_limit,
                true,
                &[],
                None,
                ctx,
            )?;
            (out.stdout_text(), out.stdout_truncated)
        };
        let diff_hash = hash_diff(&patch.0, &files);
        return Ok(empty_result(vec![ReviewDiffPreviewSource {
            id: commit.sha.clone(),
            kind: ReviewSourceKind::Commit,
            title: format!("Commit {}", &commit.sha[..7]),
            commit_subject: {
                let trimmed = subject.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            },
            base_ref: Some(commit.parent),
            head_ref: Some(commit.sha),
            diff: patch.0,
            diff_hash,
            truncated: patch.1,
            patches_deferred: (input.metadata_only == Some(true) && input.file.is_none())
                .then_some(true),
            files: Some(files),
        }]));
    }

    let dirty_ref: Option<&str> = if source_kind == Some(ReviewSourceKind::BranchRange) {
        None
    } else {
        Some("HEAD")
    };
    let (review_base_ref, review_merge_base) = if source_kind == Some(ReviewSourceKind::WorkingTree)
    {
        (
            input
                .base_ref
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            None,
        )
    } else {
        let resolved = resolve_review_merge_base(
            &cwd,
            repository
                .as_ref()
                .and_then(|repo| repo.current_branch.as_deref()),
            input
                .base_ref
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            ctx,
        )?;
        (resolved.base_ref, Some(resolved.merge_base))
    };

    let mut diff_args: Vec<String> = REVIEW_DIFF_ARGS.iter().map(|a| (*a).to_owned()).collect();
    diff_args.push("--minimal".into());
    if input.ignore_whitespace == Some(true) {
        diff_args.push("--ignore-all-space".into());
    }

    let untracked = if input.tracked_only == Some(true) {
        UntrackedPrep::Ready(None)
    } else {
        prepare_untracked_index(
            &cwd,
            &path_args,
            input.file.as_ref().map(|file| file.path.as_str()),
            ctx,
        )?
    };
    let untracked_unavailable = matches!(untracked, UntrackedPrep::Unavailable);
    let metadata_only =
        input.metadata_only == Some(true) && input.file.is_none() && !untracked_unavailable;
    let index_env = match &untracked {
        UntrackedPrep::Ready(Some(index)) => Some(index.env()),
        _ => None,
    };
    let env: Vec<(&str, &str)> = index_env
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();

    let read_stats = |reference: &str,
                      stat_env: &[(&str, &str)],
                      diff_args: &[String]|
     -> R<(String, Vec<ReviewDiffFileStat>)> {
        let mut args = diff_args.to_vec();
        args.extend(str_args(&["--numstat", "-z", reference, "--"]));
        args.extend(path_args.iter().cloned());
        let result = run_git(
            "GitVcsDriver.getReviewDiffPreview.stat",
            &cwd,
            &args,
            DEFAULT_TIMEOUT,
            REVIEW_METADATA_MAX_OUTPUT_BYTES,
            false,
            stat_env,
            None,
            ctx,
        )?;
        if result.exit_code == Some(0) {
            return Ok((
                reference.to_owned(),
                parse_review_numstat(&result.stdout_text()),
            ));
        }
        if reference == "HEAD" && is_unborn_head_stderr(&result.stderr_text()) {
            let empty_tree = read_empty_tree_hash(&cwd, ctx)?;
            let mut args = diff_args.to_vec();
            args.extend(str_args(&["--numstat", "-z", &empty_tree, "--"]));
            args.extend(path_args.iter().cloned());
            let stdout = run_git_checked(
                "GitVcsDriver.getReviewDiffPreview.unbornStat",
                &cwd,
                &args,
                DEFAULT_TIMEOUT,
                REVIEW_METADATA_MAX_OUTPUT_BYTES,
                false,
                stat_env,
                None,
                ctx,
            )?;
            return Ok((empty_tree, parse_review_numstat(&stdout.stdout_text())));
        }
        git_fail(
            "GitVcsDriver.getReviewDiffPreview.stat",
            &cwd,
            "Could not read complete diff statistics.",
        )
    };

    let read_tracked_diff = |reference: Option<&str>,
                             diff_env: &[(&str, &str)],
                             metadata_only: bool|
     -> R<TrackedDiff> {
        let Some(reference) = reference else {
            return Ok(TrackedDiff {
                stdout: String::new(),
                truncated: false,
                files: Some(Vec::new()),
            });
        };
        let (used_ref, files) = read_stats(reference, diff_env, &diff_args)?;
        if metadata_only || files.is_empty() {
            return Ok(TrackedDiff {
                stdout: String::new(),
                truncated: false,
                files: Some(files),
            });
        }
        let mut args = diff_args.clone();
        args.extend(str_args(&["--patch", &used_ref, "--"]));
        args.extend(path_args.iter().cloned());
        let patch = run_git_checked(
            "GitVcsDriver.getReviewDiffPreview.patch",
            &cwd,
            &args,
            DEFAULT_TIMEOUT,
            patch_limit,
            true,
            diff_env,
            None,
            ctx,
        )?;
        Ok(TrackedDiff {
            stdout: patch.stdout_text(),
            truncated: patch.stdout_truncated,
            files: Some(files),
        })
    };

    let (dirty, base) = if review_merge_base.as_deref() == dirty_ref {
        let dirty = read_tracked_diff(dirty_ref, &env, metadata_only)?;
        (dirty.clone(), dirty)
    } else {
        (
            read_tracked_diff(dirty_ref, &env, metadata_only)?,
            read_tracked_diff(review_merge_base.as_deref(), &env, metadata_only)?,
        )
    };
    let incomplete = |reference: Option<&str>, result: TrackedDiff| {
        if untracked_unavailable && reference.is_some() {
            TrackedDiff {
                stdout: result.stdout,
                truncated: true,
                files: None,
            }
        } else {
            result
        }
    };
    let dirty = incomplete(dirty_ref, dirty);
    let base = incomplete(review_merge_base.as_deref(), base);

    let dirty_hash = hash_diff(&dirty.stdout, dirty.files.as_deref().unwrap_or(&[]));
    let base_hash = hash_diff(&base.stdout, base.files.as_deref().unwrap_or(&[]));
    let patches_deferred = |files: &Option<Vec<ReviewDiffFileStat>>| -> Option<bool> {
        (input.metadata_only == Some(true) && input.file.is_none() && files.is_some())
            .then_some(true)
    };

    let mut sources = vec![
        ReviewDiffPreviewSource {
            id: "working-tree".into(),
            kind: ReviewSourceKind::WorkingTree,
            title: "Uncommitted".into(),
            commit_subject: None,
            base_ref: Some("HEAD".into()),
            head_ref: None,
            diff: dirty.stdout,
            diff_hash: dirty_hash,
            truncated: dirty.truncated,
            patches_deferred: patches_deferred(&dirty.files),
            files: dirty.files,
        },
        ReviewDiffPreviewSource {
            id: "branch-range".into(),
            kind: ReviewSourceKind::BranchRange,
            title: match &review_base_ref {
                Some(base_ref) => format!("Changes vs {base_ref}"),
                None => "Changes".into(),
            },
            commit_subject: None,
            base_ref: review_base_ref,
            head_ref: Some(
                repository
                    .as_ref()
                    .and_then(|repo| repo.current_branch.clone())
                    .unwrap_or_else(|| "HEAD".into()),
            ),
            diff: base.stdout,
            diff_hash: base_hash,
            truncated: base.truncated,
            patches_deferred: patches_deferred(&base.files),
            files: base.files,
        },
    ];
    if let Some(kind) = input.source_kind {
        sources.retain(|source| source.kind == ReviewSourceKind::from(kind));
    }
    let mut result = empty_result(sources);
    if input.tracked_only == Some(true) {
        result.incomplete = Some(true);
    }
    Ok(result)
}

/// KSG `readReviewFileAtRevision`: a NUL byte means binary.
fn read_file_at_revision(cwd: &Path, revision: &str, path: &str, ctx: &GitContext) -> R<String> {
    let out = run_git_checked(
        "GitVcsDriver.getReviewDiffFileContents.revision",
        cwd,
        &[
            "show".into(),
            "--end-of-options".into(),
            format!("{revision}:{path}"),
        ],
        DEFAULT_TIMEOUT,
        REVIEW_DIFF_FILE_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    if out.stdout.contains(&0) {
        return review_file_error(cwd, &format!("Cannot expand binary file '{path}'."));
    }
    Ok(out.stdout_text())
}

fn is_within_root(root: &Path, candidate: &Path) -> bool {
    candidate.strip_prefix(root).is_ok()
}

/// KSG `isServerInternalPath` against Shina's state dir layout.
fn is_server_internal_path(real_target: &Path, state_dir: &Path) -> bool {
    let real_state = std::fs::canonicalize(state_dir).unwrap_or_else(|_| state_dir.to_path_buf());
    let name = real_target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    (real_target.parent() == Some(real_state.as_path()) && name.contains(".sqlite"))
        || (real_target.starts_with(&real_state)
            && SERVER_STATE_FILE_NAMES.contains(&name.as_str()))
        || real_target.starts_with("/dev/fd")
}

/// KSG `readWorkingTreeReviewFile`: lexical and symlink containment, file
/// type, 1 MB size bound, and the NUL binary check.
fn read_working_tree_file(
    input: &ReviewDiffFileContentsInput,
    cwd: &Path,
    repository_root: &Path,
    state_dir: &Path,
) -> R<String> {
    let repository_root = normalize_lexical(repository_root);
    let requested = normalize_lexical(&repository_root.join(input.new_path.trim()));
    if !is_within_root(&repository_root, &requested) {
        return working_tree_file_error(
            cwd,
            "path.resolve",
            &format!(
                "Diff file '{}' resolves outside the review workspace.",
                input.new_path.trim()
            ),
        );
    }
    let canonical = |path: &Path| {
        std::fs::canonicalize(path).map_err(|_| {
            RunError::Failed(git_error(
                "GitVcsDriver.getReviewDiffFileContents.workingTree.fs.realPath",
                cwd,
                "",
                &format!("Could not resolve diff file '{}'.", input.new_path.trim()),
            ))
        })
    };
    let real_root = canonical(&repository_root)?;
    let real_target = canonical(&requested)?;
    if !is_within_root(&real_root, &real_target) {
        return working_tree_file_error(
            cwd,
            "fs.realPath",
            &format!(
                "Diff file '{}' resolves outside the review workspace.",
                input.new_path.trim()
            ),
        );
    }
    if is_server_internal_path(&real_target, state_dir) {
        return working_tree_file_error(
            cwd,
            "fs.realPath",
            &format!("Diff file '{}' is not available.", input.new_path.trim()),
        );
    }
    let info = std::fs::metadata(&real_target).map_err(|_| {
        RunError::Failed(git_error(
            "GitVcsDriver.getReviewDiffFileContents.workingTree.fs.stat",
            cwd,
            "",
            &format!("Could not inspect diff file '{}'.", input.new_path.trim()),
        ))
    })?;
    if !info.is_file() {
        return working_tree_file_error(
            cwd,
            "fs.stat",
            &format!("Diff path '{}' is not a file.", input.new_path.trim()),
        );
    }
    if info.len() > REVIEW_DIFF_FILE_MAX_OUTPUT_BYTES as u64 {
        return working_tree_file_error(
            cwd,
            "fs.stat",
            &format!(
                "Diff file '{}' exceeds the 1 MB expansion limit.",
                input.new_path.trim()
            ),
        );
    }
    let bytes = std::fs::read(&real_target).map_err(|_| {
        RunError::Failed(git_error(
            "GitVcsDriver.getReviewDiffFileContents.workingTree.fs.readFile",
            cwd,
            "",
            &format!("Could not read diff file '{}'.", input.new_path.trim()),
        ))
    })?;
    if bytes.contains(&0) {
        return working_tree_file_error(
            cwd,
            "fs.readFile",
            &format!("Cannot expand binary file '{}'.", input.new_path.trim()),
        );
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// KSG `getReviewDiffFileContents`. `state_dir` is the server's data directory;
/// it replaces KSG's config.stateDir/secretsDir pair.
fn get_diff_file_contents_impl(
    input: &ReviewDiffFileContentsInput,
    ctx: &GitContext,
    state_dir: &Path,
) -> R<ReviewDiffFileContentsResult> {
    let cwd = validate_existing_directory(&input.cwd, "cwd").map_err(RunError::Failed)?;
    let root = run_git(
        "GitVcsDriver.getReviewDiffFileContents.repositoryRoot",
        &cwd,
        &str_args(&["rev-parse", "--show-toplevel"]),
        DEFAULT_TIMEOUT,
        DEFAULT_MAX_OUTPUT_BYTES,
        false,
        &[],
        None,
        ctx,
    )?;
    if root.exit_code != Some(0) {
        if is_non_repository_stderr(&root.stderr_text()) {
            return failed("Unchanged diff expansion currently requires a Git repository.".into());
        }
        return failed(git_error(
            "GitVcsDriver.getReviewDiffFileContents.repositoryRoot",
            &cwd,
            &root.stderr_text(),
            "Git command exited with a non-zero status.",
        ));
    }
    let root_text = root.stdout_text();
    let repository_root = {
        let trimmed = root_text.trim();
        if trimmed.is_empty() {
            return review_file_error(&cwd, "Could not resolve the Git repository root.");
        }
        let path = Path::new(trimmed);
        if path.is_absolute() {
            normalize_lexical(path)
        } else {
            normalize_lexical(&cwd.join(path))
        }
    };

    if input.source_kind == ReviewSourceKind::Commit {
        let Some(head_ref) = input
            .head_ref
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return review_file_error(&cwd, "Selected commit is no longer available.");
        };
        let commit = resolve_review_commit(&cwd, head_ref, ctx)?;
        if input.base_ref.as_deref() != Some(commit.parent.as_str()) {
            return review_file_error(&cwd, "Selected commit changed. Refresh the comparison.");
        }
        let old_contents = if input.change_type == ReviewChangeType::New {
            String::new()
        } else {
            read_file_at_revision(&cwd, &commit.parent, input.old_path.trim(), ctx)?
        };
        let new_contents = if input.change_type == ReviewChangeType::Deleted {
            String::new()
        } else {
            read_file_at_revision(&cwd, &commit.sha, input.new_path.trim(), ctx)?
        };
        return Ok(ReviewDiffFileContentsResult {
            old_contents,
            new_contents,
        });
    }

    let old_revision = if input.source_kind == ReviewSourceKind::WorkingTree {
        input
            .base_ref
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("HEAD")
            .to_owned()
    } else {
        resolve_review_merge_base(
            &cwd,
            None,
            input
                .base_ref
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            ctx,
        )?
        .merge_base
    };
    let old_contents = if input.change_type == ReviewChangeType::New {
        String::new()
    } else {
        read_file_at_revision(&cwd, &old_revision, input.old_path.trim(), ctx)?
    };
    let new_contents = if input.change_type == ReviewChangeType::Deleted {
        String::new()
    } else {
        read_working_tree_file(input, &cwd, &repository_root, state_dir)?
    };
    Ok(ReviewDiffFileContentsResult {
        old_contents,
        new_contents,
    })
}

/// KSG `ReviewService.assertWorkspaceBoundCwd`: the canonicalized `cwd` must sit
/// inside a known project root or a worktree registered to one. `resolve_cwd`'s
/// project-root check runs first so its basic path errors survive.
pub fn resolve_workspace_cwd(cwd: &str, project_roots: &[PathBuf]) -> Result<PathBuf, String> {
    if let Ok(cwd) = crate::workspace_search::resolve_cwd(cwd, project_roots) {
        return Ok(cwd);
    }
    let trimmed = cwd.trim();
    let path = Path::new(trimmed);
    if trimmed.is_empty() || !path.is_absolute() {
        return Err("Workspace path must be absolute.".into());
    }
    if !path.is_dir() {
        return Err("Workspace path is not a directory.".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "Workspace path could not be resolved.".to_string())?;
    for root in project_roots {
        let Ok(listed) = crate::vcs::list_worktrees(&crate::vcs::VcsListWorktreesInput {
            cwd: root.to_string_lossy().into_owned(),
        }) else {
            continue;
        };
        if !listed.is_repo {
            continue;
        }
        let inside = listed.worktrees.iter().any(|entry| {
            let worktree = crate::vcs::real_location(Path::new(&entry.path));
            canonical == worktree || canonical.starts_with(&worktree)
        });
        if inside {
            return Ok(canonical);
        }
    }
    Err("Workspace path is not inside a project on this server.".into())
}

pub fn get_branch_commits(
    input: &ReviewBranchCommitsInput,
    ctx: &GitContext,
) -> Result<ReviewBranchCommitsResult, String> {
    get_branch_commits_impl(input, ctx).map_err(RunError::into_string)
}

pub fn get_diff_preview(
    input: &ReviewDiffPreviewInput,
    ctx: &GitContext,
) -> Result<ReviewDiffPreviewResult, String> {
    get_diff_preview_impl(input, ctx).map_err(RunError::into_string)
}

pub fn get_diff_file_contents(
    input: &ReviewDiffFileContentsInput,
    ctx: &GitContext,
    state_dir: &Path,
) -> Result<ReviewDiffFileContentsResult, String> {
    get_diff_file_contents_impl(input, ctx, state_dir).map_err(RunError::into_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new(label: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("shina-review-{label}-{}", uuid::Uuid::new_v4()));
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
            let mut cmd = StdCommand::new("git");
            cmd.arg("-c")
                .arg("user.name=Test User")
                .arg("-c")
                .arg("user.email=test@example.com")
                .args(args)
                .current_dir(cwd)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_CONFIG_GLOBAL", &self.empty_config)
                .env("GIT_CONFIG_SYSTEM", &self.empty_config);
            let out = crate::process::output(&mut cmd, Duration::from_secs(15), 16 * 1024 * 1024)
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
        fn state_dir(&self) -> PathBuf {
            self._root.path.join("state")
        }
    }

    macro_rules! require_git {
        () => {
            if !crate::vcs::git_available() {
                eprintln!("skipping test: git is not on PATH");
                return;
            }
        };
    }

    fn preview_input(repo: &Path) -> ReviewDiffPreviewInput {
        ReviewDiffPreviewInput {
            cwd: repo.to_string_lossy().into_owned(),
            base_ref: None,
            commit_sha: None,
            ignore_whitespace: None,
            source_kind: None,
            tracked_only: None,
            metadata_only: None,
            file: None,
        }
    }

    fn contents_input(
        repo: &Path,
        change: ReviewChangeType,
        path: &str,
    ) -> ReviewDiffFileContentsInput {
        ReviewDiffFileContentsInput {
            cwd: repo.to_string_lossy().into_owned(),
            source_kind: ReviewSourceKind::WorkingTree,
            change_type: change,
            base_ref: None,
            head_ref: None,
            old_path: path.to_owned(),
            new_path: path.to_owned(),
        }
    }

    fn file_names(result: &ReviewDiffPreviewResult, index: usize) -> Vec<String> {
        result.sources[index]
            .files
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|file| file.path.clone())
            .collect()
    }

    #[test]
    fn working_tree_diff_covers_all_change_kinds() {
        require_git!();
        let env = TestEnv::new("kinds");
        let repo = env.init_repo("repo");
        std::fs::write(repo.join("modified.txt"), "one\n").unwrap();
        std::fs::write(repo.join("deleted.txt"), "gone\n").unwrap();
        std::fs::write(repo.join("renamed.txt"), "rename me\n").unwrap();
        std::fs::write(repo.join("binary.bin"), [0u8, 159, 146, 150]).unwrap();
        env.git(&repo, &["add", "."]);
        env.git(&repo, &["commit", "-m", "seed", "--quiet"]);

        std::fs::write(repo.join("modified.txt"), "one\ntwo\n").unwrap();
        std::fs::remove_file(repo.join("deleted.txt")).unwrap();
        env.git(&repo, &["mv", "renamed.txt", "renamed-new.txt"]);
        std::fs::write(repo.join("staged.txt"), "staged\n").unwrap();
        env.git(&repo, &["add", "staged.txt"]);
        std::fs::write(repo.join("untracked.txt"), "untracked\n").unwrap();
        std::fs::write(repo.join("untracked.bin"), [0u8, 1, 2, 3]).unwrap();
        std::fs::write(repo.join("binary.bin"), [0u8, 9, 9, 9]).unwrap();

        let result = get_diff_preview(&preview_input(&repo), &env.ctx).unwrap();
        assert_eq!(result.sources.len(), 2);
        let source = &result.sources[0];
        assert_eq!(source.id, "working-tree");
        assert_eq!(source.kind, ReviewSourceKind::WorkingTree);
        assert_eq!(source.title, "Uncommitted");
        assert!(!source.truncated);
        let files = file_names(&result, 0);
        for expected in [
            "modified.txt",
            "deleted.txt",
            "renamed-new.txt",
            "staged.txt",
            "untracked.txt",
            "untracked.bin",
            "binary.bin",
        ] {
            assert!(
                files.iter().any(|f| f == expected),
                "missing {expected}: {files:?}"
            );
        }
        let renamed = source
            .files
            .as_ref()
            .unwrap()
            .iter()
            .find(|f| f.path == "renamed-new.txt")
            .expect("rename entry");
        assert_eq!(renamed.previous_path.as_deref(), Some("renamed.txt"));
        assert!(source.diff.contains("deleted.txt"));
        assert!(source.diff.contains("untracked"));
        // With no configured base, branch-range resolves HEAD and matches the dirty diff.
        assert_eq!(result.sources[1].id, "branch-range");
        assert_eq!(result.sources[1].base_ref, None);
        assert_eq!(result.sources[1].diff, source.diff);
    }

    #[test]
    fn branch_range_diffs_and_lists_commits_against_base() {
        require_git!();
        let env = TestEnv::new("range");
        let repo = env.init_repo("repo");
        env.git(&repo, &["checkout", "-b", "feature", "--quiet"]);
        for index in 1..=3 {
            std::fs::write(
                repo.join(format!("feature-{index}.txt")),
                format!("{index}\n"),
            )
            .unwrap();
            env.git(&repo, &["add", "."]);
            env.git(
                &repo,
                &[
                    "commit",
                    "-m",
                    &format!("feature commit {index}"),
                    "--quiet",
                ],
            );
        }

        let commits = get_branch_commits(
            &ReviewBranchCommitsInput {
                cwd: repo.to_string_lossy().into_owned(),
                base_ref: None,
            },
            &env.ctx,
        )
        .unwrap();
        assert_eq!(commits.commits.len(), 3);
        assert!(!commits.truncated);
        assert_eq!(commits.commits[0].subject, "feature commit 3");
        assert_eq!(commits.commits[2].subject, "feature commit 1");
        assert!(commits.commits.iter().all(|c| is_commit_sha(&c.sha)));

        let mut input = preview_input(&repo);
        input.source_kind = Some(ReviewPreviewSourceKind::BranchRange);
        let result = get_diff_preview(&input, &env.ctx).unwrap();
        assert_eq!(result.sources.len(), 1);
        let source = &result.sources[0];
        assert_eq!(source.kind, ReviewSourceKind::BranchRange);
        assert_eq!(source.base_ref.as_deref(), Some("main"));
        assert_eq!(source.title, "Changes vs main");
        for index in 1..=3 {
            assert!(source.diff.contains(&format!("feature-{index}.txt")));
        }
        assert_eq!(file_names(&result, 0).len(), 3);
    }

    #[test]
    fn branch_commits_truncates_past_the_commit_limit() {
        require_git!();
        let env = TestEnv::new("commit-limit");
        let repo = env.init_repo("repo");
        env.git(&repo, &["checkout", "-b", "feature", "--quiet"]);
        for index in 0..(REVIEW_BRANCH_COMMIT_LIMIT + 3) {
            env.git(
                &repo,
                &[
                    "commit",
                    "--allow-empty",
                    "-m",
                    &format!("c{index}"),
                    "--quiet",
                ],
            );
        }
        let result = get_branch_commits(
            &ReviewBranchCommitsInput {
                cwd: repo.to_string_lossy().into_owned(),
                base_ref: Some("main".into()),
            },
            &env.ctx,
        )
        .unwrap();
        assert_eq!(result.commits.len(), REVIEW_BRANCH_COMMIT_LIMIT);
        assert!(result.truncated);

        let long_subject = "x".repeat(REVIEW_BRANCH_SUBJECT_LIMIT + 100);
        env.git(&repo, &["checkout", "-b", "subjects", "--quiet"]);
        env.git(
            &repo,
            &["commit", "--allow-empty", "-m", &long_subject, "--quiet"],
        );
        let subjects = get_branch_commits(
            &ReviewBranchCommitsInput {
                cwd: repo.to_string_lossy().into_owned(),
                base_ref: Some("feature".into()),
            },
            &env.ctx,
        )
        .unwrap();
        assert_eq!(subjects.commits.len(), 1);
        assert_eq!(
            subjects.commits[0].subject.chars().count(),
            REVIEW_BRANCH_SUBJECT_LIMIT
        );
    }

    #[test]
    fn unborn_repository_reports_additions_and_empty_commits() {
        require_git!();
        let env = TestEnv::new("unborn");
        let repo = env._root.path.join("unborn-repo");
        std::fs::create_dir_all(&repo).unwrap();
        env.git(&repo, &["init", "-b", "main", "--quiet"]);
        std::fs::write(repo.join("fresh.txt"), "fresh\n").unwrap();
        env.git(&repo, &["add", "fresh.txt"]);
        std::fs::write(repo.join("loose.txt"), "loose\n").unwrap();

        let result = get_diff_preview(&preview_input(&repo), &env.ctx).unwrap();
        let source = result
            .sources
            .iter()
            .find(|source| source.kind == ReviewSourceKind::WorkingTree)
            .unwrap();
        let files: Vec<&str> = source
            .files
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(files.contains(&"fresh.txt"), "{files:?}");
        assert!(files.contains(&"loose.txt"), "{files:?}");
        assert!(source.diff.contains("fresh.txt"));

        let commits = get_branch_commits(
            &ReviewBranchCommitsInput {
                cwd: repo.to_string_lossy().into_owned(),
                base_ref: None,
            },
            &env.ctx,
        )
        .unwrap();
        assert!(commits.commits.is_empty());
        assert!(!commits.truncated);
    }

    #[test]
    fn non_repository_returns_empty_shapes() {
        require_git!();
        let env = TestEnv::new("non-repo");
        let plain = env._root.path.join("plain");
        std::fs::create_dir_all(&plain).unwrap();

        let preview = get_diff_preview(&preview_input(&plain), &env.ctx).unwrap();
        assert!(preview.sources.is_empty());
        assert_eq!(preview.cwd, plain.to_string_lossy());

        let commits = get_branch_commits(
            &ReviewBranchCommitsInput {
                cwd: plain.to_string_lossy().into_owned(),
                base_ref: None,
            },
            &env.ctx,
        )
        .unwrap();
        assert!(commits.commits.is_empty());
        assert!(!commits.truncated);

        let err = get_diff_file_contents(
            &contents_input(&plain, ReviewChangeType::Change, "x.txt"),
            &env.ctx,
            &env.state_dir(),
        )
        .unwrap_err();
        assert!(err.contains("requires a Git repository"), "{err}");
    }

    #[test]
    fn patch_output_truncates_at_review_limit() {
        require_git!();
        let env = TestEnv::new("patch-limit");
        let repo = env.init_repo("repo");
        let big: String = (0..9000)
            .map(|i| format!("line-{i}-xxxxxxxxxxxxxxxx\n"))
            .collect();
        std::fs::write(repo.join("big.txt"), &big).unwrap();
        env.git(&repo, &["add", "big.txt"]);
        env.git(&repo, &["commit", "-m", "big", "--quiet"]);
        let bigger: String = (0..9000)
            .map(|i| format!("changed-{i}-yyyyyyyyyyyyyyyyyyyy\n"))
            .collect();
        std::fs::write(repo.join("big.txt"), &bigger).unwrap();

        let result = get_diff_preview(&preview_input(&repo), &env.ctx).unwrap();
        let source = &result.sources[0];
        assert!(source.truncated);
        assert_eq!(source.diff.len(), REVIEW_DIFF_PATCH_MAX_OUTPUT_BYTES);
    }

    #[test]
    fn metadata_only_defers_patches() {
        require_git!();
        let env = TestEnv::new("metadata");
        let repo = env.init_repo("repo");
        std::fs::write(repo.join("changed.txt"), "new\n").unwrap();
        let mut input = preview_input(&repo);
        input.metadata_only = Some(true);
        let result = get_diff_preview(&input, &env.ctx).unwrap();
        let source = &result.sources[0];
        assert_eq!(source.diff, "");
        assert_eq!(source.patches_deferred, Some(true));
        assert!(!source.truncated);
        assert!(file_names(&result, 0).contains(&"changed.txt".to_owned()));
    }

    #[test]
    fn commit_source_uses_parent_range() {
        require_git!();
        let env = TestEnv::new("commit-source");
        let repo = env.init_repo("repo");
        std::fs::write(repo.join("committed.txt"), "first\n").unwrap();
        env.git(&repo, &["add", "."]);
        env.git(&repo, &["commit", "-m", "second commit", "--quiet"]);
        let sha = env.git(&repo, &["rev-parse", "HEAD"]).trim().to_owned();

        let mut input = preview_input(&repo);
        input.commit_sha = Some(sha.clone());
        let result = get_diff_preview(&input, &env.ctx).unwrap();
        assert_eq!(result.sources.len(), 1);
        let source = &result.sources[0];
        assert_eq!(source.kind, ReviewSourceKind::Commit);
        assert_eq!(source.id, sha);
        assert_eq!(source.title, format!("Commit {}", &sha[..7]));
        assert_eq!(source.commit_subject.as_deref(), Some("second commit"));
        assert!(source.diff.contains("committed.txt"));

        // Commit-scoped file contents read both sides at the right revisions.
        let contents = get_diff_file_contents(
            &ReviewDiffFileContentsInput {
                cwd: repo.to_string_lossy().into_owned(),
                source_kind: ReviewSourceKind::Commit,
                change_type: ReviewChangeType::New,
                base_ref: source.base_ref.clone(),
                head_ref: Some(sha),
                old_path: "committed.txt".into(),
                new_path: "committed.txt".into(),
            },
            &env.ctx,
            &env.state_dir(),
        )
        .unwrap();
        assert_eq!(contents.old_contents, "");
        assert_eq!(contents.new_contents, "first\n");

        // A mismatched baseRef is a stale selection.
        let stale = get_diff_file_contents(
            &ReviewDiffFileContentsInput {
                cwd: repo.to_string_lossy().into_owned(),
                source_kind: ReviewSourceKind::Commit,
                change_type: ReviewChangeType::New,
                base_ref: Some("0".repeat(40)),
                head_ref: source.head_ref.clone(),
                old_path: "committed.txt".into(),
                new_path: "committed.txt".into(),
            },
            &env.ctx,
            &env.state_dir(),
        )
        .unwrap_err();
        assert!(stale.contains("Selected commit changed"), "{stale}");
    }

    #[test]
    fn file_contents_reads_revision_and_working_tree() {
        require_git!();
        let env = TestEnv::new("file-contents");
        let repo = env.init_repo("repo");
        std::fs::write(repo.join("text.txt"), "before\n").unwrap();
        env.git(&repo, &["add", "."]);
        env.git(&repo, &["commit", "-m", "text", "--quiet"]);
        std::fs::write(repo.join("text.txt"), "after\n").unwrap();

        let result = get_diff_file_contents(
            &contents_input(&repo, ReviewChangeType::Change, "text.txt"),
            &env.ctx,
            &env.state_dir(),
        )
        .unwrap();
        assert_eq!(result.old_contents, "before\n");
        assert_eq!(result.new_contents, "after\n");

        std::fs::write(repo.join("added.txt"), "new file\n").unwrap();
        let added = get_diff_file_contents(
            &contents_input(&repo, ReviewChangeType::New, "added.txt"),
            &env.ctx,
            &env.state_dir(),
        )
        .unwrap();
        assert_eq!(added.old_contents, "");
        assert_eq!(added.new_contents, "new file\n");

        let mut removed = contents_input(&repo, ReviewChangeType::Deleted, "README.md");
        std::fs::remove_file(repo.join("README.md")).unwrap();
        removed.new_path = "README.md".into();
        let deleted = get_diff_file_contents(&removed, &env.ctx, &env.state_dir()).unwrap();
        assert_eq!(deleted.old_contents, "hello\n");
        assert_eq!(deleted.new_contents, "");

        std::fs::write(repo.join("binary.dat"), [0u8, 1, 2]).unwrap();
        let binary = get_diff_file_contents(
            &contents_input(&repo, ReviewChangeType::New, "binary.dat"),
            &env.ctx,
            &env.state_dir(),
        )
        .unwrap_err();
        assert!(binary.contains("Cannot expand binary file"), "{binary}");
    }

    #[test]
    fn file_contents_rejects_escapes_and_state_files() {
        require_git!();
        let env = TestEnv::new("escape");
        let repo = env.init_repo("repo");
        std::fs::create_dir_all(&env.state_dir()).unwrap();
        std::fs::write(env.state_dir().join("state.sqlite"), b"sqlite").unwrap();

        // `..` escape is rejected lexically.
        let mut input = contents_input(&repo, ReviewChangeType::New, "../outside.txt");
        let err = get_diff_file_contents(&input, &env.ctx, &env.state_dir()).unwrap_err();
        assert!(err.contains("outside the review workspace"), "{err}");

        // A symlink pointing outside the repository is rejected after realPath.
        let outside = env._root.path.join("outside.txt");
        std::fs::write(&outside, "outside\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, repo.join("linked.txt")).unwrap();
            input.new_path = "linked.txt".into();
            let err = get_diff_file_contents(&input, &env.ctx, &env.state_dir()).unwrap_err();
            assert!(err.contains("outside the review workspace"), "{err}");
        }

        // A symlink into the server's own state is refused as well.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                env.state_dir().join("state.sqlite"),
                repo.join("state-link"),
            )
            .unwrap();
            input.new_path = "state-link".into();
            // It also escapes the repo, so either refusal message is correct;
            // use a state dir inside the repo instead for a cleaner check.
        }
        let repo_state = repo.join("repo-state");
        std::fs::create_dir_all(&repo_state).unwrap();
        let nested_db = repo_state.join("nested.sqlite");
        std::fs::write(&nested_db, b"db").unwrap();
        input.new_path = "repo-state/nested.sqlite".into();
        let err = get_diff_file_contents(&input, &env.ctx, &repo_state).unwrap_err();
        assert!(err.contains("is not available"), "{err}");
    }

    #[test]
    fn file_reads_enforce_the_one_mb_limit() {
        require_git!();
        let env = TestEnv::new("file-limit");
        let repo = env.init_repo("repo");
        let oversized = "x".repeat(REVIEW_DIFF_FILE_MAX_OUTPUT_BYTES + 16);
        std::fs::write(repo.join("huge.txt"), &oversized).unwrap();

        let err = get_diff_file_contents(
            &contents_input(&repo, ReviewChangeType::New, "huge.txt"),
            &env.ctx,
            &env.state_dir(),
        )
        .unwrap_err();
        assert!(err.contains("exceeds the 1 MB expansion limit"), "{err}");

        env.git(&repo, &["add", "huge.txt"]);
        env.git(&repo, &["commit", "-m", "huge", "--quiet"]);
        std::fs::remove_file(repo.join("huge.txt")).unwrap();
        let err = get_diff_file_contents(
            &contents_input(&repo, ReviewChangeType::Deleted, "huge.txt"),
            &env.ctx,
            &env.state_dir(),
        )
        .unwrap_err();
        assert!(err.contains("Git output exceeded"), "{err}");
    }

    #[test]
    fn too_many_untracked_files_mark_the_result_incomplete() {
        require_git!();
        let env = TestEnv::new("untracked-limit");
        let repo = env.init_repo("repo");
        for index in 0..=REVIEW_UNTRACKED_MAX_FILES {
            std::fs::write(repo.join(format!("u{index}.txt")), "x").unwrap();
        }
        let result = get_diff_preview(&preview_input(&repo), &env.ctx).unwrap();
        let source = &result.sources[0];
        assert!(source.truncated);
        assert!(source.files.is_none());
    }

    #[test]
    fn file_scoped_preview_diffs_only_that_path() {
        require_git!();
        let env = TestEnv::new("file-scoped");
        let repo = env.init_repo("repo");
        std::fs::write(repo.join("kept.txt"), "keep\n").unwrap();
        std::fs::write(repo.join("other.txt"), "other\n").unwrap();
        env.git(&repo, &["add", "."]);
        env.git(&repo, &["commit", "-m", "seed", "--quiet"]);
        std::fs::write(repo.join("kept.txt"), "keep\nmore\n").unwrap();
        std::fs::write(repo.join("other.txt"), "other\nmore\n").unwrap();
        std::fs::write(repo.join("new-untracked.txt"), "fresh\n").unwrap();

        let mut input = preview_input(&repo);
        input.file = Some(ReviewDiffFileArg {
            path: "kept.txt".into(),
            previous_path: None,
            source_kind: ReviewSourceKind::WorkingTree,
        });
        let result = get_diff_preview(&input, &env.ctx).unwrap();
        let source = &result.sources[0];
        assert!(source.diff.contains("kept.txt"));
        assert!(!source.diff.contains("other.txt"));
        assert_eq!(file_names(&result, 0), vec!["kept.txt".to_owned()]);

        // File-scoped working-tree diff picks up an untracked file too.
        input.file = Some(ReviewDiffFileArg {
            path: "new-untracked.txt".into(),
            previous_path: None,
            source_kind: ReviewSourceKind::WorkingTree,
        });
        let result = get_diff_preview(&input, &env.ctx).unwrap();
        assert!(result.sources[0].diff.contains("new-untracked.txt"));
        assert_eq!(file_names(&result, 0), vec!["new-untracked.txt".to_owned()]);
    }

    #[test]
    fn tracked_only_excludes_untracked_and_reports_incomplete() {
        require_git!();
        let env = TestEnv::new("tracked-only");
        let repo = env.init_repo("repo");
        std::fs::write(repo.join("tracked.txt"), "a\n").unwrap();
        env.git(&repo, &["add", "tracked.txt"]);
        env.git(&repo, &["commit", "-m", "tracked", "--quiet"]);
        std::fs::write(repo.join("tracked.txt"), "b\n").unwrap();
        std::fs::write(repo.join("untracked.txt"), "u\n").unwrap();

        let mut input = preview_input(&repo);
        input.tracked_only = Some(true);
        let result = get_diff_preview(&input, &env.ctx).unwrap();
        assert_eq!(result.incomplete, Some(true));
        let files = file_names(&result, 0);
        assert!(files.contains(&"tracked.txt".to_owned()));
        assert!(!files.contains(&"untracked.txt".to_owned()));
    }
}
