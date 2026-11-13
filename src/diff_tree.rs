//! The Diff tab's file tree: the folder column beside the diff list.
//!
//! Ported from KSG Code's `components/diffs/DiffFileTree.tsx` (the tree of
//! changed files), `components/files/FileExplorerPane.tsx` (the resizable
//! aside it mounts in) and the scroll-tracking half of
//! `components/diffs/useCodeViewFileReveal.ts`. Where KSG renders Pierre's
//! `FileTree`, Shina flattens the same row model — the diff list's
//! `review_tree::order_like_tree` order — into `virtual_rows`, which is less
//! code here than adapting the rune-kit tree to per-index diff state.

use super::diff_panel::{DiffRow, ThreadDiff};
use super::*;
use crate::review_tree::TreeStatus;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// KSG's compact `itemHeight` (`pierre-tree-theme` `--trees-item-height`).
pub(crate) const ROW_H: f32 = 24.0;
/// KSG `FileExplorerPane`'s `TREE_MIN_WIDTH`, `TREE_MAX_WIDTH` and the
/// `TREE_CONTENT_MIN_WIDTH` it reserves for the diff list.
pub(crate) const MIN_W: f32 = 160.0;
pub(crate) const MAX_W: f32 = 560.0;
const RESERVE_W: f32 = 256.0;
/// KSG's `defaultWidthFraction` and `defaultWidth` for the explorer pane.
const FRACTION: f32 = 0.4;
const DEFAULT_W: f32 = 256.0;
/// Below this diff-lane width the tree and its toggle hide outright: KSG
/// never names the threshold, but its own bounds imply it — the tree's
/// minimum plus the diff's content reserve.
pub(crate) const MIN_LANE: f32 = MIN_W + RESERVE_W;
/// The row's left inset: KSG `--trees-item-padding-x` (8px) at the compact
/// 0.8 density factor.
const PAD_X: f32 = 6.4;
/// KSG's per-level indent: each level's spacing item plus the level gap it
/// keeps on its right (~18.7 px at the compact factor).
const LEVEL_INDENT: f32 = 18.7;
/// The first level's indent: the spacing item plus KSG's extra
/// `--trees-indent-first-level-gap`.
const FIRST_LEVEL_INDENT: f32 = 20.3;
/// The resize handle's hit width, the right panel handle's 4 px.
const HANDLE_W: f32 = 4.0;
/// The handle's 16 px keyboard step, the right panel's.
const KEYBOARD_STEP: f32 = 16.0;

/// A row of the flattened tree. `File` rows point into `Ready::files` by
/// `file` and carry `display`, the shown-file ordinal `diff-file-{display}`
/// keys use. `Dir` rows carry the flattened chain: `path` is the chain's
/// deepest directory (`a/b/c/`) and `name` its segments joined `a / b / c`,
/// as Pierre's `flattenEmptyDirectories` shows them. `parent` is the row
/// index of the enclosing directory row for Left's "move to parent".
/// `collapsed` folds any chain member the collapse set holds.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffTreeRow {
    Dir {
        path: String,
        name: String,
        depth: usize,
        parent: Option<usize>,
        collapsed: bool,
    },
    File {
        file: usize,
        display: usize,
        depth: usize,
        parent: Option<usize>,
        status: TreeStatus,
    },
}

impl DiffTreeRow {
    fn depth(&self) -> usize {
        match self {
            Self::Dir { depth, .. } | Self::File { depth, .. } => *depth,
        }
    }
    fn parent(&self) -> Option<usize> {
        match self {
            Self::Dir { parent, .. } | Self::File { parent, .. } => *parent,
        }
    }
    /// `diff-tree-file-{display}` lines up with the diff list's
    /// `diff-file-{display}` keys; directories key by path.
    pub fn key(&self) -> String {
        match self {
            Self::Dir { path, .. } => format!("diff-tree-dir-{path}"),
            Self::File { display, .. } => format!("diff-tree-file-{display}"),
        }
    }
}

/// KSG's `toGitStatus`: the tree's A/D/M/R mark for a change kind.
fn tree_status(change: &crate::review_patch::ChangeKind) -> TreeStatus {
    use crate::review_patch::ChangeKind as K;
    match change {
        K::Added | K::Copied => TreeStatus::Added,
        K::Deleted => TreeStatus::Deleted,
        K::Renamed => TreeStatus::Renamed,
        K::Modified => TreeStatus::Modified,
    }
}

/// Every ancestor directory of a path (`a/b/c.rs` -> `a/`, `a/b/`; the same
/// for the directory `a/b/c/`).
pub(crate) fn ancestor_dirs(path: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut directory = String::new();
    let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    for segment in &segments[..segments.len().saturating_sub(1)] {
        directory.push_str(segment);
        directory.push('/');
        dirs.push(directory.clone());
    }
    dirs
}

/// Any prefix directory of `dir_path` — `dir_path` included — the collapse
/// set holds. A flattened chain collapses when any member does, the way
/// Pierre renders a flattened node open only while every member is expanded.
fn dir_collapsed(collapsed: &HashSet<String>, dir_path: &str) -> bool {
    ancestor_dirs(dir_path)
        .iter()
        .chain(std::iter::once(&dir_path.to_string()))
        .any(|dir| collapsed.contains(dir))
}

/// A directory node of the shown tree; children keep first-seen order, which
/// is `review_tree`'s position ordering that KSG's comparator reproduces.
struct DirNode {
    path: String,
    children: Vec<DirChild>,
}

enum DirChild {
    Dir(DirNode),
    File {
        file: usize,
        display: usize,
        status: TreeStatus,
    },
}

impl DirNode {
    fn new(path: String) -> Self {
        Self {
            path,
            children: Vec::new(),
        }
    }
    fn dir_child(&mut self, path: String) -> &mut DirNode {
        if let Some(index) = self
            .children
            .iter()
            .position(|child| matches!(child, DirChild::Dir(dir) if dir.path == path))
        {
            let DirChild::Dir(dir) = &mut self.children[index] else {
                unreachable!()
            };
            return dir;
        }
        self.children.push(DirChild::Dir(DirNode::new(path)));
        let DirChild::Dir(dir) = self.children.last_mut().unwrap() else {
            unreachable!()
        };
        dir
    }
}

/// The visible tree rows for a ready diff. `files` is `Ready::files` in
/// `order_like_tree` order; KSG hands the tree that same order and ranks
/// siblings by first-seen position, which insertion order reproduces here.
/// `hide_tests` drops test files and the folders they leave empty (a folder
/// with no shown descendants is never an ancestor of a shown file, so it
/// never exists in the model). `dirs_collapsed` prunes a folder's subtree
/// while keeping its row.
pub(crate) fn flatten_tree(
    files: &[diff_panel::FileEntry],
    hide_tests: bool,
    dirs_collapsed: &HashSet<String>,
) -> Vec<DiffTreeRow> {
    // Dedupe paths the way KSG's `diffFileTreeEntries` does: one row per
    // path, a differing status folding to Modified; `display` is the file's
    // shown ordinal in the diff list.
    let mut status_by_path: HashMap<&str, TreeStatus> = HashMap::new();
    let mut unique: Vec<(usize, usize, &str)> = Vec::new();
    let mut display = 0usize;
    for (index, entry) in files.iter().enumerate() {
        if hide_tests && crate::review_tree::is_test_changed_file(&entry.file.path) {
            continue;
        }
        let ordinal = display;
        display += 1;
        let path = entry.file.path.as_str();
        let status = tree_status(&entry.file.change);
        match status_by_path.get_mut(path) {
            Some(prev) => {
                if *prev != status {
                    *prev = TreeStatus::Modified;
                }
            }
            None => {
                status_by_path.insert(path, status);
                unique.push((index, ordinal, path));
            }
        }
    }
    let mut root = DirNode::new(String::new());
    for &(index, ordinal, path) in &unique {
        let mut node = &mut root;
        let mut directory = String::new();
        for segment in path.split('/').take(path.matches('/').count()) {
            directory.push_str(segment);
            directory.push('/');
            node = node.dir_child(directory.clone());
        }
        node.children.push(DirChild::File {
            file: index,
            display: ordinal,
            status: status_by_path[path],
        });
    }
    let mut out = Vec::new();
    emit_children(&root, 0, None, dirs_collapsed, &mut out);
    out
}

/// Emit a directory's children in first-seen order. A directory child first
/// follows its single-child chain to the terminal directory — KSG's
/// flattening of `a/b/c` into one row — then recurses while expanded.
fn emit_children(
    node: &DirNode,
    depth: usize,
    parent: Option<usize>,
    dirs_collapsed: &HashSet<String>,
    out: &mut Vec<DiffTreeRow>,
) {
    for child in &node.children {
        match child {
            DirChild::File {
                file,
                display,
                status,
            } => out.push(DiffTreeRow::File {
                file: *file,
                display: *display,
                depth,
                parent,
                status: *status,
            }),
            DirChild::Dir(dir) => {
                let mut terminal = dir;
                loop {
                    if terminal.children.len() != 1 {
                        break;
                    }
                    let DirChild::Dir(next) = &terminal.children[0] else {
                        break;
                    };
                    terminal = next;
                }
                let collapsed = dir_collapsed(dirs_collapsed, &terminal.path);
                // The shown name is every segment of the chain, joined like
                // KSG's formatted chain ("a / b / c").
                let name = terminal.path[node.path.len()..]
                    .trim_end_matches('/')
                    .split('/')
                    .collect::<Vec<_>>()
                    .join(" / ");
                let row = out.len();
                out.push(DiffTreeRow::Dir {
                    path: terminal.path.clone(),
                    name,
                    depth,
                    parent,
                    collapsed,
                });
                if !collapsed {
                    emit_children(terminal, depth + 1, Some(row), dirs_collapsed, out);
                }
            }
        }
    }
}

/// The file whose section tops the diff list at `start`: the row there names
/// its file directly; the trailing `Boundary` row belongs to the last file.
/// KSG's `lastViewedFileKey` tracking reduced to the flat rows Shina has.
pub(crate) fn current_file_at(rows: &[DiffRow], start: usize) -> Option<usize> {
    let row = rows.get(start.min(rows.len().saturating_sub(1)))?;
    match row {
        DiffRow::File { file, .. }
        | DiffRow::Hunk { file, .. }
        | DiffRow::Line { file, .. }
        | DiffRow::Marker { file, .. }
        | DiffRow::Note { file, .. } => Some(*file),
        DiffRow::Boundary { .. } => rows.iter().rev().find_map(|row| match row {
            DiffRow::File { file, .. }
            | DiffRow::Hunk { file, .. }
            | DiffRow::Line { file, .. }
            | DiffRow::Marker { file, .. }
            | DiffRow::Note { file, .. } => Some(*file),
            DiffRow::Boundary { .. } => None,
        }),
    }
}

/// The `diff-tree-file-*` key a file index maps to, when it has a row.
pub(crate) fn file_row_key(ready: &diff_panel::Ready, index: usize) -> Option<String> {
    ready.tree_rows.iter().find_map(|row| match row {
        DiffTreeRow::File { file, .. } if *file == index => Some(row.key()),
        _ => None,
    })
}

/// KSG's width math for the pane: `max(min, min(default, lane * 0.4))` when
/// no width was chosen, else the chosen width clamped to `min..=max(lane)`.
fn tree_width(lane: f32, chosen: Option<f32>) -> f32 {
    let default = MIN_W.max((lane * FRACTION).min(DEFAULT_W));
    match chosen {
        Some(width) => width.clamp(MIN_W, tree_max_width(lane)),
        None => default,
    }
}

/// KSG `FileExplorerPane`: `max(min, min(560, max(lane - 256, default)))`.
fn tree_max_width(lane: f32) -> f32 {
    let default = MIN_W.max((lane * FRACTION).min(DEFAULT_W));
    MIN_W.max(MAX_W.min((lane - RESERVE_W).max(default)))
}

/// The diff lane's width: the right panel's last committed layout less its
/// 2 px frame. `Cx::layout_size` reads one frame behind, the way KSG's pane
/// reads `parentElement.clientWidth` after mount.
pub(crate) fn lane_width() -> f32 {
    rune::memo::layout_size("right-panel")
        .map(|size| size[0] - 2.0)
        .unwrap_or(0.0)
}

impl State {
    /// The header toggle (KSG `fileTreeOpen`): app-wide, saved only when it
    /// differs from KSG's hidden default.
    pub(crate) fn diff_tree_toggle(&mut self) {
        self.diff_tree_open = !self.diff_tree_open;
        self.checkpoint();
    }

    /// A folder row's press (KSG's directory `onSelect`): chains collapse and
    /// expand as one node.
    pub(crate) fn diff_toggle_dir(&mut self, panel_key: &str, dir_path: &str) {
        let Some(diff) = self.diffs.get_mut(panel_key) else {
            return;
        };
        if dir_collapsed(&diff.dirs_collapsed, dir_path) {
            let mut prefixes = ancestor_dirs(dir_path);
            prefixes.push(dir_path.to_string());
            diff.dirs_collapsed.retain(|dir| !prefixes.contains(dir));
        } else {
            diff.dirs_collapsed.insert(dir_path.to_string());
        }
        self.diff_rebuild_tree(panel_key);
    }

    /// The header's chevrons button (KSG `setAllDirectoriesExpanded` over
    /// `directoryPaths`): collapse every folder row, or clear the set.
    pub(crate) fn diff_toggle_all_dirs(&mut self, panel_key: &str) {
        let Some(diff) = self.diffs.get_mut(panel_key) else {
            return;
        };
        let Some(ready) = diff.ready() else {
            return;
        };
        let paths: Vec<String> = ready
            .tree_rows
            .iter()
            .filter_map(|row| match row {
                DiffTreeRow::Dir { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        let all_open = !ready.tree_rows.iter().any(|row| {
            matches!(
                row,
                DiffTreeRow::Dir {
                    collapsed: true,
                    ..
                }
            )
        });
        if all_open {
            for path in paths {
                diff.dirs_collapsed.insert(path);
            }
        } else {
            diff.dirs_collapsed.clear();
        }
        self.diff_rebuild_tree(panel_key);
    }

    /// Rebuild the tree's flat rows after file, filter or collapse changes.
    pub(crate) fn diff_rebuild_tree(&mut self, panel_key: &str) {
        let hide = self.diff_hide_tests;
        let Some(diff) = self.diffs.get_mut(panel_key) else {
            return;
        };
        // `ready_mut` borrows all of `diff`; take the set out for the call.
        let collapsed = std::mem::take(&mut diff.dirs_collapsed);
        if let Some(ready) = diff.ready_mut() {
            ready.tree_rows = flatten_tree(&ready.files, hide, &collapsed);
            ready.tree_version += 1;
        }
        diff.dirs_collapsed = collapsed;
    }

    /// The diff list's first visible row moved: mark the file whose section
    /// tops the viewport (KSG's `lastViewedFileKey` tracking) and keep its
    /// tree row scrolled into view. A pending reveal owns the position — as
    /// KSG's hook ignores user scrolling while its reveal is in flight.
    pub(crate) fn diff_note_visible(&mut self, panel_key: &str, start: usize) {
        let Some(diff) = self.diffs.get_mut(panel_key) else {
            return;
        };
        if diff.reveal_row.is_some() {
            return;
        }
        let Some(ready) = diff.ready() else {
            return;
        };
        let next = current_file_at(&ready.rows, start);
        if diff.current_file == next {
            return;
        }
        let sync = next.and_then(|index| file_row_key(ready, index));
        diff.current_file = next;
        diff.tree_sync = sync;
    }

    /// A tree row's press: folders fold; files reveal their diff section
    /// through `apply_reveal`, KSG's `onSelectFile` -> `revealDiffFile`.
    /// Pressing the already-current file reveals again, as KSG's
    /// `onClickCapture` forwards the click Pierre's selection callback drops.
    /// The handler resolves the row by its key: virtual rows keep built
    /// elements across rebuilds, so a captured index can go stale.
    pub(crate) fn diff_tree_press(&mut self, panel_key: &str, row_key: &str, tx: &mut Tx) {
        enum Act {
            Toggle(String),
            Reveal(String),
        }
        let act = {
            let Some(diff) = self.diffs.get_mut(panel_key) else {
                return;
            };
            let Some(ready) = diff.ready() else {
                return;
            };
            let Some(index) = ready.tree_rows.iter().position(|row| row.key() == row_key) else {
                return;
            };
            let row = &ready.tree_rows[index];
            let act = match row {
                DiffTreeRow::Dir { path, .. } => Act::Toggle(path.clone()),
                DiffTreeRow::File { file, .. } => Act::Reveal(ready.files[*file].file.path.clone()),
            };
            // `row`'s borrow of `diff` ends here.
            diff.tree_cursor = index;
            act
        };
        // Clicking a row hands the tree focus so the arrows work, KSG's row
        // tab stop.
        tx.focus("diff-tree", false);
        match act {
            Act::Toggle(path) => self.diff_toggle_dir(panel_key, &path),
            Act::Reveal(path) => diff_panel::apply_reveal(self, panel_key, &path),
        }
    }

    /// The tree's key handling while it has focus (KSG/Pierre's `handleKeyDown`).
    pub(crate) fn diff_tree_key(
        &mut self,
        panel_key: &str,
        ev: &rune::KeyEvent,
        tx: &mut Tx,
    ) -> bool {
        enum Act {
            Move(usize),
            Toggle(String),
            Reveal(String),
        }
        let Some(diff) = self.diffs.get_mut(panel_key) else {
            return false;
        };
        let Some(ready) = diff.ready() else {
            return false;
        };
        let len = ready.tree_rows.len();
        if len == 0 {
            return false;
        }
        let cursor = diff.tree_cursor.min(len - 1);
        let row = &ready.tree_rows[cursor];
        // `on_key` gives Space as text.
        let space = matches!(&ev.key, rune::Key::Text(text) if text == " ");
        let act = match ev.key {
            rune::Key::Up => Act::Move(cursor.saturating_sub(1)),
            rune::Key::Down => Act::Move((cursor + 1).min(len - 1)),
            rune::Key::Home => Act::Move(0),
            rune::Key::End => Act::Move(len - 1),
            rune::Key::Left => match row {
                DiffTreeRow::Dir {
                    path, collapsed, ..
                } if !collapsed => Act::Toggle(path.clone()),
                _ => Act::Move(row.parent().unwrap_or(cursor)),
            },
            rune::Key::Right => match row {
                DiffTreeRow::Dir {
                    path,
                    collapsed: true,
                    ..
                } => Act::Toggle(path.clone()),
                DiffTreeRow::Dir { .. } => Act::Move((cursor + 1).min(len - 1)),
                DiffTreeRow::File { .. } => return true,
            },
            _ if space || ev.key == rune::Key::Enter => match row {
                DiffTreeRow::Dir { path, .. } => Act::Toggle(path.clone()),
                DiffTreeRow::File { file, .. } => Act::Reveal(ready.files[*file].file.path.clone()),
            },
            _ => return false,
        };
        match act {
            Act::Move(next) => {
                let key = ready.tree_rows[next].key();
                diff.tree_cursor = next;
                tx.scroll_into_view_in("diff-tree-rows-virtual", key);
            }
            Act::Toggle(path) => self.diff_toggle_dir(panel_key, &path),
            Act::Reveal(path) => diff_panel::apply_reveal(self, panel_key, &path),
        }
        true
    }
}

// --- view ------------------------------------------------------------------

/// KSG `FileExplorerPane`'s aside: the "Files" header, then the virtualized
/// tree, with the resize handle on its left edge like KSG's.
pub(super) fn column(s: &State, p: Palette, panel_key: &str) -> El {
    let lane = lane_width();
    let width = tree_width(lane, s.diff_tree_width);
    let Some(diff) = s.diffs.get(panel_key) else {
        return el().into();
    };
    let Some(ready) = diff.ready() else {
        return el().into();
    };
    let count = ready.tree_rows.len();
    let version = ready.tree_version;
    let syncing = diff.tree_sync.clone();
    let key = panel_key.to_string();
    let key2 = panel_key.to_string();
    let key3 = panel_key.to_string();
    let key4 = panel_key.to_string();
    let mut root = col()
        .key("diff-tree")
        .w(width)
        .shrink()
        .h_full()
        .min_h(0.0)
        .focusable(true)
        .role("tree")
        .label("File tree")
        .on_key(move |s: &mut State, ev, tx| s.diff_tree_key(&key, ev, tx))
        // The 1 px `p.frame` line KSG's `border-l` draws beside the diff.
        .child(
            el().pin(Some(0.0), Some(0.0), None, Some(0.0))
                .w(1.0)
                .h_full()
                .fill(p.frame),
        )
        .child(header(p, &key2, ready))
        .child(el().h(12.0))
        .child(
            col()
                .key("diff-tree-rows")
                .grow(1.0)
                .min_h(0.0)
                .min_w(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .key("diff-tree-rows-virtual")
                        .growth(Default::default())
                        .virtual_rows(
                            count,
                            ROW_H,
                            move |s: &State, i| -> Cow<'_, str> {
                                s.diffs
                                    .get(&key3)
                                    .and_then(ThreadDiff::ready)
                                    .and_then(|ready| ready.tree_rows.get(i))
                                    .map(|row| Cow::Owned(row.key()))
                                    .unwrap_or_else(|| Cow::Owned(format!("diff-tree-missing-{i}")))
                            },
                            move |s: &State, i| tree_row(s, palette(s.dark), &key4, i),
                        )
                        .virtual_rows_version(version),
                ),
        )
        .child(resize_handle(s, p, lane));
    if let Some(row_key) = syncing {
        root = root.child(sync_marker(panel_key, &row_key));
    }
    root.into()
}

/// The `mb-3` inline tree header: "Files", the shown-file count at the right,
/// and the collapse/expand-all-folders button while directories exist.
fn header(p: Palette, panel_key: &str, ready: &diff_panel::Ready) -> El {
    let files = ready
        .tree_rows
        .iter()
        .filter(|row| matches!(row, DiffTreeRow::File { .. }))
        .count();
    let dirs = ready
        .tree_rows
        .iter()
        .any(|row| matches!(row, DiffTreeRow::Dir { .. }));
    let all_open = !ready.tree_rows.iter().any(|row| {
        matches!(
            row,
            DiffTreeRow::Dir {
                collapsed: true,
                ..
            }
        )
    });
    let mut bar = row()
        .key("diff-tree-header")
        .h(28.0)
        .px(8.0)
        .center_y()
        .gap(4.0)
        .child(label("Files", SMALL, p.ink).px(4.0))
        .child(spacer())
        .child(pixel_text(files.to_string(), p.muted));
    if dirs {
        let panel_key = panel_key.to_string();
        bar = bar.child(
            el().key("diff-tree-dirs-all")
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(if all_open {
                    "Collapse all folders"
                } else {
                    "Expand all folders"
                })
                .focusable(true)
                .size(24.0, 24.0)
                .center()
                .radius(4.0)
                .hover(p.selected)
                .press(p.selected)
                .on_press(move |s: &mut State| s.diff_toggle_all_dirs(&panel_key))
                .child(tool_icon(
                    if all_open { "chevron-up" } else { "expand" },
                    p.muted,
                )),
        );
    }
    bar.into()
}

/// One tree row: a folder's chevron, icon and (possibly chained) name, or a
/// file's icon, name and right-edge status mark. The file the diff's scroll
/// position marks current gets `p.selected` plus the 2 px coral bar KSG's
/// `border-primary` draws.
fn tree_row(s: &State, p: Palette, panel_key: &str, i: usize) -> El {
    let Some(diff) = s.diffs.get(panel_key) else {
        return el().into();
    };
    let Some(ready) = diff.ready() else {
        return el().into();
    };
    let Some(row) = ready.tree_rows.get(i) else {
        return el().into();
    };
    let cursor = diff.tree_cursor == i;
    let indent = PAD_X
        + match row.depth() {
            0 => 0.0,
            depth => FIRST_LEVEL_INDENT + (depth - 1) as f32 * LEVEL_INDENT,
        };
    let key = panel_key.to_string();
    match row {
        DiffTreeRow::Dir {
            name, collapsed, ..
        } => {
            let open = !collapsed;
            row_el(row.key(), key, indent, cursor, p)
                .role("treeitem")
                .label(name.clone())
                .expanded(open)
                .level(row.depth() as u32 + 1)
                .child(icon(if open { "chevron" } else { "open" }, p.muted).size(12.0, 12.0))
                .child(icon("folder", p.muted).size(16.0, 16.0))
                .child(label(name.clone(), SMALL, p.ink).truncate().shrink())
                .into()
        }
        DiffTreeRow::File { file, status, .. } => {
            let entry = &ready.files[*file];
            let current = diff.current_file.is_some_and(|current| {
                ready
                    .files
                    .get(current)
                    .is_some_and(|other| other.file.path == entry.file.path)
            });
            let name = entry
                .file
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&entry.file.path)
                .to_string();
            let (letter, ink) = tree_status_mark(*status, p);
            let mut file_row = row_el(row.key(), key, indent, cursor, p)
                .role("treeitem")
                .label(entry.file.path.clone())
                .focus_selected(current)
                .level(row.depth() as u32 + 1)
                .child(icon("file", p.muted).size(16.0, 16.0))
                .child(label(name, SMALL, p.ink).truncate().shrink())
                .child(spacer())
                .child(pixel_text(letter, ink).pr(8.0));
            if current {
                // KSG's `data-[active]` row: selected fill plus the 2 px
                // left-edge bar.
                file_row = file_row.fill(p.selected).child(
                    el().pin(Some(0.0), Some(0.0), None, Some(0.0))
                        .w(2.0)
                        .h_full()
                        .fill(p.coral),
                );
            }
            file_row.into()
        }
    }
}

/// The shared row chrome: height, indent, hover/selected fills, the coral
/// outline the focused row gets, and the press that selects and acts.
fn row_el(key: String, panel_key: String, indent: f32, cursor: bool, p: Palette) -> El {
    let row = row()
        .key(key.clone())
        .w_full()
        .h(ROW_H)
        .pl(indent)
        .center_y()
        .gap(4.0)
        .cursor(rune::Cursor::Pointer)
        .focusable(false)
        .hover(p.selected)
        .press(p.selected)
        .on_press_tx(move |s: &mut State, tx| s.diff_tree_press(&panel_key, &key, tx));
    if cursor {
        row.border(1.0, p.coral)
    } else {
        row
    }
}

/// KSG `RightPanelResizeHandle` for the tree pane: a 4 px strip on the pane's
/// left edge that drags the tree wider leftward, with the same keyboard
/// sizing (Left wider, Right narrower, Home/End the bounds) and a 1 px line
/// that goes coral while dragging.
fn resize_handle(s: &State, p: Palette, lane: f32) -> El {
    let max = tree_max_width(lane);
    el().key("diff-tree-resize")
        .role("separator")
        .cursor(rune::Cursor::ResizeColumn)
        .label("Resize file tree")
        .description(
            "Drag to resize. Left and Right arrows adjust 16 points. Home smallest. End largest.",
        )
        .a11y_value(format!("{:.0} points", tree_width(lane, s.diff_tree_width)))
        .focusable(true)
        .w(HANDLE_W)
        .h_full()
        .pin(Some(0.0), Some(0.0), None, Some(0.0))
        .on_drag(move |s: &mut State, drag, _| {
            if drag.phase == rune::Phase::Down {
                drag.data = tree_width(lane, s.diff_tree_width);
                s.diff_tree_resizing = true;
            }
            if drag.cancelled() {
                s.diff_tree_width = Some(drag.data);
            } else {
                let width = (drag.data - drag.dx / s.zoom.clamp(0.5, 2.0)).clamp(MIN_W, max);
                s.diff_tree_width = Some(width);
            }
            if drag.phase == rune::Phase::Up {
                s.diff_tree_resizing = false;
                s.checkpoint();
            }
        })
        .on_key(move |s: &mut State, ev, _| {
            let width = match ev.key {
                rune::Key::Left => Some(tree_width(lane, s.diff_tree_width) + KEYBOARD_STEP),
                rune::Key::Right => Some(tree_width(lane, s.diff_tree_width) - KEYBOARD_STEP),
                rune::Key::Home => Some(MIN_W),
                rune::Key::End => Some(max),
                _ => None,
            };
            if let Some(width) = width {
                s.diff_tree_width = Some(width.clamp(MIN_W, max));
                s.checkpoint();
            }
            width.is_some()
        })
        .center()
        .child(el().w(1.0).h_full().fill(if s.diff_tree_resizing {
            p.coral
        } else {
            Color::TRANSPARENT
        }))
        .into()
}

/// A one-shot scroll trigger for the tracked current file: mounting it
/// scrolls the named tree row into view and clears the flag.
fn sync_marker(panel_key: &str, row_key: &str) -> El {
    let panel = panel_key.to_string();
    el().key(format!("diff-tree-sync-{row_key}"))
        .a11y_hidden()
        .absolute(0.0, 0.0)
        .size(1.0, 1.0)
        .on_visible(move |s: &mut State, visible, tx| {
            if !visible {
                return;
            }
            let Some(row) = s
                .diffs
                .get_mut(&panel)
                .and_then(|diff| diff.tree_sync.take())
            else {
                return;
            };
            tx.instant();
            tx.scroll_into_view_in("diff-tree-rows-virtual", row);
        })
        .into()
}

/// The mark the file's status gets, KSG's `gitStatus` lane rendered with
/// `diff_panel::status_mark`'s palette mapping.
fn tree_status_mark(status: TreeStatus, p: Palette) -> (&'static str, Color) {
    match status {
        TreeStatus::Added => ("A", p.sea),
        TreeStatus::Deleted => ("D", p.danger),
        TreeStatus::Modified => ("M", p.muted),
        TreeStatus::Renamed => ("R", p.muted),
    }
}

#[cfg(test)]
mod tests {
    use super::diff_panel::{FileBody, FileEntry};
    use super::*;
    use crate::review_patch::{ChangeKind, FileDiff};

    fn file(_index: usize, change: ChangeKind, path: &str) -> FileEntry {
        FileEntry {
            additions: 1,
            deletions: 0,
            file: FileDiff {
                path: path.to_string(),
                previous_path: None,
                change,
                binary: false,
                old_mode: None,
                new_mode: None,
                hunks: Vec::new(),
                additions: 1,
                deletions: 0,
            },
            body: FileBody::Loaded,
            truncated: false,
        }
    }

    /// The six-file fixture across three folder levels, already in
    /// `order_like_tree` order as `Ready::files` holds them.
    fn six_files() -> Vec<FileEntry> {
        vec![
            file(0, ChangeKind::Modified, "README.md"),
            file(1, ChangeKind::Modified, "src/lib.rs"),
            file(2, ChangeKind::Added, "src/ui/app.rs"),
            file(3, ChangeKind::Modified, "src/ui/widgets/list.rs"),
            file(4, ChangeKind::Deleted, "tests/app.test.ts"),
            file(5, ChangeKind::Modified, "src/ui/widgets/deep/item.rs"),
        ]
    }

    fn row_names(files: &[FileEntry], rows: &[DiffTreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                DiffTreeRow::Dir { name, .. } => format!("dir:{name}"),
                DiffTreeRow::File { file, .. } => {
                    let path = &files[*file].file.path;
                    format!("file:{}", path.rsplit('/').next().unwrap_or(path))
                }
            })
            .collect()
    }

    #[test]
    fn tree_rows_order_depth_and_folder_rows() {
        let rows = flatten_tree(&six_files(), false, &HashSet::new());
        // src/lib.rs first-seens src/ ahead of README.md? No: README.md is at
        // index 0, so the root file comes first, then src/'s subtree.
        let names: Vec<&str> = rows
            .iter()
            .map(|row| match row {
                DiffTreeRow::Dir { name, .. } => name.as_str(),
                DiffTreeRow::File { file, .. } => match file {
                    0 => "README.md",
                    1 => "lib.rs",
                    2 => "app.rs",
                    3 => "list.rs",
                    4 => "app.test.ts",
                    5 => "item.rs",
                    _ => "?",
                },
            })
            .collect();
        // src/ui/app.rs and src/ui/widgets/* put `src / ui` on one row; the
        // widgets chain still forks at list.rs/deep, so `widgets` is a row.
        assert_eq!(
            names,
            vec![
                "README.md",
                "src",
                "lib.rs",
                "ui",
                "app.rs",
                "widgets",
                "list.rs",
                "deep",
                "item.rs",
                "tests",
                "app.test.ts"
            ]
        );
        // Depths: README 0, src 0, lib.rs 1, ui 1, app.rs 2, widgets 2,
        // list.rs 3, deep 3, item.rs 4, tests 0, app.test.ts 1.
        let depths: Vec<usize> = rows.iter().map(|row| row.depth()).collect();
        assert_eq!(depths, vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 0, 1]);
        let dirs: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                DiffTreeRow::Dir { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            dirs,
            vec![
                "src/",
                "src/ui/",
                "src/ui/widgets/",
                "src/ui/widgets/deep/",
                "tests/"
            ]
        );
    }

    #[test]
    fn collapsing_a_folder_hides_its_descendants() {
        let files = six_files();
        let collapsed = HashSet::from(["src/ui/".to_string()]);
        let rows = flatten_tree(&files, false, &collapsed);
        let names = row_names(&files, &rows);
        assert_eq!(
            names,
            vec![
                "file:README.md",
                "dir:src",
                "file:lib.rs",
                "dir:ui",
                "dir:tests",
                "file:app.test.ts"
            ]
        );
        // The row keeps its collapsed mark and the same depth.
        let DiffTreeRow::Dir {
            collapsed, depth, ..
        } = &rows[3]
        else {
            panic!("src/ui/ stays a folder row");
        };
        assert!(*collapsed && *depth == 1);
        // Re-opening restores the subtree.
        let open = flatten_tree(&files, false, &HashSet::new());
        assert!(open.len() > rows.len());
    }

    #[test]
    fn hide_tests_removes_files_and_empty_folders() {
        let files = six_files();
        let rows = flatten_tree(&files, true, &HashSet::new());
        let names = row_names(&files, &rows);
        assert!(
            !names
                .iter()
                .any(|name| name.contains("app.test.ts") || name == "dir:tests")
        );
        assert_eq!(
            names,
            vec![
                "file:README.md",
                "dir:src",
                "file:lib.rs",
                "dir:ui",
                "file:app.rs",
                "dir:widgets",
                "file:list.rs",
                "dir:deep",
                "file:item.rs"
            ]
        );
    }

    #[test]
    fn single_child_folder_chains_show_joined_names() {
        // Every level holds only the next: KSG draws the whole run as one
        // "a / b / c / d" row keyed on its deepest directory.
        let files = vec![
            file(0, ChangeKind::Modified, "a/b/c/d/leaf.rs"),
            file(1, ChangeKind::Modified, "root.rs"),
        ];
        let rows = flatten_tree(&files, false, &HashSet::new());
        let names = row_names(&files, &rows);
        assert_eq!(
            names,
            vec!["dir:a / b / c / d", "file:leaf.rs", "file:root.rs"]
        );
        let DiffTreeRow::Dir { path, depth, .. } = &rows[0] else {
            panic!("the chain is a folder row");
        };
        assert_eq!(path, "a/b/c/d/");
        assert_eq!(*depth, 0);
        assert_eq!(rows[0].key(), "diff-tree-dir-a/b/c/d/");

        // A fork ends the chain: `a` folds into `b` only as far as `b`'s two
        // children allow, so `c` gets its own row.
        let files = vec![
            file(0, ChangeKind::Modified, "a/b/c/leaf.rs"),
            file(1, ChangeKind::Modified, "a/b/sibling.rs"),
        ];
        let names = row_names(&files, &flatten_tree(&files, false, &HashSet::new()));
        assert_eq!(
            names,
            vec!["dir:a / b", "dir:c", "file:leaf.rs", "file:sibling.rs"]
        );
    }

    #[test]
    fn current_file_tracks_the_row_at_the_top() {
        let rows = vec![
            DiffRow::File {
                display: 0,
                file: 0,
                identity: "a".into(),
                key: "f0".into(),
            },
            DiffRow::Line {
                file: 0,
                kind: crate::review_patch::LineKind::Added,
                old: None,
                new: Some(1),
                text: "x".into(),
                key: "l0".into(),
            },
            DiffRow::File {
                display: 1,
                file: 1,
                identity: "b".into(),
                key: "f1".into(),
            },
            DiffRow::Line {
                file: 1,
                kind: crate::review_patch::LineKind::Added,
                old: None,
                new: Some(1),
                text: "y".into(),
                key: "l1".into(),
            },
            DiffRow::Boundary {
                remaining: 0,
                key: "b".into(),
            },
        ];
        assert_eq!(current_file_at(&rows, 0), Some(0));
        assert_eq!(current_file_at(&rows, 1), Some(0));
        assert_eq!(current_file_at(&rows, 2), Some(1));
        assert_eq!(current_file_at(&rows, 3), Some(1));
        assert_eq!(current_file_at(&rows, 4), Some(1));
        assert_eq!(current_file_at(&rows, 9), Some(1));
    }

    #[test]
    fn built_row_keys_are_stable_across_collapse() {
        let files = six_files();
        let rows = flatten_tree(&files, false, &HashSet::new());
        let keys: Vec<String> = rows.iter().map(|row| row.key()).collect();
        assert!(keys.contains(&"diff-tree-file-3".to_string()));
        assert!(keys.contains(&"diff-tree-dir-src/ui/".to_string()));
        let collapsed = HashSet::from(["src/ui/widgets/deep/".to_string()]);
        let after = flatten_tree(&files, false, &collapsed);
        let after_keys: Vec<String> = after.iter().map(|row| row.key()).collect();
        for key in keys
            .iter()
            .take_while(|k| k.as_str() != "diff-tree-dir-src/ui/widgets/deep/")
        {
            assert!(after_keys.contains(key), "{key}");
        }
    }

    #[test]
    fn five_thousand_files_build_bounded_rows() {
        // One row per shown path plus its branch folders — the row list stays
        // linear in files, and `virtual_rows` bounds the built elements.
        let files: Vec<FileEntry> = (0..5000)
            .map(|i| {
                file(
                    i,
                    ChangeKind::Modified,
                    &format!("dir{}/sub/file{}.rs", i % 50, i),
                )
            })
            .collect();
        let rows = flatten_tree(&files, false, &HashSet::new());
        let file_rows = rows
            .iter()
            .filter(|row| matches!(row, DiffTreeRow::File { .. }))
            .count();
        assert_eq!(file_rows, 5000);
        // dirX holds only subX, so each folds into a "dirX / sub" chain row.
        assert_eq!(rows.len() - file_rows, 50);
    }

    #[test]
    fn widths_follow_ksg_bounds() {
        // Narrow lane: `min(256, lane * 0.4)` picks the fraction.
        assert!((tree_width(538.0, None) - 215.2).abs() < 0.5);
        // Wide lane: the 256 px default caps the fraction.
        assert_eq!(tree_width(1400.0, None), DEFAULT_W);
        // Chosen widths clamp to the lane's `min(560, lane - 256)` bound.
        assert_eq!(tree_width(538.0, Some(900.0)), 282.0);
        assert_eq!(tree_width(538.0, Some(1.0)), MIN_W);
        assert_eq!(tree_max_width(2000.0), MAX_W);
    }

    #[test]
    fn duplicate_paths_fold_to_one_row_and_modified() {
        // KSG's `diffFileTreeEntries` dedupes by path; differing statuses
        // fold to Modified.
        let files = vec![
            file(0, ChangeKind::Added, "src/dup.rs"),
            file(1, ChangeKind::Deleted, "src/dup.rs"),
            file(2, ChangeKind::Modified, "src/other.rs"),
        ];
        let rows = flatten_tree(&files, false, &HashSet::new());
        let names = row_names(&files, &rows);
        assert_eq!(names, vec!["dir:src", "file:dup.rs", "file:other.rs"]);
        let DiffTreeRow::File { file, status, .. } = &rows[1] else {
            panic!("the dup row is a file");
        };
        assert_eq!(*file, 0);
        assert_eq!(*status, TreeStatus::Modified);
    }

    #[test]
    fn keyboard_navigation_moves_expands_and_reveals() {
        let mut s = State::new(".".into(), false);
        let key = s.panel_key();
        s.diffs.insert(
            key.clone(),
            diff_panel::ThreadDiff {
                done: Some(diff_panel::Done::Ready(Box::new(diff_panel::Ready {
                    files: six_files(),
                    raw: None,
                    additions: 6,
                    deletions: 0,
                    truncated: false,
                    incomplete: false,
                    cwd: "/repo".into(),
                    source_kind: crate::review::ReviewSourceKind::BranchRange,
                    base_ref: None,
                    rows: vec![],
                    rows_version: 0,
                    tree_rows: vec![],
                    tree_version: 0,
                }))),
                ..Default::default()
            },
        );
        s.diff_rebuild_rows(&key);
        let ev = |key: rune::Key| rune::KeyEvent {
            key,
            mods: Default::default(),
        };
        let mut tx = Tx::default();
        // Down moves the cursor; Up returns.
        assert!(s.diff_tree_key(&key, &ev(rune::Key::Down), &mut tx));
        assert!(s.diff_tree_key(&key, &ev(rune::Key::Down), &mut tx));
        assert_eq!(s.diffs[&key].tree_cursor, 2);
        assert!(s.diff_tree_key(&key, &ev(rune::Key::Up), &mut tx));
        assert_eq!(s.diffs[&key].tree_cursor, 1);
        // Left on the expanded `src/` row collapses it.
        assert!(s.diff_tree_key(&key, &ev(rune::Key::Left), &mut tx));
        assert!(s.diffs[&key].dirs_collapsed.contains("src/"));
        // Right on the collapsed row expands it again.
        assert!(s.diff_tree_key(&key, &ev(rune::Key::Right), &mut tx));
        assert!(!s.diffs[&key].dirs_collapsed.contains("src/"));
        // End lands on the last row, Home on the first.
        assert!(s.diff_tree_key(&key, &ev(rune::Key::End), &mut tx));
        let last = s.diffs[&key].ready().unwrap().tree_rows.len() - 1;
        assert_eq!(s.diffs[&key].tree_cursor, last);
        assert!(s.diff_tree_key(&key, &ev(rune::Key::Home), &mut tx));
        assert_eq!(s.diffs[&key].tree_cursor, 0);
        // Enter on the file row reveals it through the same path the tree's
        // press takes.
        assert!(s.diff_tree_key(&key, &ev(rune::Key::Enter), &mut tx));
        assert_eq!(s.diffs[&key].current_file, Some(0));
        assert!(s.diffs[&key].reveal_row.is_some());
        // Space activates the same way (on_key gives it as text).
        s.diffs.get_mut(&key).unwrap().reveal_row = None;
        assert!(s.diff_tree_key(&key, &ev(rune::Key::Text(" ".into())), &mut tx));
        assert!(s.diffs[&key].reveal_row.is_some());
    }
}
