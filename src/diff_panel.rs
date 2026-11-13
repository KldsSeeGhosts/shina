//! The right panel's Diff tab: the changed files of the thread's workspace,
//! fetched as patch text, parsed by `review_patch` and drawn as collapsible
//! sections of hunks and lines.
//!
//! Ported from KSG Code's `DiffPanel.tsx` (the git scopes "unstaged" and
//! "branch" only), `components/diffs/useReviewFilePatches.ts` (metadata-first
//! loading with lazy per-file patches) and `hooks/useLiveRefresh.ts` (the
//! refresh cadence). Data comes from `review.getDiffPreview`: in-process for
//! local threads, a capability-gated RPC for threads on linked machines.

use super::*;
use crate::review::{
    ReviewDiffFileArg, ReviewDiffFileStat, ReviewDiffPreviewInput, ReviewDiffPreviewResult,
    ReviewDiffPreviewSource, ReviewPreviewSourceKind, ReviewSourceKind,
};
use crate::review_patch::{ChangeKind, FileDiff, LineKind, RenderablePatch};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// KSG `useLiveRefresh.ts`'s `LIVE_REFRESH_MIN_INTERVAL_MS`: how fresh a shown
/// diff must be to skip a re-read, and how soon a queued refresh may run.
const LIVE_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(10);
/// KSG `useReviewFilePatches.ts`'s `INITIAL_FILE_COUNT`: files fetched ahead of
/// the fold in the first batch and in each scroll-triggered batch.
const INITIAL_FILE_COUNT: usize = 4;
/// KSG's empty-state wording.
const EMPTY_TEXT: &str = "No net changes in this selection.";
const MISSING_TEXT: &str = "No patch available for this selection.";
const NOT_REPO_TEXT: &str =
    "Turn diffs are unavailable because this project is not a git repository.";
const TRUNCATED_TEXT: &str = "This preview exceeds the size limit. Changes shown are incomplete.";
const INCOMPLETE_TEXT: &str = "Showing tracked changes. Checking untracked files...";
const NO_NEWLINE_TEXT: &str = "No newline at end of file";

/// The git scopes this port covers. KSG's `DEFAULT_SELECTION` is
/// `{ kind: "branch" }`, so Branch is the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scope {
    /// KSG "branch": the branch against its base, with the server's default
    /// base until a base picker exists.
    #[default]
    Branch,
    /// KSG "unstaged": the working tree against HEAD, untracked files included.
    Uncommitted,
}

impl Scope {
    fn preview_kind(self) -> ReviewPreviewSourceKind {
        match self {
            Self::Uncommitted => ReviewPreviewSourceKind::WorkingTree,
            Self::Branch => ReviewPreviewSourceKind::BranchRange,
        }
    }
    fn source_kind(self) -> ReviewSourceKind {
        match self {
            Self::Uncommitted => ReviewSourceKind::WorkingTree,
            Self::Branch => ReviewSourceKind::BranchRange,
        }
    }
    /// KSG's scope menu labels ("Uncommitted", "Changes").
    pub fn label(self) -> &'static str {
        match self {
            Self::Uncommitted => "Uncommitted",
            Self::Branch => "Changes",
        }
    }
    /// KSG's loading labels.
    fn loading_label(self) -> &'static str {
        match self {
            Self::Uncommitted => "Loading uncommitted changes...",
            Self::Branch => "Loading changes...",
        }
    }
}

/// A file's patch lifecycle: metadata rows exist from the preview; a `Loaded`
/// entry carries the parsed patch. `Pending` is never requested, `Loading` has
/// a request in flight, `Missing` answered without the file, `Failed` errored
/// (KSG's per-file error state with retry).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileBody {
    Pending,
    Loading,
    Loaded,
    Missing,
    Failed(String),
}

/// One row of the file list: the stat's entry until its patch loads.
#[derive(Clone, Debug)]
pub struct FileEntry {
    /// A stat-derived placeholder until the patch lands; the parsed file after.
    pub file: FileDiff,
    /// The header stat: metadata counts while stats exist, parsed counts after.
    pub additions: u32,
    pub deletions: u32,
    pub body: FileBody,
    /// The file's own patch came back truncated (KSG `fileStates.truncated`).
    pub truncated: bool,
}

/// KSG's raw-patch fallback: the parser's reason, then the raw patch text.
#[derive(Clone, Debug)]
pub struct RawPatch {
    pub reason: String,
    pub text: String,
}

/// A finished preview that has files or a raw patch to show.
#[derive(Clone, Debug)]
pub struct Ready {
    /// The full file list in `review_tree::order_like_tree` order. Entries stay
    /// at their full-list index; hide-tests filters only what is drawn.
    pub files: Vec<FileEntry>,
    /// Set when the patch would not parse into files (KSG's raw fallback).
    pub raw: Option<RawPatch>,
    /// Header totals: the metadata sum when stats exist, else the parsed sum.
    pub additions: u32,
    pub deletions: u32,
    pub truncated: bool,
    /// The server could not index untracked files in time (KSG `incomplete`).
    pub incomplete: bool,
    /// Request context the per-file patch requests reuse.
    pub cwd: String,
    pub source_kind: ReviewSourceKind,
    /// KSG `lazySource?.baseRef`: the resolved base for per-file requests.
    pub base_ref: Option<String>,
    /// The flattened row list behind `virtual_rows`, rebuilt on data changes
    /// (never per frame) and versioned so row keys re-commit after a rebuild.
    pub rows: Vec<DiffRow>,
    pub rows_version: u64,
}

/// What the last preview left to show; mirrors KSG's empty-state chain.
#[derive(Clone, Debug)]
pub enum Done {
    Ready(Box<Ready>),
    /// The selected scope resolved to an empty patch.
    Empty,
    /// The scope's source was absent from the result.
    Missing,
    /// `sources` was empty: the directory is not a git repository.
    NotRepo,
    /// The owning machine lacks `review-diff-v1`; the name is its label.
    Unsupported(String),
    Failed(String),
}

/// Per-thread panel state, keyed like `right_panels`.
#[derive(Clone, Debug, Default)]
pub struct ThreadDiff {
    pub scope: Scope,
    /// The request the panel is waiting on; newer replaces older.
    pub request: u64,
    pub loading: bool,
    pub done: Option<Done>,
    /// Collapsed file identities per scope (KSG `collapsedFileKeysByScopeKey`).
    pub collapsed: HashMap<Scope, HashSet<String>>,
    /// KSG `lastRefreshedAt`: set when a request starts.
    pub last_refresh_at: Option<Instant>,
    /// A provider turn or file change asked for a refresh (KSG's mutation bump).
    pub refresh_wanted: bool,
    /// The thread's last seen `life.last_completed_at`: a bump means a turn
    /// finished, which marks the diff stale (for local and remote threads).
    pub seen_completed_at: Option<u64>,
}

impl ThreadDiff {
    pub fn ready(&self) -> Option<&Ready> {
        match &self.done {
            Some(Done::Ready(ready)) => Some(ready),
            _ => None,
        }
    }
    pub fn ready_mut(&mut self) -> Option<&mut Ready> {
        match &mut self.done {
            Some(Done::Ready(ready)) => Some(ready),
            _ => None,
        }
    }
}

/// What an in-flight `diff:{req}` RPC is for; stored in `pending_diff_requests`.
#[derive(Clone, Debug)]
pub enum DiffPending {
    Preview {
        panel_key: String,
        scope: Scope,
        cwd: String,
    },
    File {
        panel_key: String,
        path: String,
        preview_request: u64,
    },
}

/// The per-file patch parse finished on a worker.
#[derive(Clone, Debug)]
pub enum FileOutcome {
    Loaded { file: FileDiff, truncated: bool },
    Missing,
    Failed(String),
}

/// A finished unit of background diff work on `diff_rxs`. `request` is the
/// preview request the work belongs to; a stale value is dropped on apply.
pub enum DiffMsg {
    Preview {
        panel_key: String,
        request: u64,
        done: Done,
    },
    File {
        panel_key: String,
        request: u64,
        path: String,
        outcome: FileOutcome,
    },
}

/// A flattened row of the visible list. `file` indexes `Ready::files` (the full
/// list), `display` counts shown files for the `diff-file-{display}` keys, and
/// `key` is the row's stable list key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffRow {
    File {
        display: usize,
        file: usize,
        identity: String,
        key: String,
    },
    Hunk {
        file: usize,
        text: String,
        key: String,
    },
    Line {
        file: usize,
        kind: LineKind,
        old: Option<u32>,
        new: Option<u32>,
        text: String,
        key: String,
    },
    /// The faint "No newline at end of file" row after a `no_newline` line.
    Marker { file: usize, key: String },
    /// A one-line faint row: binary file, empty file, pending/loading, or a
    /// per-file error with retry.
    Note {
        file: usize,
        display: usize,
        text: String,
        retry: bool,
        key: String,
    },
    /// KSG's list-end loading boundary; becoming visible fetches the next batch.
    Boundary { remaining: usize, key: String },
}

impl DiffRow {
    pub fn key(&self) -> &str {
        match self {
            Self::File { key, .. }
            | Self::Hunk { key, .. }
            | Self::Line { key, .. }
            | Self::Marker { key, .. }
            | Self::Note { key, .. }
            | Self::Boundary { key, .. } => key,
        }
    }
}

// --- loading logic ---------------------------------------------------------

/// KSG `branchDiffPreviewInput`: a metadata-first preview of the selected
/// scope. `baseRef` stays unset while Shina has no base picker, which is the
/// default base KSG uses.
fn preview_input(scope: Scope, cwd: &str) -> ReviewDiffPreviewInput {
    ReviewDiffPreviewInput {
        cwd: cwd.to_string(),
        base_ref: None,
        commit_sha: None,
        ignore_whitespace: Some(false),
        source_kind: Some(scope.preview_kind()),
        tracked_only: None,
        metadata_only: Some(true),
        file: None,
    }
}

/// KSG `reviewFileQueries`'s per-file request: `review.getDiffPreview` narrowed
/// to one file of the source.
fn file_input(ready: &Ready, path: &str, previous_path: Option<String>) -> ReviewDiffPreviewInput {
    ReviewDiffPreviewInput {
        cwd: ready.cwd.clone(),
        base_ref: ready.base_ref.clone(),
        commit_sha: None,
        ignore_whitespace: Some(false),
        source_kind: None,
        tracked_only: None,
        metadata_only: None,
        file: Some(ReviewDiffFileArg {
            path: path.to_string(),
            previous_path,
            source_kind: ready.source_kind,
        }),
    }
}

/// A stat row's stand-in file: renamed when it has a previous path, else a
/// plain change — KSG's `"rename-changed" | "change"` placeholder types.
fn stat_placeholder(stat: &ReviewDiffFileStat) -> FileDiff {
    FileDiff {
        path: stat.path.clone(),
        previous_path: stat.previous_path.clone(),
        change: if stat.previous_path.is_some() {
            ChangeKind::Renamed
        } else {
            ChangeKind::Modified
        },
        binary: false,
        old_mode: None,
        new_mode: None,
        hunks: Vec::new(),
        additions: u32::try_from(stat.additions).unwrap_or(u32::MAX),
        deletions: u32::try_from(stat.deletions).unwrap_or(u32::MAX),
    }
}

fn stats_totals(stats: &[ReviewDiffFileStat]) -> (u32, u32) {
    stats.iter().fold((0u32, 0u32), |(adds, dels), stat| {
        (
            adds.saturating_add(u32::try_from(stat.additions).unwrap_or(u32::MAX)),
            dels.saturating_add(u32::try_from(stat.deletions).unwrap_or(u32::MAX)),
        )
    })
}

/// The scope's source turned into the panel's `Done`: KSG's `selectedGitSource`
/// lookup and its empty-state chain. Runs on a worker thread.
fn preview_done(scope: Scope, cwd: &str, result: ReviewDiffPreviewResult) -> Done {
    if result.sources.is_empty() {
        return Done::NotRepo;
    }
    let kind = scope.source_kind();
    let Some(source) = result
        .sources
        .iter()
        .find(|source| source.kind == kind)
        .cloned()
    else {
        return Done::Missing;
    };
    let mut ready = Ready {
        files: Vec::new(),
        raw: None,
        additions: 0,
        deletions: 0,
        truncated: source.truncated,
        incomplete: result.incomplete == Some(true),
        cwd: cwd.to_string(),
        source_kind: source.kind,
        base_ref: source.base_ref.clone(),
        rows: Vec::new(),
        rows_version: 0,
    };
    let stats = source.files.unwrap_or_default();
    // KSG `lazySource`: deferred patches (or a truncated inline patch with
    // stats) show the stat list and fetch each file's patch on demand.
    let lazy = (source.patches_deferred == Some(true) || source.truncated) && !stats.is_empty();
    if lazy {
        (ready.additions, ready.deletions) = stats_totals(&stats);
        let placeholders: Vec<FileDiff> = stats.iter().map(stat_placeholder).collect();
        ready.files = crate::review_tree::order_like_tree(&placeholders)
            .into_iter()
            .map(|file| FileEntry {
                additions: file.additions,
                deletions: file.deletions,
                file,
                body: FileBody::Pending,
                truncated: false,
            })
            .collect();
        return Done::Ready(Box::new(ready));
    }
    match crate::review_patch::parse(&source.diff) {
        None => Done::Empty,
        Some(RenderablePatch::Raw { text, reason }) => {
            if !stats.is_empty() {
                (ready.additions, ready.deletions) = stats_totals(&stats);
            }
            ready.raw = Some(RawPatch { reason, text });
            Done::Ready(Box::new(ready))
        }
        Some(RenderablePatch::Files(files)) => {
            let files = crate::review_tree::order_like_tree(&files);
            if stats.is_empty() {
                let stat = crate::review_patch::line_stat(&files);
                ready.additions = stat.additions;
                ready.deletions = stat.deletions;
            } else {
                (ready.additions, ready.deletions) = stats_totals(&stats);
            }
            ready.files = files
                .into_iter()
                .map(|file| FileEntry {
                    additions: file.additions,
                    deletions: file.deletions,
                    file,
                    body: FileBody::Loaded,
                    truncated: false,
                })
                .collect();
            Done::Ready(Box::new(ready))
        }
    }
}

/// A per-file preview answer parsed for its file — KSG's `parsedPatchAtom`
/// merge (`patch.files` filtered to the stat's path).
fn file_outcome(path: &str, result: ReviewDiffPreviewResult) -> FileOutcome {
    let text = result
        .sources
        .iter()
        .map(|source: &ReviewDiffPreviewSource| source.diff.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let truncated = result.sources.iter().any(|source| source.truncated);
    match crate::review_patch::parse(&text) {
        Some(RenderablePatch::Files(files)) => files
            .into_iter()
            .find(|file| file.path == path || file.previous_path.as_deref() == Some(path))
            .map(|file| FileOutcome::Loaded { file, truncated })
            .unwrap_or(FileOutcome::Missing),
        _ => FileOutcome::Missing,
    }
}

/// `@@ -a,b +c,d @@ section`, with git's `,1` elision.
fn hunk_header(hunk: &crate::review_patch::Hunk) -> String {
    let side = |start: u32, count: u32| {
        if count == 1 {
            format!("{start}")
        } else {
            format!("{start},{count}")
        }
    };
    let section = if hunk.section.is_empty() {
        String::new()
    } else {
        format!(" {}", hunk.section)
    };
    format!(
        "@@ -{} +{} @@{}",
        side(hunk.old_start, hunk.old_count),
        side(hunk.new_start, hunk.new_count),
        section
    )
}

/// KSG `useLiveRefresh`'s gate: a refresh is due when nothing was read yet or
/// the last read is older than the interval.
fn refresh_due(last_refresh_at: Option<Instant>, now: Instant) -> bool {
    last_refresh_at.is_none_or(|at| now.duration_since(at) >= LIVE_REFRESH_MIN_INTERVAL)
}

/// The visible row list: file headers, hunk headers, line, marker and note
/// rows, plus the trailing batch boundary. Rebuilt whenever files, the collapse
/// set or the test filter change — never per frame.
fn flatten(files: &[FileEntry], hide_tests: bool, collapsed: &HashSet<String>) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    let mut display = 0usize;
    for (index, entry) in files.iter().enumerate() {
        if hide_tests && crate::review_tree::is_test_changed_file(&entry.file.path) {
            continue;
        }
        let identity = crate::review_patch::identity_key(&entry.file);
        let d = display;
        display += 1;
        rows.push(DiffRow::File {
            display: d,
            file: index,
            identity: identity.clone(),
            key: format!("diff-file-{d}"),
        });
        if collapsed.contains(&identity) {
            continue;
        }
        let note = |text: String, key: String| DiffRow::Note {
            file: index,
            display: d,
            text,
            retry: false,
            key,
        };
        match &entry.body {
            FileBody::Pending | FileBody::Loading => rows.push(note(
                "Loading file diff...".into(),
                format!("diff-note-{d}-load"),
            )),
            FileBody::Failed(_) | FileBody::Missing => rows.push(DiffRow::Note {
                file: index,
                display: d,
                text: "Failed to load this file diff.".into(),
                retry: true,
                key: format!("diff-file-retry-{d}"),
            }),
            FileBody::Loaded => {
                if entry.file.binary {
                    rows.push(note("Binary file".into(), format!("diff-note-{d}-bin")));
                } else if entry.file.hunks.is_empty() {
                    let text = match entry.file.change {
                        ChangeKind::Renamed => "Renamed without changes.",
                        ChangeKind::Copied => "Copied without changes.",
                        _ => "No changes to display.",
                    };
                    rows.push(note(text.into(), format!("diff-note-{d}-empty")));
                } else {
                    for (h, hunk) in entry.file.hunks.iter().enumerate() {
                        rows.push(DiffRow::Hunk {
                            file: index,
                            text: hunk_header(hunk),
                            key: format!("diff-hunk-{d}-{h}"),
                        });
                        for (l, line) in hunk.lines.iter().enumerate() {
                            rows.push(DiffRow::Line {
                                file: index,
                                kind: line.kind,
                                old: line.old_number,
                                new: line.new_number,
                                text: line.text.clone(),
                                key: format!("diff-line-{d}-{h}-{l}"),
                            });
                            if line.no_newline {
                                rows.push(DiffRow::Marker {
                                    file: index,
                                    key: format!("diff-marker-{d}-{h}-{l}"),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    // KSG `remainingFileCount`: everything at or after the first unsettled file.
    let settled = files
        .iter()
        .position(|entry| matches!(entry.body, FileBody::Pending | FileBody::Loading))
        .unwrap_or(files.len());
    let remaining = files.len() - settled;
    if remaining > 0 {
        rows.push(DiffRow::Boundary {
            remaining,
            key: "diff-boundary".to_string(),
        });
    }
    rows
}

// --- state ------------------------------------------------------------------

impl State {
    /// KSG `refreshOnShow` plus first mount: (re)read when the tab is stale, or
    /// schedule a queued refresh for its interval floor. Called by the body's
    /// `on_visible`.
    pub(crate) fn diff_shown(&mut self, tx: &mut Tx) {
        let key = self.panel_key();
        let (due, wanted, first_load_pending) =
            self.diffs.get(&key).map_or((true, false, false), |d| {
                (
                    refresh_due(d.last_refresh_at, Instant::now()),
                    d.refresh_wanted,
                    d.loading && d.done.is_none(),
                )
            });
        if due && !first_load_pending {
            self.diff_request_key(&key);
        } else if wanted && !due {
            // Land the queued refresh at the interval floor.
            let wait = self
                .diffs
                .get(&key)
                .and_then(|d| d.last_refresh_at)
                .map_or(Duration::ZERO, |at| {
                    LIVE_REFRESH_MIN_INTERVAL.saturating_sub(at.elapsed())
                });
            tx.wake_after(wait);
        }
    }

    /// The current thread's diff (re)read: show, scope change, refresh, retry.
    pub(crate) fn diff_request(&mut self) {
        let key = self.panel_key();
        self.diff_request_key(&key);
    }

    fn diff_request_key(&mut self, panel_key: &str) {
        let diff = self.diffs.entry(panel_key.to_string()).or_default();
        self.next_diff_request_id += 1;
        let request = self.next_diff_request_id;
        diff.request = request;
        diff.loading = true;
        diff.last_refresh_at = Some(Instant::now());
        diff.refresh_wanted = false;
        let scope = diff.scope;
        let Some(index) = self
            .threads
            .iter()
            .position(|t| crate::right_panel::key(&t.machine, t.id) == panel_key)
        else {
            self.diff_apply_done(
                panel_key,
                request,
                Done::Failed("Thread does not exist.".into()),
            );
            return;
        };
        let machine = self.threads[index].machine.clone();
        let cwd = match self.thread_directory(index) {
            Ok(dir) => dir.to_string_lossy().into_owned(),
            Err(error) => {
                self.diff_apply_done(panel_key, request, Done::Failed(error));
                return;
            }
        };
        let input = preview_input(scope, &cwd);
        if let Some(client) = &self.client {
            // The thread's owning machine answers review.getDiffPreview.
            if !self.machine_connected(&machine) {
                self.diff_apply_done(
                    panel_key,
                    request,
                    Done::Failed("This machine is not reachable right now.".into()),
                );
                return;
            }
            if !client.supports_capability(&machine, crate::protocol::REVIEW_DIFF) {
                let label = self.machine_label(&machine);
                self.diff_apply_done(panel_key, request, Done::Unsupported(label));
                return;
            }
            let target = if machine == "local" {
                Some(client)
            } else {
                client.linked.get(&machine)
            };
            let Some(target) = target else {
                self.diff_apply_done(
                    panel_key,
                    request,
                    Done::Failed("This machine is not reachable right now.".into()),
                );
                return;
            };
            match serde_json::to_value(&input) {
                Ok(params) => {
                    self.pending_diff_requests.insert(
                        request,
                        DiffPending::Preview {
                            panel_key: panel_key.to_string(),
                            scope,
                            cwd,
                        },
                    );
                    target.call_rpc(format!("diff:{request}"), "review.getDiffPreview", params);
                }
                Err(error) => self.diff_apply_done(
                    panel_key,
                    request,
                    Done::Failed(format!("Invalid diff request: {error}")),
                ),
            }
            return;
        }
        // Local: git runs on a worker, never on the UI thread.
        let (tx, rx) = std::sync::mpsc::channel();
        self.diff_rxs.push(rx);
        let wake = self.wake.clone();
        let panel_key = panel_key.to_string();
        #[cfg(test)]
        let git_ctx = self.test_git_context.clone().unwrap_or_default();
        #[cfg(not(test))]
        let git_ctx = crate::vcs::GitContext::default();
        std::thread::spawn(move || {
            let done = match crate::review::get_diff_preview(&input, &git_ctx) {
                Ok(result) => preview_done(scope, &input.cwd, result),
                Err(error) => Done::Failed(error),
            };
            let _ = tx.send(DiffMsg::Preview {
                panel_key,
                request,
                done,
            });
            wake();
        });
    }

    /// Store a finished preview and open the automatic first file batch.
    fn diff_apply_done(&mut self, panel_key: &str, request: u64, done: Done) {
        let Some(diff) = self.diffs.get_mut(panel_key) else {
            return;
        };
        if diff.request != request {
            return;
        }
        diff.loading = false;
        diff.done = Some(done);
        self.diff_rebuild_rows(panel_key);
        self.diff_load_next(panel_key);
    }

    /// KSG's `requested` merge on `DiffFileCollapseToggle load` and `requestFile`:
    /// the file's patch joins the requested set and its request fires.
    fn diff_request_file(&mut self, panel_key: &str, index: usize) {
        let (input, path, preview_request) = {
            let Some(diff) = self.diffs.get_mut(panel_key) else {
                return;
            };
            let preview_request = diff.request;
            let Some(ready) = diff.ready_mut() else {
                return;
            };
            let Some(entry) = ready.files.get_mut(index) else {
                return;
            };
            if !matches!(
                entry.body,
                FileBody::Pending | FileBody::Failed(_) | FileBody::Missing
            ) {
                return;
            }
            entry.body = FileBody::Loading;
            let path = entry.file.path.clone();
            let previous_path = entry.file.previous_path.clone();
            (
                file_input(ready, &path, previous_path),
                path,
                preview_request,
            )
        };
        let Some(index_thread) = self
            .threads
            .iter()
            .position(|t| crate::right_panel::key(&t.machine, t.id) == panel_key)
        else {
            self.diff_apply_file(
                panel_key,
                preview_request,
                &path,
                FileOutcome::Failed("Thread does not exist.".into()),
            );
            return;
        };
        let machine = self.threads[index_thread].machine.clone();
        if let Some(client) = &self.client {
            let connected = self.machine_connected(&machine);
            let target = if machine == "local" {
                Some(client)
            } else {
                client.linked.get(&machine)
            };
            match target.filter(|_| connected) {
                Some(target) => {
                    self.next_diff_request_id += 1;
                    let request = self.next_diff_request_id;
                    match serde_json::to_value(&input) {
                        Ok(params) => {
                            self.pending_diff_requests.insert(
                                request,
                                DiffPending::File {
                                    panel_key: panel_key.to_string(),
                                    path,
                                    preview_request,
                                },
                            );
                            target.call_rpc(
                                format!("diff:{request}"),
                                "review.getDiffPreview",
                                params,
                            );
                            return;
                        }
                        Err(error) => {
                            self.diff_apply_file(
                                panel_key,
                                preview_request,
                                &path,
                                FileOutcome::Failed(format!("Invalid diff request: {error}")),
                            );
                            return;
                        }
                    }
                }
                None => {
                    self.diff_apply_file(
                        panel_key,
                        preview_request,
                        &path,
                        FileOutcome::Failed("This machine is not reachable right now.".into()),
                    );
                    return;
                }
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.diff_rxs.push(rx);
        let wake = self.wake.clone();
        let panel_key = panel_key.to_string();
        #[cfg(test)]
        let git_ctx = self.test_git_context.clone().unwrap_or_default();
        #[cfg(not(test))]
        let git_ctx = crate::vcs::GitContext::default();
        std::thread::spawn(move || {
            let outcome = match crate::review::get_diff_preview(&input, &git_ctx) {
                Ok(result) => file_outcome(&path, result),
                Err(error) => FileOutcome::Failed(error),
            };
            let _ = tx.send(DiffMsg::File {
                panel_key,
                request: preview_request,
                path,
                outcome,
            });
            wake();
        });
    }

    /// Merge a per-file patch into its stat row, KSG's `parsedPatchAtom`
    /// stat-override (`name`/`prevName` come from the stat) plus the
    /// pending-identity to loaded-identity collapse remap.
    fn diff_apply_file(
        &mut self,
        panel_key: &str,
        request: u64,
        path: &str,
        outcome: FileOutcome,
    ) -> bool {
        let Some(diff) = self.diffs.get_mut(panel_key) else {
            return false;
        };
        if diff.request != request {
            return false;
        }
        let mut remap = None;
        {
            let Some(ready) = diff.ready_mut() else {
                return true;
            };
            let Some(index) = ready.files.iter().position(|entry| {
                entry.file.path == path && matches!(entry.body, FileBody::Loading)
            }) else {
                return true;
            };
            match outcome {
                FileOutcome::Loaded {
                    mut file,
                    truncated,
                } => {
                    let entry = &mut ready.files[index];
                    let old_identity = crate::review_patch::identity_key(&entry.file);
                    file.path = entry.file.path.clone();
                    file.previous_path = entry.file.previous_path.clone();
                    entry.file = file;
                    entry.truncated = truncated;
                    entry.body = FileBody::Loaded;
                    let new_identity = crate::review_patch::identity_key(&entry.file);
                    if old_identity != new_identity {
                        remap = Some((old_identity, new_identity));
                    }
                }
                FileOutcome::Missing => ready.files[index].body = FileBody::Missing,
                FileOutcome::Failed(error) => ready.files[index].body = FileBody::Failed(error),
            }
        }
        if let Some((old_identity, new_identity)) = remap {
            let scope = diff.scope;
            let collapsed = diff.collapsed.entry(scope).or_default();
            if collapsed.remove(&old_identity) {
                collapsed.insert(new_identity);
            }
        }
        self.diff_rebuild_rows(panel_key);
        // A settled request may free the batch cursor for the next files.
        self.diff_load_next(panel_key);
        true
    }

    /// KSG `loadNextFiles`: when the list-end boundary shows and the first
    /// unsettled file is not in flight, request the next batch of four.
    pub(crate) fn diff_load_next(&mut self, panel_key: &str) -> bool {
        let indices: Vec<usize> = {
            let Some(diff) = self.diffs.get_mut(panel_key) else {
                return false;
            };
            let Some(ready) = diff.ready_mut() else {
                return false;
            };
            let Some(settled) = ready
                .files
                .iter()
                .position(|entry| matches!(entry.body, FileBody::Pending | FileBody::Loading))
            else {
                return false;
            };
            if matches!(ready.files[settled].body, FileBody::Loading) {
                return false;
            }
            (settled..(settled + INITIAL_FILE_COUNT).min(ready.files.len()))
                .filter(|&index| matches!(ready.files[index].body, FileBody::Pending))
                .collect()
        };
        if indices.is_empty() {
            return false;
        }
        for index in indices {
            self.diff_request_file(panel_key, index);
        }
        true
    }

    /// The collapsed toggle: collapse an expanded file, or expand a collapsed
    /// one — an expanded Pending file requests its patch, as KSG does.
    pub(crate) fn diff_toggle_file(&mut self, panel_key: &str, identity: &str) {
        let (index, expanding) = {
            let Some(diff) = self.diffs.get_mut(panel_key) else {
                return;
            };
            let index = diff.ready().and_then(|ready| {
                ready
                    .files
                    .iter()
                    .position(|entry| crate::review_patch::identity_key(&entry.file) == identity)
            });
            let scope = diff.scope;
            let collapsed = diff.collapsed.entry(scope).or_default();
            let expanding = collapsed.remove(identity);
            if !expanding {
                collapsed.insert(identity.to_string());
            }
            (index, expanding)
        };
        self.diff_rebuild_rows(panel_key);
        if expanding && let Some(index) = index {
            self.diff_request_file(panel_key, index);
        }
    }

    /// KSG `toggleDiffFileCollapse` on the whole shown list.
    pub(crate) fn diff_toggle_all(&mut self) {
        let panel_key = self.panel_key();
        let hide = self.diff_hide_tests;
        let Some(diff) = self.diffs.get_mut(&panel_key) else {
            return;
        };
        let Some(Done::Ready(ready)) = diff.done.as_ref() else {
            return;
        };
        // `ready` borrows `diff.done`, `collapsed` borrows `diff.collapsed`.
        let shown = ready
            .files
            .iter()
            .filter(|entry| !(hide && crate::review_tree::is_test_changed_file(&entry.file.path)))
            .map(|entry| &entry.file);
        let collapsed = diff.collapsed.entry(diff.scope).or_default();
        crate::review_patch::toggle_all(shown, collapsed);
        self.diff_rebuild_rows(&panel_key);
    }

    /// A failed file's retry (KSG `DiffFileStatus`'s retry).
    pub(crate) fn diff_retry_file(&mut self, panel_key: &str, identity: &str) {
        let index =
            self.diffs
                .get(panel_key)
                .and_then(|diff| diff.ready())
                .and_then(|ready| {
                    ready.files.iter().position(|entry| {
                        crate::review_patch::identity_key(&entry.file) == identity
                    })
                });
        if let Some(index) = index {
            self.diff_request_file(panel_key, index);
        }
    }

    /// The app-wide test filter: a display-only filter, so only rows rebuild.
    pub(crate) fn diff_toggle_hide_tests(&mut self) {
        self.diff_hide_tests = !self.diff_hide_tests;
        let keys: Vec<String> = self.diffs.keys().cloned().collect();
        for key in keys {
            self.diff_rebuild_rows(&key);
        }
    }

    /// The scope segmented toggle: a new input refetches, and the previous
    /// scope's data clears while the request runs (KSG's query remount).
    pub(crate) fn diff_set_scope(&mut self, scope: Scope) {
        let key = self.panel_key();
        let diff = self.diffs.entry(key.clone()).or_default();
        if diff.scope == scope {
            return;
        }
        diff.scope = scope;
        diff.done = None;
        diff.loading = false;
        self.diff_request_key(&key);
    }

    /// A provider record on a thread marks its diff stale: the refresh itself
    /// waits for visibility and the interval floor, as KSG's mutation bump does.
    pub(crate) fn diff_note_event(&mut self, index: usize) {
        let Some(thread) = self.threads.get(index) else {
            return;
        };
        let key = crate::right_panel::key(&thread.machine, thread.id);
        if let Some(diff) = self.diffs.get_mut(&key) {
            diff.refresh_wanted = true;
        }
    }

    /// Rebuild a thread's flattened rows after data, collapse or filter changes.
    pub(crate) fn diff_rebuild_rows(&mut self, panel_key: &str) {
        let hide = self.diff_hide_tests;
        let Some(diff) = self.diffs.get_mut(panel_key) else {
            return;
        };
        let scope = diff.scope;
        let collapsed = diff.collapsed.entry(scope).or_default();
        if let Some(Done::Ready(ready)) = &mut diff.done {
            ready.rows = flatten(&ready.files, hide, collapsed);
            ready.rows_version += 1;
        }
    }

    /// Apply a finished unit of background work. A stale request id means the
    /// answer belongs to a replaced request and is dropped (KSG's newer request
    /// winning over an older one).
    pub(crate) fn apply_diff_msg(&mut self, msg: DiffMsg) -> bool {
        match msg {
            DiffMsg::Preview {
                panel_key,
                request,
                done,
            } => {
                let Some(diff) = self.diffs.get(&panel_key) else {
                    return false;
                };
                if diff.request != request {
                    return false;
                }
                self.diff_apply_done(&panel_key, request, done);
                true
            }
            DiffMsg::File {
                panel_key,
                request,
                path,
                outcome,
            } => self.diff_apply_file(&panel_key, request, &path, outcome),
        }
    }
}

/// The `Event::Rpc` half of remote answers: deserialize, then parse on a worker
/// so the UI thread never does the patch work.
pub(crate) fn handle_rpc(
    state: &mut State,
    request: u64,
    result: Result<serde_json::Value, String>,
) {
    let Some(pending) = state.pending_diff_requests.remove(&request) else {
        return;
    };
    let parsed = result.and_then(|value| {
        serde_json::from_value::<ReviewDiffPreviewResult>(value)
            .map_err(|e| format!("Invalid review.getDiffPreview response: {e}"))
    });
    let (tx, rx) = std::sync::mpsc::channel();
    state.diff_rxs.push(rx);
    let wake = state.wake.clone();
    std::thread::spawn(move || {
        let msg = match pending {
            DiffPending::Preview {
                panel_key,
                scope,
                cwd,
            } => DiffMsg::Preview {
                panel_key,
                request,
                done: match parsed {
                    Ok(result) => preview_done(scope, &cwd, result),
                    Err(error) => Done::Failed(error),
                },
            },
            DiffPending::File {
                panel_key,
                path,
                preview_request,
            } => DiffMsg::File {
                panel_key,
                request: preview_request,
                outcome: match parsed {
                    Ok(result) => file_outcome(&path, result),
                    Err(error) => FileOutcome::Failed(error),
                },
                path,
            },
        };
        let _ = tx.send(msg);
        wake();
    });
}

/// `model::poll`'s hook: drains finished background work, watches provider
/// completion via `life.last_completed_at` (synced for remote threads), and
/// runs the queued refresh when the tab is visible and due.
pub(crate) fn tick(state: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    let mut remaining = Vec::with_capacity(state.diff_rxs.len());
    for rx in std::mem::take(&mut state.diff_rxs) {
        match rx.try_recv() {
            Ok(msg) => changed |= state.apply_diff_msg(msg),
            Err(std::sync::mpsc::TryRecvError::Empty) => remaining.push(rx),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
        }
    }
    state.diff_rxs = remaining;
    // A turn finishing bumps last_completed_at for local and remote threads.
    for (key, diff) in state.diffs.iter_mut() {
        let completed = state
            .threads
            .iter()
            .find(|t| crate::right_panel::key(&t.machine, t.id) == *key)
            .and_then(|t| t.life.last_completed_at);
        if diff.seen_completed_at.is_some() && diff.seen_completed_at != completed {
            diff.refresh_wanted = true;
        }
        diff.seen_completed_at = completed;
    }
    let panel_key = state.panel_key();
    let visible =
        state.right_panels.active_surface(&panel_key) == Some(crate::right_panel::Surface::Diff);
    if !visible {
        return changed;
    }
    let (wanted, due) = state.diffs.get(&panel_key).map_or((false, true), |d| {
        (
            d.refresh_wanted,
            refresh_due(d.last_refresh_at, Instant::now()),
        )
    });
    if wanted && due {
        state.diff_request_key(&panel_key);
        changed = true;
    } else if wanted {
        let wait = state
            .diffs
            .get(&panel_key)
            .and_then(|d| d.last_refresh_at)
            .map_or(Duration::ZERO, |at| {
                LIVE_REFRESH_MIN_INTERVAL.saturating_sub(at.elapsed())
            });
        tx.wake_after(wait);
    }
    changed
}

// --- view -------------------------------------------------------------------

/// KSG `formatCompactDiffCount`: exact to 999, then one-decimal k/m/b that
/// trims a trailing ".0" below 10.
fn format_compact_diff_count(value: u32) -> String {
    fn compact(value: f32, suffix: &str) -> String {
        if value < 10.0 {
            let mut text = format!("{value:.1}");
            if text.ends_with(".0") {
                text.truncate(text.len() - 2);
            }
            format!("{text}{suffix}")
        } else {
            format!("{}{suffix}", value.round() as u32)
        }
    }
    if value < 1_000 {
        value.to_string()
    } else if value < 999_500 {
        compact(value as f32 / 1_000.0, "k")
    } else if value < 999_500_000 {
        compact(value as f32 / 1_000_000.0, "m")
    } else {
        compact(value as f32 / 1_000_000_000.0, "b")
    }
}

const MONO: TextStyle = TextStyle::new(12.0, 400.0).family(rune::Family::SystemMono);
const MONO_SMALL: TextStyle = TextStyle::new(11.0, 400.0).family(rune::Family::SystemMono);
/// A code line's row height.
const LINE_H: f32 = 20.0;
/// A file section header's row height.
const FILE_H: f32 = 32.0;
/// The hunk header's row height.
const HUNK_H: f32 = 24.0;
/// The two line-number gutters plus the marker column before code text.
const GUTTER_W: f32 = 100.0;

/// KSG `DiffFileStatus`'s letter marks: A/D/M/R with the git-status colours
/// mapped to `Palette` (add = sea, delete = danger, modify/rename = muted).
fn status_mark(change: &ChangeKind, p: Palette) -> (&'static str, Color) {
    match change {
        ChangeKind::Added | ChangeKind::Copied => ("A", p.sea),
        ChangeKind::Deleted => ("D", p.danger),
        ChangeKind::Modified | ChangeKind::Renamed => (
            if matches!(change, ChangeKind::Renamed) {
                "R"
            } else {
                "M"
            },
            p.muted,
        ),
    }
}

/// The scope segmented toggle — KSG's scope control reduced to the two git
/// scopes, labels as in its scope menu.
fn scope_option(scope: Scope, current: Scope, key: &str, p: Palette) -> El {
    let on = scope == current;
    el().key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(format!("Diff scope: {}", scope.label()))
        .focusable(true)
        .center()
        .h(18.0)
        .px(8.0)
        .radius(3.0)
        .fill(if on { p.selected } else { Color::TRANSPARENT })
        .hover(if on { p.selected } else { p.card })
        .press(p.selected)
        .on_press(move |s: &mut State| s.diff_set_scope(scope))
        .child(label(
            scope.label(),
            PIXEL,
            if on { p.ink } else { p.muted },
        ))
}

/// KSG `DiffStatLabel`: `+N -N` in the addition and deletion colours, Departure
/// Mono, with its group aria label.
fn diff_stat_label(key: &str, additions: u32, deletions: u32, p: Palette) -> El {
    row()
        .key(key)
        .role("group")
        .label(format!("{additions} additions, {deletions} deletions"))
        .center_y()
        .gap(8.0)
        .child(
            pixel_text(format!("+{}", format_compact_diff_count(additions)), p.sea)
                .align(rune::Align::End)
                .w(28.0),
        )
        .child(
            pixel_text(
                format!("-{}", format_compact_diff_count(deletions)),
                p.danger,
            )
            .align(rune::Align::End)
            .w(28.0),
        )
}

/// KSG `DiffPanelShell`'s header: the scope toggle at the left; totals, the
/// test-file filter, collapse-all and refresh on the right.
pub(super) fn header(s: &State, p: Palette) -> El {
    let panel_key = s.panel_key();
    let diff = s.diffs.get(&panel_key);
    let scope = diff.map(|d| d.scope).unwrap_or_default();
    let loading = diff.is_some_and(|d| d.loading);
    let hide = s.diff_hide_tests;
    let ready = diff.and_then(ThreadDiff::ready);
    let shown: Vec<&FileEntry> = ready
        .map(|ready| {
            ready
                .files
                .iter()
                .filter(|entry| {
                    !(hide && crate::review_tree::is_test_changed_file(&entry.file.path))
                })
                .collect()
        })
        .unwrap_or_default();
    let collapsed = diff.and_then(|d| d.collapsed.get(&d.scope));
    let all_collapsed = collapsed.is_some_and(|set| {
        crate::review_patch::all_collapsed(shown.iter().map(|entry| &entry.file), set)
    });
    let has_tests = ready.is_some_and(|ready| {
        ready
            .files
            .iter()
            .any(|entry| crate::review_tree::is_test_changed_file(&entry.file.path))
    });

    let mut bar = row()
        .key("right-panel-header")
        .h(28.0)
        .px(8.0)
        .center_y()
        .gap(6.0)
        .child(
            row()
                .key("diff-scope")
                .center_y()
                .border(1.0, p.frame)
                .radius(4.0)
                .child(scope_option(
                    Scope::Uncommitted,
                    scope,
                    "diff-scope-uncommitted",
                    p,
                ))
                .child(el().w(1.0).h(16.0).fill(p.frame))
                .child(scope_option(Scope::Branch, scope, "diff-scope-branch", p)),
        )
        .child(spacer());
    // KSG shows the stat once files or stats exist (`codeViewFiles.length > 0
    // || selectedGitSource?.files?.length`).
    if ready.is_some_and(|ready| !ready.files.is_empty() || ready.additions + ready.deletions > 0) {
        let ready = ready.unwrap();
        bar = bar.child(diff_stat_label(
            "diff-stat",
            ready.additions,
            ready.deletions,
            p,
        ));
    }
    if has_tests {
        bar = bar.child(toggle_button(
            "diff-hide-tests",
            "Hide test files",
            "eye",
            s.diff_hide_tests,
            p,
            |s: &mut State| s.diff_toggle_hide_tests(),
        ));
    }
    if !shown.is_empty() {
        bar = bar.child(
            el().key("diff-collapse-all")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(if all_collapsed {
                    "Expand all files"
                } else {
                    "Collapse all files"
                })
                .focusable(true)
                .size(24.0, 24.0)
                .center()
                .radius(4.0)
                .hover(p.selected)
                .press(p.selected)
                .on_press(|s: &mut State| s.diff_toggle_all())
                .child(tool_icon(
                    if all_collapsed {
                        "expand"
                    } else {
                        "chevron-up"
                    },
                    p.muted,
                )),
        );
    }
    let reload = tool_icon("reload", p.muted);
    bar = bar.child(
        el().key("diff-refresh")
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .label(if loading {
                "Refreshing diff"
            } else {
                "Refresh diff"
            })
            .focusable(true)
            .size(24.0, 24.0)
            .center()
            .radius(4.0)
            .hover(p.selected)
            .press(p.selected)
            .on_press(|s: &mut State| s.diff_request())
            .child(if loading {
                reload.looping(rune::LoopSpec::rotation(1.0))
            } else {
                reload
            }),
    );
    bar.into()
}

/// A `p.frame` block of the loading skeleton — KSG `DiffPanelShell`'s
/// `Skeleton` rows drawn as pixel blocks.
fn block(width: f32, height: f32, p: Palette, pill: bool) -> El {
    el().w(width)
        .h(height)
        .radius(if pill { height / 2.0 } else { 4.0 })
        .fill(p.frame)
}

/// KSG `DiffFileHeaderSkeleton`: dot, mark block, title pill, two stat pills.
fn header_skeleton(width: f32, p: Palette) -> El {
    row()
        .h(FILE_H)
        .w_full()
        .center_y()
        .px(8.0)
        .gap(8.0)
        .child(
            el().size(20.0, 20.0)
                .center()
                .child(block(10.0, 10.0, p, false)),
        )
        .child(block(20.0, 20.0, p, false))
        .child(block(width, 12.0, p, true))
        .child(spacer())
        .child(block(20.0, 12.0, p, true))
        .child(block(20.0, 12.0, p, true))
}

/// KSG `DiffPanelLoadingState`: a file header skeleton, the hunk divider, three
/// code-line skeletons and two more file headers, shimmering.
fn loading_skeleton(scope: Scope, p: Palette) -> El {
    let mut lines = col().px(12.0).py(8.0).gap(8.0);
    for width in [140.0, 190.0, 110.0] {
        lines = lines.child(
            row()
                .center_y()
                .gap(12.0)
                .child(block(20.0, 10.0, p, true))
                .child(block(width, 10.0, p, true)),
        );
    }
    col()
        .key("diff-body")
        .role("status")
        .label(scope.loading_label())
        .w_full()
        .h_full()
        .looping(rune::LoopSpec::opacity(1.1, 0.0, 0.45, 0.95))
        .child(header_skeleton(220.0, p))
        .child(
            row()
                .h(24.0)
                .w_full()
                .center_y()
                .px(8.0)
                .gap(8.0)
                .child(el().h(1.0).grow(1.0).fill(p.line))
                .child(block(96.0, 10.0, p, true))
                .child(el().h(1.0).grow(1.0).fill(p.line)),
        )
        .child(lines)
        .child(header_skeleton(160.0, p))
        .child(header_skeleton(260.0, p))
}

/// A centered state: the message (KSG's wording), optionally a framed Retry.
fn centered_state(text: impl Into<String>, p: Palette, retry: bool) -> El {
    let mut body = col().center().w_full().h_full().px(16.0).gap(10.0).child(
        label(text.into(), SMALL, p.muted)
            .align(rune::Align::Center)
            .wrap(),
    );
    if retry {
        body = body.child(row().center().child(secondary_button(
            "diff-retry",
            "Retry",
            p,
            |s: &mut State| s.diff_request(),
        )));
    }
    body
}

/// The status mark of a file header, then the path with the directory part in
/// `p.muted` and the name in `p.ink` — `old → new` for a rename.
fn file_title(entry: &FileEntry, p: Palette) -> El {
    let mut title = row().center_y().min_w(0.0).shrink().gap(0.0);
    if let Some(previous) = &entry.file.previous_path {
        title = title.child(
            label(format!("{previous} → "), SMALL, p.muted)
                .truncate()
                .shrink(),
        );
    }
    let (dir, name) = entry
        .file
        .path
        .rsplit_once('/')
        .map(|(dir, name)| (format!("{dir}/"), name.to_string()))
        .unwrap_or_else(|| (String::new(), entry.file.path.clone()));
    title
        .child(label(dir, SMALL, p.muted).truncate().shrink())
        .child(label(name, SMALL, p.ink).truncate().shrink())
        .into()
}

/// A file section header: chevron, status mark, title, `+N -N`, press to fold.
/// KSG's `@pierre/diffs` file header row.
fn file_header(
    p: Palette,
    ready: &Ready,
    display: usize,
    file: usize,
    identity: &str,
    collapsed: &HashSet<String>,
    panel_key: &str,
) -> El {
    let entry = &ready.files[file];
    let is_collapsed = collapsed.contains(identity);
    let (letter, ink) = status_mark(&entry.file.change, p);
    let id = identity.to_string();
    let key = panel_key.to_string();
    let truncated = entry.truncated;
    let mut stat = row().center_y().gap(4.0);
    if truncated {
        stat = stat.child(pixel_text("truncated", p.brass));
    }
    if entry.additions > 0 || entry.deletions > 0 {
        stat = stat.child(diff_stat_inline(entry.additions, entry.deletions, p));
    }
    // The virtual row carries `diff-file-{display}`; the pressable row inside
    // it carries `diff-file-toggle-{display}`.
    row()
        .w_full()
        .h(FILE_H)
        .child(
            row()
                .key(format!("diff-file-toggle-{display}"))
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(entry.file.path.clone())
                .focusable(true)
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .center_y()
                .px(8.0)
                .gap(6.0)
                .hover(p.selected)
                .press(p.selected)
                .on_press(move |s: &mut State| s.diff_toggle_file(&key, &id))
                .child(
                    icon(if is_collapsed { "open" } else { "chevron" }, p.muted).size(12.0, 12.0),
                )
                .child(pixel_text(letter, ink).w(10.0))
                .child(file_title(entry, p))
                .child(spacer())
                .child(stat),
        )
        .into()
}

/// The file header's own stat: the KSG `aligned` layout, two right-aligned
/// cells — shown only when non-zero.
fn diff_stat_inline(additions: u32, deletions: u32, p: Palette) -> El {
    row()
        .role("group")
        .label(format!("{additions} additions, {deletions} deletions"))
        .center_y()
        .gap(8.0)
        .child(
            pixel_text(format!("+{}", format_compact_diff_count(additions)), p.sea)
                .align(rune::Align::End)
                .w(28.0),
        )
        .child(
            pixel_text(
                format!("-{}", format_compact_diff_count(deletions)),
                p.danger,
            )
            .align(rune::Align::End)
            .w(28.0),
        )
}

/// A number gutter cell: 44 px wide, right aligned, Departure Mono, sticky so
/// horizontally scrolled lines keep their numbers — KSG's sticky line-number
/// columns.
fn gutter(number: Option<u32>, bg: Color, p: Palette, left: f32) -> El {
    el().w(44.0)
        .h_full()
        .sticky_left(left)
        .fill(bg)
        .center_y()
        .px(6.0)
        .child(
            pixel_text(number.map(|n| n.to_string()).unwrap_or_default(), p.faint)
                .align(rune::Align::End)
                .w_full(),
        )
}

/// A hunk's `@@` header row: faint mono on `p.message_bg`.
fn hunk_header_row(text: &str, p: Palette) -> El {
    row()
        .w_full()
        .h(HUNK_H)
        .fill(p.message_bg)
        .center_y()
        .px(8.0)
        .child(label(text.to_string(), MONO_SMALL, p.faint).truncate())
        .into()
}

/// A diff line row: old number, new number, `+`/`-`/blank marker, mono text —
/// green or red tinted for added and deleted, selectable, never wrapping.
fn line_row(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str, p: Palette) -> El {
    let (bg, marker, marker_ink, text_ink) = match kind {
        LineKind::Added => (p.sea_bg, "+", p.sea, p.ink),
        LineKind::Deleted => (p.coral_bg, "-", p.danger, p.ink),
        LineKind::Context => (p.panel, "", p.faint, p.ink),
    };
    row()
        .w_full()
        .h(LINE_H)
        .fill(bg)
        .center_y()
        .child(gutter(old, bg, p, 0.0))
        .child(gutter(new, bg, p, 44.0))
        .child(
            pixel_text(marker, marker_ink)
                .align(rune::Align::Center)
                .w(12.0),
        )
        .child(label(text.to_string(), MONO, text_ink).selectable(true))
        .into()
}

/// The "No newline at end of file" faint row, aligned under the code text.
fn marker_row(p: Palette) -> El {
    row()
        .w_full()
        .h(LINE_H)
        .center_y()
        .child(el().w(GUTTER_W))
        .child(label(NO_NEWLINE_TEXT, SMALL, p.faint))
        .into()
}

/// A one-line note row (binary file, empty file, pending file, or a per-file
/// error that presses to retry).
fn note_row(
    text: &str,
    retry: bool,
    display: usize,
    p: Palette,
    panel_key: &str,
    identity: &str,
) -> El {
    if retry {
        let key = panel_key.to_string();
        let id = identity.to_string();
        row()
            .w_full()
            .h(LINE_H)
            .center_y()
            .px(8.0)
            .gap(8.0)
            .child(label(text.to_string(), SMALL, p.danger))
            .child(
                el().key(format!("diff-file-retry-{display}"))
                    .role("button")
                    .cursor(rune::Cursor::Pointer)
                    .label("Retry")
                    .focusable(true)
                    .center()
                    .h(18.0)
                    .px(8.0)
                    .radius(4.0)
                    .border(1.0, p.frame)
                    .fill(p.card)
                    .hover(p.selected)
                    .press(p.selected)
                    .on_press(move |s: &mut State| s.diff_retry_file(&key, &id))
                    .child(pixel_text("Retry", p.muted)),
            )
            .into()
    } else {
        row()
            .w_full()
            .h(LINE_H)
            .center_y()
            .child(el().w(GUTTER_W))
            .child(label(text.to_string(), SMALL, p.faint))
            .into()
    }
}

/// One row of the virtual list, built for the visible window only.
fn diff_row(s: &State, p: Palette, panel_key: &str, i: usize) -> El {
    let Some(diff) = s.diffs.get(panel_key) else {
        return el().into();
    };
    let Some(ready) = diff.ready() else {
        return el().into();
    };
    let Some(row_data) = ready.rows.get(i) else {
        return el().into();
    };
    let collapsed = diff.collapsed.get(&diff.scope).cloned().unwrap_or_default();
    match row_data {
        DiffRow::File {
            display,
            file,
            identity,
            ..
        } => file_header(p, ready, *display, *file, identity, &collapsed, panel_key),
        DiffRow::Hunk { text, .. } => hunk_header_row(text, p),
        DiffRow::Line {
            kind,
            old,
            new,
            text,
            ..
        } => line_row(*kind, *old, *new, text, p),
        DiffRow::Marker { .. } => marker_row(p),
        DiffRow::Note {
            file,
            display,
            text,
            retry,
            ..
        } => {
            let identity = crate::review_patch::identity_key(&ready.files[*file].file);
            note_row(text, *retry, *display, p, panel_key, &identity)
        }
        DiffRow::Boundary { remaining, .. } => row()
            .w_full()
            .h(28.0)
            .center()
            .looping(rune::LoopSpec::opacity(1.2, 0.0, 0.4, 0.95))
            .child(pixel_text(
                format!("Loading {remaining} file diffs..."),
                p.faint,
            ))
            .into(),
    }
}

/// KSG's ready state: notices, then either the raw patch or the virtualized
/// file list.
fn ready_body(p: Palette, panel_key: &str, ready: &Ready) -> El {
    let mut body = col().w_full().h_full().min_h(0.0).min_w(0.0);
    if ready.incomplete {
        body = body.child(el().w_full().px(8.0).py(6.0).child(label(
            INCOMPLETE_TEXT,
            SMALL,
            p.muted,
        )));
    }
    if ready.truncated {
        let mut text = TRUNCATED_TEXT.to_string();
        // KSG appends " Totals include all changes." when stats cover the
        // files the truncated patch could not.
        if !ready.files.is_empty() && !ready.incomplete {
            text.push_str(" Totals include all changes.");
        }
        body = body.child(
            el().w_full()
                .px(8.0)
                .py(6.0)
                .child(label(text, SMALL, p.muted).wrap()),
        );
    }
    if let Some(raw) = &ready.raw {
        return body
            .child(label(raw.reason.clone(), SMALL, p.muted).px(8.0).py(4.0))
            .child(
                col().grow(1.0).min_h(0.0).min_w(0.0).scroll_xy().child(
                    label(raw.text.clone(), MONO, p.ink)
                        .selectable(true)
                        .px(8.0)
                        .py(4.0),
                ),
            );
    }
    if ready.files.is_empty() {
        return body.child(centered_state(EMPTY_TEXT, p, false));
    }
    let row_count = ready.rows.len();
    let version = ready.rows_version;
    let key = panel_key.to_string();
    let key2 = panel_key.to_string();
    let key3 = panel_key.to_string();
    body.child(
        col()
            .key("diff-rows")
            .grow(1.0)
            .min_h(0.0)
            .min_w(0.0)
            .scroll_xy()
            .child(
                col()
                    .key("diff-rows-virtual")
                    .growth(Default::default())
                    .virtual_rows(
                        row_count,
                        LINE_H,
                        move |s: &State, i| -> Cow<'_, str> {
                            s.diffs
                                .get(&key)
                                .and_then(ThreadDiff::ready)
                                .and_then(|ready| ready.rows.get(i))
                                .map(|row| Cow::Borrowed(row.key()))
                                .unwrap_or_else(|| Cow::Owned(format!("diff-row-missing-{i}")))
                        },
                        move |s: &State, i| diff_row(s, palette(s.dark), &key2, i),
                    )
                    .virtual_rows_version(version)
                    // The list-end boundary becoming visible fetches the next
                    // batch, KSG's renderLoadingBoundary.
                    .on_visible_rows(move |s: &mut State, range| {
                        let needs =
                            s.diffs
                                .get(&key3)
                                .and_then(ThreadDiff::ready)
                                .is_some_and(|ready| {
                                    matches!(ready.rows.last(), Some(DiffRow::Boundary { .. }))
                                        && range.end >= ready.rows.len()
                                });
                        needs && s.diff_load_next(&key3)
                    }),
            ),
    )
}

/// The Diff tab body for the current thread. KSG `DiffPanel`'s state chain:
/// loading skeleton, the not-a-repository and capability notices, the error
/// state with retry, the empty states, then the ready content.
pub(super) fn body(s: &State, p: Palette) -> El {
    let panel_key = s.panel_key();
    let scope = s.diffs.get(&panel_key).map(|d| d.scope).unwrap_or_default();
    let body = match s.diffs.get(&panel_key).and_then(|diff| diff.done.as_ref()) {
        None => loading_skeleton(scope, p).into(),
        Some(Done::NotRepo) => centered_state(NOT_REPO_TEXT, p, false).into(),
        Some(Done::Unsupported(machine)) => centered_state(
            format!("Update Shina on {machine} to view changes"),
            p,
            false,
        )
        .into(),
        Some(Done::Failed(error)) => centered_state(error.clone(), p, true).into(),
        Some(Done::Empty) => centered_state(EMPTY_TEXT, p, false).into(),
        Some(Done::Missing) => centered_state(MISSING_TEXT, p, false).into(),
        Some(Done::Ready(ready)) => ready_body(p, &panel_key, ready).into(),
    };
    // Mounting the tab (or coming back to it) starts a read when the data is
    // stale — KSG `refreshOnShow` under the live-refresh floor.
    el().key("diff-body-root")
        .w_full()
        .h_full()
        .on_visible(move |s: &mut State, visible, tx| {
            if visible {
                s.diff_shown(tx);
            }
        })
        .child(body)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(kind: ReviewSourceKind, diff: &str) -> ReviewDiffPreviewSource {
        ReviewDiffPreviewSource {
            id: kind_name(kind).to_string(),
            kind,
            title: kind_name(kind).to_string(),
            commit_subject: None,
            base_ref: None,
            head_ref: None,
            diff: diff.to_string(),
            diff_hash: String::new(),
            truncated: false,
            patches_deferred: None,
            files: None,
        }
    }
    fn kind_name(kind: ReviewSourceKind) -> &'static str {
        match kind {
            ReviewSourceKind::WorkingTree => "working-tree",
            ReviewSourceKind::BranchRange => "branch-range",
            ReviewSourceKind::Commit => "commit",
        }
    }
    fn stat(path: &str, additions: usize, deletions: usize) -> ReviewDiffFileStat {
        ReviewDiffFileStat {
            path: path.to_string(),
            previous_path: None,
            additions,
            deletions,
        }
    }
    fn file(path: &str, change: ChangeKind) -> FileDiff {
        FileDiff {
            path: path.to_string(),
            previous_path: None,
            change,
            binary: false,
            old_mode: None,
            new_mode: None,
            hunks: vec![],
            additions: 0,
            deletions: 0,
        }
    }
    fn entry(file: FileDiff, body: FileBody) -> FileEntry {
        FileEntry {
            additions: file.additions,
            deletions: file.deletions,
            file,
            body,
            truncated: false,
        }
    }
    fn state() -> State {
        State::new(".".into(), false)
    }

    #[test]
    fn preview_input_sends_the_scope_fields_ksg_sends() {
        let uncommitted = preview_input(Scope::Uncommitted, "/repo");
        assert_eq!(uncommitted.cwd, "/repo");
        assert_eq!(
            uncommitted.source_kind,
            Some(ReviewPreviewSourceKind::WorkingTree)
        );
        assert_eq!(uncommitted.metadata_only, Some(true));
        assert_eq!(uncommitted.base_ref, None);
        assert_eq!(uncommitted.commit_sha, None);
        assert_eq!(uncommitted.file, None);
        assert_eq!(uncommitted.ignore_whitespace, Some(false));

        let branch = preview_input(Scope::Branch, "/repo");
        assert_eq!(
            branch.source_kind,
            Some(ReviewPreviewSourceKind::BranchRange)
        );
        assert_eq!(branch.base_ref, None);
        assert_eq!(branch.file, None);
    }

    #[test]
    fn a_late_answer_for_a_replaced_request_is_ignored() {
        let mut s = state();
        let key = s.panel_key();
        s.diffs.insert(
            key.clone(),
            ThreadDiff {
                request: 2,
                loading: true,
                ..ThreadDiff::default()
            },
        );
        assert!(!s.apply_diff_msg(DiffMsg::Preview {
            panel_key: key.clone(),
            request: 1,
            done: Done::Empty,
        }));
        assert!(s.diffs[&key].done.is_none());
        assert!(s.diffs[&key].loading);
        assert!(s.apply_diff_msg(DiffMsg::Preview {
            panel_key: key.clone(),
            request: 2,
            done: Done::Empty,
        }));
        assert!(matches!(s.diffs[&key].done, Some(Done::Empty)));
        assert!(!s.diffs[&key].loading);
    }

    #[test]
    fn refresh_respects_the_live_refresh_interval() {
        let now = Instant::now();
        assert!(refresh_due(None, now));
        assert!(refresh_due(
            Some(now - LIVE_REFRESH_MIN_INTERVAL - Duration::from_secs(1)),
            now
        ));
        assert!(!refresh_due(
            Some(now - LIVE_REFRESH_MIN_INTERVAL + Duration::from_secs(1)),
            now
        ));
    }

    #[test]
    fn hide_tests_filters_rows_but_never_totals() {
        let mut code = file("src/code.rs", ChangeKind::Modified);
        code.additions = 3;
        code.deletions = 1;
        let mut test = file("src/code.test.ts", ChangeKind::Modified);
        test.additions = 10;
        test.deletions = 2;
        let files = vec![entry(code, FileBody::Loaded), entry(test, FileBody::Loaded)];
        let collapsed = HashSet::new();
        let all = flatten(&files, false, &collapsed);
        let filtered = flatten(&files, true, &collapsed);
        let files_in = |rows: &[DiffRow]| {
            rows.iter()
                .filter(|row| matches!(row, DiffRow::File { .. }))
                .count()
        };
        assert_eq!(files_in(&all), 2);
        assert_eq!(files_in(&filtered), 1);
        // Totals count every changed file either way (KSG `diffLineStat`).
        let stats = crate::review_patch::line_stat(&[files[0].file.clone(), files[1].file.clone()]);
        assert_eq!((stats.additions, stats.deletions), (13, 3));
        assert!(crate::review_tree::is_test_changed_file("src/code.test.ts"));
        assert!(!crate::review_tree::is_test_changed_file("src/code.rs"));
    }

    #[test]
    fn collapse_tracks_file_identity_not_position() {
        let mut a = file("src/a.rs", ChangeKind::Modified);
        a.hunks.push(crate::review_patch::Hunk {
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
            section: String::new(),
            lines: vec![crate::review_patch::Line {
                kind: LineKind::Context,
                text: "x".into(),
                old_number: Some(1),
                new_number: Some(1),
                no_newline: false,
            }],
        });
        let mut b = file("src/b.rs", ChangeKind::Added);
        b.hunks.push(crate::review_patch::Hunk {
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: 1,
            section: String::new(),
            lines: vec![crate::review_patch::Line {
                kind: LineKind::Added,
                text: "y".into(),
                old_number: None,
                new_number: Some(1),
                no_newline: false,
            }],
        });
        let id_a = crate::review_patch::identity_key(&a);
        let mut collapsed = HashSet::new();
        collapsed.insert(id_a);
        let rows = flatten(
            &[
                entry(a.clone(), FileBody::Loaded),
                entry(b.clone(), FileBody::Loaded),
            ],
            false,
            &collapsed,
        );
        // a is folded: only its header and b's section rows appear.
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row, DiffRow::Hunk { file: 0, .. }))
        );
        assert!(
            rows.iter()
                .any(|row| matches!(row, DiffRow::Hunk { file: 1, .. }))
        );
        // Reordering the files keeps a folded: identity, not position.
        let rows = flatten(
            &[entry(b, FileBody::Loaded), entry(a, FileBody::Loaded)],
            false,
            &collapsed,
        );
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row, DiffRow::Hunk { file: 1, .. }))
        );
    }

    #[test]
    fn flatten_covers_hunks_binary_rename_and_no_newline() {
        let mut code = file("src/code.rs", ChangeKind::Modified);
        code.hunks = vec![
            crate::review_patch::Hunk {
                old_start: 1,
                old_count: 2,
                new_start: 1,
                new_count: 3,
                section: "fn main".into(),
                lines: vec![
                    crate::review_patch::Line {
                        kind: LineKind::Context,
                        text: "keep".into(),
                        old_number: Some(1),
                        new_number: Some(1),
                        no_newline: false,
                    },
                    crate::review_patch::Line {
                        kind: LineKind::Deleted,
                        text: "old".into(),
                        old_number: Some(2),
                        new_number: None,
                        no_newline: false,
                    },
                    crate::review_patch::Line {
                        kind: LineKind::Added,
                        text: "new".into(),
                        old_number: None,
                        new_number: Some(2),
                        no_newline: true,
                    },
                ],
            },
            crate::review_patch::Hunk {
                old_start: 9,
                old_count: 1,
                new_start: 10,
                new_count: 1,
                section: String::new(),
                lines: vec![crate::review_patch::Line {
                    kind: LineKind::Context,
                    text: "end".into(),
                    old_number: Some(9),
                    new_number: Some(10),
                    no_newline: false,
                }],
            },
        ];
        let mut binary = file("img/logo.png", ChangeKind::Modified);
        binary.binary = true;
        let mut renamed = file("src/new.rs", ChangeKind::Renamed);
        renamed.previous_path = Some("src/old.rs".into());
        let files = vec![
            entry(code, FileBody::Loaded),
            entry(binary, FileBody::Loaded),
            entry(renamed, FileBody::Loaded),
        ];
        let rows = flatten(&files, false, &HashSet::new());
        let kinds: Vec<&'static str> = rows
            .iter()
            .map(|row| match row {
                DiffRow::File { .. } => "file",
                DiffRow::Hunk { .. } => "hunk",
                DiffRow::Line { .. } => "line",
                DiffRow::Marker { .. } => "marker",
                DiffRow::Note { .. } => "note",
                DiffRow::Boundary { .. } => "boundary",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "file", "hunk", "line", "line", "line", "marker", "hunk", "line", "file", "note",
                "file", "note",
            ]
        );
        // The hunk headers keep git's `,1` elision and the section text.
        let headers: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                DiffRow::Hunk { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(headers, vec!["@@ -1,2 +1,3 @@ fn main", "@@ -9 +10 @@"]);
        // The rename note and the binary note say which.
        let notes: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                DiffRow::Note { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(notes, vec!["Binary file", "Renamed without changes."]);
    }

    #[test]
    fn preview_done_maps_sources_to_states() {
        let cwd = "/repo";
        // Not a repository.
        let result = ReviewDiffPreviewResult {
            cwd: cwd.into(),
            generated_at: String::new(),
            sources: vec![],
            incomplete: None,
        };
        assert!(matches!(
            preview_done(Scope::Branch, cwd, result),
            Done::NotRepo
        ));
        // The requested scope's source is missing.
        let result = ReviewDiffPreviewResult {
            cwd: cwd.into(),
            generated_at: String::new(),
            sources: vec![source(
                ReviewSourceKind::WorkingTree,
                "diff --git a/x b/x\n",
            )],
            incomplete: None,
        };
        assert!(matches!(
            preview_done(Scope::Branch, cwd, result),
            Done::Missing
        ));
        // An empty patch is the empty state.
        let result = ReviewDiffPreviewResult {
            cwd: cwd.into(),
            generated_at: String::new(),
            sources: vec![source(ReviewSourceKind::BranchRange, "")],
            incomplete: None,
        };
        assert!(matches!(
            preview_done(Scope::Branch, cwd, result),
            Done::Empty
        ));
        // Deferred stats make the lazy file list.
        let mut deferred = source(ReviewSourceKind::BranchRange, "");
        deferred.patches_deferred = Some(true);
        deferred.files = Some(vec![stat("src/a.rs", 2, 1), stat("tests/t.rs", 4, 0)]);
        let result = ReviewDiffPreviewResult {
            cwd: cwd.into(),
            generated_at: String::new(),
            sources: vec![deferred],
            incomplete: Some(true),
        };
        let Done::Ready(ready) = preview_done(Scope::Branch, cwd, result) else {
            panic!("deferred stats make a lazy Ready");
        };
        assert_eq!(ready.files.len(), 2);
        assert!(ready.files.iter().all(|e| e.body == FileBody::Pending));
        assert_eq!((ready.additions, ready.deletions), (6, 1));
        assert!(ready.incomplete);
        // An inline patch parses straight into loaded files.
        let mut inline = source(
            ReviewSourceKind::WorkingTree,
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n",
        );
        inline.files = None;
        let result = ReviewDiffPreviewResult {
            cwd: cwd.into(),
            generated_at: String::new(),
            sources: vec![inline],
            incomplete: None,
        };
        let Done::Ready(ready) = preview_done(Scope::Uncommitted, cwd, result) else {
            panic!("an inline patch makes a loaded Ready");
        };
        assert_eq!(ready.files.len(), 1);
        assert_eq!(ready.files[0].body, FileBody::Loaded);
    }

    #[test]
    fn file_outcome_merges_the_files_patch() {
        let result = ReviewDiffPreviewResult {
            cwd: "/repo".into(),
            generated_at: String::new(),
            sources: vec![source(
                ReviewSourceKind::BranchRange,
                "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\n",
            )],
            incomplete: None,
        };
        match file_outcome("src/a.rs", result.clone()) {
            FileOutcome::Loaded { file, .. } => assert_eq!(file.path, "src/a.rs"),
            _ => panic!("the matching file loads"),
        }
        assert!(matches!(
            file_outcome("src/other.rs", result),
            FileOutcome::Missing
        ));
        let empty = ReviewDiffPreviewResult {
            cwd: "/repo".into(),
            generated_at: String::new(),
            sources: vec![source(ReviewSourceKind::BranchRange, "noise\n")],
            incomplete: None,
        };
        assert!(matches!(
            file_outcome("src/a.rs", empty),
            FileOutcome::Missing
        ));
    }

    #[test]
    fn the_capability_notice_names_the_machine() {
        let mut s = state();
        let key = s.panel_key();
        s.diffs.insert(
            key.clone(),
            ThreadDiff {
                request: 1,
                done: Some(Done::Unsupported("Studio".into())),
                ..ThreadDiff::default()
            },
        );
        // The view string is produced in `body`; the notice shape is asserted
        // here on the state it renders.
        let Some(Done::Unsupported(label)) = &s.diffs[&key].done else {
            panic!("the unsupported outcome keeps its label");
        };
        assert_eq!(
            format!("Update Shina on {label} to view changes"),
            "Update Shina on Studio to view changes"
        );
    }

    #[test]
    fn format_compact_diff_count_matches_ksg() {
        assert_eq!(format_compact_diff_count(0), "0");
        assert_eq!(format_compact_diff_count(999), "999");
        assert_eq!(format_compact_diff_count(1000), "1k");
        assert_eq!(format_compact_diff_count(1234), "1.2k");
        assert_eq!(format_compact_diff_count(999_499), "999k");
        assert_eq!(format_compact_diff_count(1_000_000), "1m");
        assert_eq!(format_compact_diff_count(2_400_000), "2.4m");
    }
}
