//! The changed-files card under a finished turn, KSG Code's per-turn file list.
//!
//! Ported from KSG Code's `chat/ChangedFilesTree.tsx` (`ChangedFilesCard` for
//! the card and its header, `ChangedFilesTree` for the collapsible rows),
//! `chat/DiffStatLabel.tsx` for the `+N -N` labels, `lib/turnDiffTree.ts` for
//! the sibling order and the directories' summed stats, and
//! `chat/TimelineAssistantRows.tsx` (`AssistantChangedFilesSection`) for where
//! it mounts: under the turn's last assistant message, above its meta row,
//! once the turn has finished. Shina has no per-turn checkpoints and no
//! multi-repository turns, so the file list derives from the turn's own
//! file-change tool calls and `ChangedFilesGroupSection` is left out.

use super::crew_ui::{hover_wash, motion};
use super::diff_tree::DiffTreeRow;
use super::*;
use crate::review_tree::TreeStatus;
use crate::work::{FileChange, ToolKind};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// KSG's `h-7` tree row.
const ROW_H: f32 = 28.0;
/// The row's `paddingLeft` base and per-depth step (KSG `8 + depth * 14`).
const PAD_X: f32 = 8.0;
const LEVEL: f32 = 14.0;
/// KSG's `text-2xs` directory names.
const DIR_NAME: TextStyle = TextStyle::new(11.0, 500.0).lh(16.0);
/// KSG's `text-body` file names.
const FILE_NAME: TextStyle = TextStyle::new(14.0, 400.0).lh(20.0);
/// KSG's `text-caption tabular-nums` stat labels.
const STAT: TextStyle = TextStyle::new(11.0, 500.0).lh(16.0).features(TABULAR);
/// The card's `@container` breakpoint that hides the "Open diff" label (24 rem).
const LABEL_MIN_W: f32 = 384.0;

/// The card's folder state for one turn, as KSG keeps it: the header's
/// all-folders choice (`threadChangedFilesExpandedById`, `persistedExpanded ??
/// false`) and the tree's per-directory overrides (`directoryExpansionState`).
/// Session only; `expansion_key` matches KSG's `expansionStateKey`, so the
/// overrides reset whenever the all-folders choice or the directory set
/// changes.
#[derive(Default)]
pub(crate) struct CardUi {
    pub all_expanded: bool,
    pub expansion_key: Option<String>,
    pub overrides: HashMap<String, bool>,
}

/// One path a turn touched, as KSG's `TurnDiffFileChange`: the tree shows the
/// path relative to the working directory, the diff panel resolves the raw
/// tool path, and the stat holds the line counts when the tool data has them.
struct TurnFile {
    path: String,
    raw: String,
    stat: Option<(u32, u32)>,
}

/// The files a turn's file-change calls touched, deduped by path and ordered
/// as KSG's `buildTurnDiffTree` leaves its rows: directories before files,
/// siblings by natural name. A call with no structured `changes` still lists
/// the path it took, without counts.
fn turn_files(t: &Thread, root: &std::path::Path, start: usize, end: usize) -> Vec<TurnFile> {
    let mut files: Vec<TurnFile> = Vec::new();
    let mut index_by_path: HashMap<String, usize> = HashMap::new();
    let mut push = |raw: &str, stat: Option<(u32, u32)>| {
        let path = work_ui::relative(&raw.replace('\\', "/"), root);
        if path.is_empty() {
            return;
        }
        match (index_by_path.get(&path), stat) {
            (Some(&index), Some((add, del))) => {
                let stat = files[index].stat.get_or_insert((0, 0));
                stat.0 = stat.0.saturating_add(add);
                stat.1 = stat.1.saturating_add(del);
            }
            (Some(_), None) => {}
            (None, stat) => {
                index_by_path.insert(path.clone(), files.len());
                files.push(TurnFile {
                    path,
                    raw: raw.to_string(),
                    stat,
                });
            }
        }
    };
    for tool in &t.tools {
        if !(start..end).contains(&tool.anchor.message)
            || tool.failed
            || tool.resolved_kind() != ToolKind::FileChange
        {
            continue;
        }
        if tool.changes.is_empty() {
            let path = tool.input["path"]
                .as_str()
                .or_else(|| tool.input["file_path"].as_str())
                .unwrap_or(&tool.detail);
            push(path, None);
        } else {
            for change in &tool.changes {
                push(&change.path, Some(change_stat(change)));
            }
        }
    }
    files.sort_by(|a, b| tree_order(&a.path, &b.path));
    files
}

/// KSG `readStat`: the counts a change carries.
fn change_stat(change: &FileChange) -> (u32, u32) {
    (
        change.additions.min(u32::MAX as usize) as u32,
        change.deletions.min(u32::MAX as usize) as u32,
    )
}

/// KSG's `buildTurnDiffTree` order flattened to paths: at each level
/// directories come before files and both sort by `Intl.Collator` numeric
/// name, so a continuing segment always outranks a file segment before names
/// compare.
fn tree_order(a: &str, b: &str) -> Ordering {
    let a: Vec<&str> = a.split('/').collect();
    let b: Vec<&str> = b.split('/').collect();
    for i in 0..a.len().min(b.len()) {
        match (i + 1 < a.len(), i + 1 < b.len()) {
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => match natural_compare(a[i], b[i]) {
                Ordering::Equal => {}
                order => return order,
            },
        }
    }
    a.len().cmp(&b.len())
}

/// Case-insensitive name comparison with numeric runs, as KSG's
/// `Intl.Collator(undefined, { numeric: true, sensitivity: "base" })`. Digit
/// runs compare by value; letters compare lowercased (accents fold only as far
/// as `to_lowercase` takes them, close enough for file names).
fn natural_compare(a: &str, b: &str) -> Ordering {
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut run = String::new();
                    while let Some(c) = it.peek().copied().filter(|c| c.is_ascii_digit()) {
                        run.push(c);
                        it.next();
                    }
                    run
                };
                let num_a = run(&mut a).trim_start_matches('0').to_string();
                let num_b = run(&mut b).trim_start_matches('0').to_string();
                match num_a
                    .len()
                    .cmp(&num_b.len())
                    .then_with(|| num_a.cmp(&num_b))
                {
                    Ordering::Equal => {}
                    order => return order,
                }
            }
            (Some(x), Some(y)) => {
                a.next();
                b.next();
                match x
                    .to_lowercase()
                    .next()
                    .unwrap_or(x)
                    .cmp(&y.to_lowercase().next().unwrap_or(y))
                {
                    Ordering::Equal => {}
                    order => return order,
                }
            }
        }
    }
}

/// KSG's `expansionStateKey`: the all-folders choice and the tree's directory
/// paths; overrides apply only while the key still matches.
fn expansion_key(directories: &[String], all_expanded: bool) -> String {
    format!(
        "{}\u{0}{}",
        if all_expanded {
            "expanded"
        } else {
            "collapsed"
        },
        directories.join("\u{0}")
    )
}

/// A directory's expanded state: its override while `expansion_key` matches,
/// else the card's all-folders choice (KSG
/// `expandedDirectories[node.path] ?? allDirectoriesExpanded`).
fn dir_expanded(ui: Option<&CardUi>, expansion_key: &str, dir: &str, all_expanded: bool) -> bool {
    match ui.filter(|ui| ui.expansion_key.as_deref() == Some(expansion_key)) {
        Some(ui) => ui.overrides.get(dir).copied().unwrap_or(all_expanded),
        None => all_expanded,
    }
}

/// Bump the turn's revision so its memoized element rebuilds, as the work
/// fold's toggle does. The turn is named by its first message's id so index
/// shifts from loaded history cannot touch a different turn.
fn touch_turn(s: &mut State, turn_msg: Option<String>) {
    let Some(id) = turn_msg else {
        return;
    };
    let t = s.current_mut();
    if let Some(start) = t.messages.iter().position(|message| message.id == id) {
        t.timeline_cache.get_mut().touch_message(start);
    }
}

/// A directory row's press, KSG's `toggleDirectory`: the override flips what
/// the all-folders choice gives that path.
fn toggle_dir(s: &mut State, card_key: &str, turn_msg: Option<String>, key: &str, dir: &str) {
    let ui = s.changed_files.entry(card_key.to_string()).or_default();
    if ui.expansion_key.as_deref() != Some(key) {
        ui.expansion_key = Some(key.to_string());
        ui.overrides.clear();
    }
    let next = !ui.overrides.get(dir).copied().unwrap_or(ui.all_expanded);
    ui.overrides.insert(dir.to_string(), next);
    touch_turn(s, turn_msg);
}

/// The header chevrons' press, KSG's `onToggleAllDirectories`: the all-folders
/// choice flips and the per-directory overrides reset with it.
fn toggle_all(s: &mut State, card_key: &str, turn_msg: Option<String>) {
    let ui = s.changed_files.entry(card_key.to_string()).or_default();
    ui.all_expanded = !ui.all_expanded;
    ui.expansion_key = None;
    ui.overrides.clear();
    touch_turn(s, turn_msg);
}

/// KSG's `onOpenTurnDiff`: open the Changes tab on this file (the first file
/// for the header button), through the diff panel's reveal.
fn open_file(s: &mut State, path: &str) {
    let panel_key = s.panel_key();
    diff_panel::reveal_file(s, &panel_key, path);
}

/// KSG `DiffStatLabel` at the rows' `text-caption` size: right-aligned `+N`
/// and `-N` cells in the diff colours. The header uses the inline variant.
fn stat_label(key: String, stat: (u32, u32), p: Palette) -> El {
    row()
        .key(key)
        .role("group")
        .label(format!("{} additions, {} deletions", stat.0, stat.1))
        .center_y()
        .gap(8.0)
        .child(
            label(
                format!("+{}", diff_panel::format_compact_diff_count(stat.0)),
                STAT,
                p.ok,
            )
            .align(rune::Align::End)
            .w(24.0),
        )
        .child(
            label(
                format!("-{}", diff_panel::format_compact_diff_count(stat.1)),
                STAT,
                p.danger,
            )
            .align(rune::Align::End)
            .w(24.0),
        )
        .into()
}

/// KSG `DiffStatLabel layout="inline"` for the header's `text-xs` totals.
fn stat_inline(key: String, stat: (u32, u32), p: Palette) -> El {
    row()
        .key(key)
        .role("group")
        .label(format!("{} additions, {} deletions", stat.0, stat.1))
        .center_y()
        .gap(4.0)
        .child(label(
            format!("+{}", diff_panel::format_compact_diff_count(stat.0)),
            META,
            p.ok,
        ))
        .child(label(
            format!("-{}", diff_panel::format_compact_diff_count(stat.1)),
            META,
            p.danger,
        ))
        .into()
}

/// A tree row's shared chrome: KSG's `ThreadDetailsControl` row at `h-7`, its
/// `8 + depth * 14` indent, `surface-selected-soft` on hover and press, and
/// the `group-hover` ink on the chevron and name.
fn row_el(s: &State, p: Palette, key: String, name: String, depth: usize) -> El {
    row()
        .key(key)
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label(name)
        .focusable(true)
        .w_full()
        .h(ROW_H)
        .pl(PAD_X + depth as f32 * LEVEL)
        .pr(4.0)
        .center_y()
        .gap(10.0)
        .hover_group()
        .hover(hover_wash(s, p))
        .press(hover_wash(s, p))
        .hover_motion(motion(), motion())
}

/// One directory row: chevron, folder icon and the flattened chain name, with
/// the subtree's summed stat at the right. Ported from KSG's directory branch
/// of `ChangedFilesTree`'s `renderRow`.
fn dir_row(
    s: &State,
    p: Palette,
    card_key: &str,
    turn_key: &str,
    turn_msg: Option<String>,
    expansion_key: &str,
    path: &str,
    name: &str,
    depth: usize,
    stat: (u32, u32),
    expanded: bool,
) -> El {
    let key = card_key.to_string();
    let expand_key = expansion_key.to_string();
    let dir = path.to_string();
    let mut row = row_el(
        s,
        p,
        format!("changed-files-dir-{turn_key}-{path}"),
        name.to_string(),
        depth,
    )
    .expanded(expanded)
    .on_press(move |s: &mut State| toggle_dir(s, &key, turn_msg.clone(), &expand_key, &dir))
    .child(
        icon(if expanded { "chevron" } else { "open" }, p.muted)
            .group_color(p.ink)
            .size(12.0, 12.0),
    )
    .child(icon("folder", p.muted).size(12.0, 12.0))
    .child(
        label(name.to_string(), DIR_NAME, p.muted)
            .group_color(p.ink)
            .truncate()
            .shrink()
            .min_w(0.0),
    );
    if stat.0 > 0 || stat.1 > 0 {
        row = row.child(spacer()).child(stat_label(
            format!("changed-files-dir-stat-{turn_key}-{path}"),
            stat,
            p,
        ));
    }
    row.into()
}

/// One file row: the chevron slot spacer where the tree has directories, the
/// file icon, name and its stat, pressing through to the diff. Ported from
/// KSG's file branch of `ChangedFilesTree`'s `renderRow`.
fn file_row(
    s: &State,
    p: Palette,
    turn_key: &str,
    file: &TurnFile,
    depth: usize,
    spacer_slot: bool,
) -> El {
    let name = file.path.rsplit('/').next().unwrap_or(&file.path);
    let raw = file.raw.clone();
    let mut row = row_el(
        s,
        p,
        format!("changed-files-file-{turn_key}-{}", file.path),
        file.path.clone(),
        depth,
    )
    .on_press(move |s: &mut State| open_file(s, &raw));
    if spacer_slot {
        row = row.child(el().size(14.0, 14.0).shrink());
    }
    row = row.child(icon("file", p.muted).size(12.0, 12.0)).child(
        label(name.to_string(), FILE_NAME, p.ink)
            .truncate_middle()
            .shrink()
            .min_w(0.0),
    );
    if let Some(stat) = file.stat {
        row = row.child(spacer()).child(stat_label(
            format!("changed-files-file-stat-{turn_key}-{}", file.path),
            stat,
            p,
        ));
    }
    row.into()
}

/// The card itself, KSG's `ChangedFilesCard` reduced to the single tree Shina
/// can list: `mt-4 rounded-card surface-card` holding the header row and the
/// collapsible file tree. `None` when the turn changed no file.
pub(super) fn card(s: &State, p: Palette, start: usize, end: usize) -> Option<El> {
    let t = s.current();
    let turn_key = message_key(t, start);
    let card_key = format!("changed-files-{turn_key}");
    let files = turn_files(t, &s.working_directory(), start, end);
    if files.is_empty() {
        return None;
    }
    let ui = s.changed_files.get(&card_key);
    let all_expanded = ui.is_some_and(|ui| ui.all_expanded);
    // Every directory the files pass through, so `flatten_tree` can fold the
    // ones the expanded state leaves closed (KSG `collectExpandedRows`).
    let mut directories: Vec<String> = files
        .iter()
        .flat_map(|file| diff_tree::ancestor_dirs(&file.path))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    directories.sort();
    let expand_key = expansion_key(&directories, all_expanded);
    // KSG's overrides key on the row's own path, which a compacted chain
    // carries at its terminal directory. `flatten_tree` treats every member
    // of a collapsed chain as collapsed, so the set is filled per chain from
    // the terminal's expanded state.
    let mut dir_children: HashMap<String, Vec<String>> = HashMap::new();
    let mut dirs_with_files: HashSet<String> = HashSet::new();
    for file in &files {
        let mut parent = String::new();
        let segments: Vec<&str> = file.path.split('/').collect();
        for (index, segment) in segments.iter().enumerate() {
            if index + 1 == segments.len() {
                dirs_with_files.insert(parent.clone());
                break;
            }
            let child = format!("{parent}{segment}/");
            let children = dir_children.entry(parent.clone()).or_default();
            if !children.contains(&child) {
                children.push(child.clone());
            }
            parent = child;
        }
    }
    let terminal = |dir: &str| {
        let mut dir = dir.to_string();
        loop {
            let children = dir_children.get(&dir);
            if dirs_with_files.contains(&dir) || children.map(Vec::len) != Some(1) {
                return dir;
            }
            dir = children.expect("one child")[0].clone();
        }
    };
    let collapsed: HashSet<String> = directories
        .iter()
        .filter(|dir| !dir_expanded(ui, &expand_key, &terminal(dir), all_expanded))
        .cloned()
        .collect();
    let inputs: Vec<diff_tree::TreeFile<'_>> = files
        .iter()
        .map(|file| diff_tree::TreeFile {
            path: &file.path,
            status: TreeStatus::Modified,
            stat: file.stat,
        })
        .collect();
    let rows = diff_tree::flatten_tree(&inputs, false, &collapsed, "/");
    let has_directories = !directories.is_empty();
    let has_directory_nodes = rows
        .iter()
        .any(|row| matches!(row, DiffTreeRow::Dir { .. }));
    let count = files.len();
    let (additions, deletions) =
        files
            .iter()
            .fold((0u32, 0u32), |(add, del), file| match file.stat {
                Some((a, d)) => (add.saturating_add(a), del.saturating_add(d)),
                None => (add, del),
            });
    let turn_msg = t.messages.get(start).map(|message| message.id.clone());

    // The header: `{count} changed files`, the summed stat, the folders toggle
    // and "Open diff" (KSG `ChangedFilesCard`'s header, `px-4 py-2`).
    let mut title = row().center_y().gap(12.0).shrink().min_w(0.0).child(
        label(
            format!(
                "{count} changed {}",
                if count == 1 { "file" } else { "files" }
            ),
            META,
            p.ink,
        )
        .key(format!("changed-files-count-{turn_key}")),
    );
    if additions > 0 || deletions > 0 {
        title = title.child(stat_inline(
            format!("changed-files-totals-{turn_key}"),
            (additions, deletions),
            p,
        ));
    }
    let mut actions = row().center_y().gap(4.0);
    if has_directories {
        let key = card_key.clone();
        let msg = turn_msg.clone();
        actions = actions.child(
            el().key(format!("changed-files-toggle-{turn_key}"))
                .role("button")
                .cursor(rune::Cursor::Pointer)
                .label(if all_expanded {
                    "Collapse all folders"
                } else {
                    "Expand all folders"
                })
                .help(if all_expanded {
                    "Collapse all folders"
                } else {
                    "Expand all folders"
                })
                .focusable(true)
                .size(24.0, 24.0)
                .center()
                .radius(8.0)
                .hover(hover_wash(s, p))
                .press(hover_wash(s, p))
                .on_press(move |s: &mut State| toggle_all(s, &key, msg.clone()))
                .child(
                    icon(
                        if all_expanded {
                            "chevrons-down-up"
                        } else {
                            "chevrons-up-down"
                        },
                        p.muted,
                    )
                    .size(14.0, 14.0),
                ),
        );
    }
    // KSG's `@[24rem]/changed-files` container query hides the label on a
    // narrow card; the recorded width lags one frame, as `chat-lane` does.
    let wide_label = rune::memo::layout_size(&card_key)
        .map(|size| size[0] >= LABEL_MIN_W)
        .unwrap_or(true);
    let first_raw = files[0].raw.clone();
    let mut open = row()
        .key(format!("changed-files-open-diff-{turn_key}"))
        .role("button")
        .cursor(rune::Cursor::Pointer)
        .label("Open diff")
        .help("Open the full diff")
        .focusable(true)
        .center_y()
        .h(24.0)
        .px(7.0)
        .gap(4.0)
        .radius(8.0)
        .fill(p.row)
        .press(hover_wash(s, p))
        .on_press(move |s: &mut State| open_file(s, &first_raw))
        .child(icon("diff", p.ink).size(12.0, 12.0));
    if wide_label {
        open = open.child(label("Open diff", META, p.ink));
    }
    actions = actions.child(framed(open, p));

    let mut tree = col()
        .key(format!("changed-files-rows-{turn_key}"))
        .min_w(0.0);
    for row in &rows {
        match row {
            DiffTreeRow::Dir {
                path,
                name,
                depth,
                stat,
                collapsed,
                ..
            } => {
                tree = tree.child(dir_row(
                    s,
                    p,
                    &card_key,
                    &turn_key,
                    turn_msg.clone(),
                    &expand_key,
                    path,
                    name,
                    *depth,
                    *stat,
                    !collapsed,
                ));
            }
            DiffTreeRow::File { file, depth, .. } => {
                tree = tree.child(file_row(
                    s,
                    p,
                    &turn_key,
                    &files[*file],
                    *depth,
                    has_directory_nodes || *depth > 0,
                ));
            }
        }
    }

    Some(
        col()
            .key(card_key)
            .role("group")
            .label(format!(
                "{count} changed {}",
                if count == 1 { "file" } else { "files" }
            ))
            .min_w(0.0)
            // KSG `mt-4` above the card, over the meta row's `mt-1.5`.
            .mt(4.0)
            .radius(12.0)
            .clip_rounded(12.0)
            .fill(p.card)
            .child(
                row()
                    .key(format!("changed-files-header-{turn_key}"))
                    .w_full()
                    .min_w(0.0)
                    .px(16.0)
                    .py(8.0)
                    .center_y()
                    .gap(8.0)
                    .child(title)
                    .child(spacer())
                    .child(actions),
            )
            .child(tree)
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_order_puts_directories_before_files_and_names_naturally() {
        let mut paths = vec![
            "src/z.rs",
            "b.rs",
            "src/a2.rs",
            "src/a10.rs",
            "a.rs",
            "docs/b.md",
            "docs/a.md",
        ];
        paths.sort_by(|a, b| tree_order(a, b));
        assert_eq!(
            paths,
            vec![
                "docs/a.md",
                "docs/b.md",
                "src/a2.rs",
                "src/a10.rs",
                "src/z.rs",
                "a.rs",
                "b.rs",
            ]
        );
    }
}
