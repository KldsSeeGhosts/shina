//! KSG Code's list continuation, indentation, outdentation, and renumbering,
//! ported from composer-list-continuation.ts and composer-rich-text-doc.ts.

use rune::{BlockAttributes, BlockKind, Key, KeyEvent, MAX_LIST_LEVEL, TextEdit};

pub fn is_list(kind: BlockKind) -> bool {
    matches!(
        kind,
        BlockKind::Bullet(_) | BlockKind::Numbered(_) | BlockKind::Task { .. }
    )
}

pub fn list_level(kind: BlockKind) -> Option<u8> {
    match kind {
        BlockKind::Bullet(l) | BlockKind::Numbered(l) | BlockKind::Task { level: l, .. } => Some(l),
        _ => None,
    }
}

pub fn with_list_level(kind: BlockKind, level: u8) -> BlockKind {
    match kind {
        BlockKind::Bullet(_) => BlockKind::Bullet(level),
        BlockKind::Numbered(_) => BlockKind::Numbered(level),
        BlockKind::Task { checked, .. } => BlockKind::Task { level, checked },
        other => other,
    }
}

pub fn indent_width(s: &str, from_col: usize) -> usize {
    let mut col = from_col;
    for ch in s.chars() {
        if ch == ' ' {
            col += 1;
        } else if ch == '\t' {
            col += 4 - (col % 4);
        } else {
            break;
        }
    }
    col - from_col
}

pub fn effective_indent(edit: &TextEdit, line: usize) -> String {
    if let Some(ind) = edit.block_attributes(line).indent() {
        return ind.to_owned();
    }
    let level = list_level(edit.block_kind(line)).unwrap_or(0);
    "    ".repeat(level as usize)
}

pub fn next_ordered_marker(marker: &str) -> String {
    let Some(delim) = marker.chars().last().filter(|c| matches!(c, '.' | ')')) else {
        return "1.".to_owned();
    };
    let num_str = &marker[..marker.len() - 1];
    let Ok(num) = num_str.parse::<u64>() else {
        return marker.to_owned();
    };
    let Some(next) = num.checked_add(1) else {
        return marker.to_owned();
    };
    let next_str = next.to_string();
    if num_str.starts_with('0') && next_str.len() < num_str.len() {
        format!("{next_str:0>width$}{delim}", width = num_str.len())
    } else {
        format!("{next_str}{delim}")
    }
}

fn marker_number(marker: &str) -> Option<(u64, char)> {
    let delim = marker.chars().last().filter(|c| matches!(c, '.' | ')'))?;
    let num = marker[..marker.len() - 1].parse::<u64>().ok()?;
    Some((num, delim))
}

fn format_ordered_marker(cur_marker: &str, number: u64, delim: char) -> String {
    let cur_num = cur_marker.strip_suffix(['.', ')']).unwrap_or(cur_marker);
    let num_str = number.to_string();
    if cur_num.starts_with('0') && num_str.len() < cur_num.len() {
        format!("{num_str:0>width$}{delim}", width = cur_num.len())
    } else {
        format!("{num_str}{delim}")
    }
}

fn content_column(edit: &TextEdit, line: usize) -> usize {
    let indent = effective_indent(edit, line);
    let ind_w = indent_width(&indent, 0);
    let (marker_w, space) = match edit.block_kind(line) {
        BlockKind::Bullet(_) => (1, edit.block_attributes(line).space().unwrap_or(" ")),
        BlockKind::Numbered(_) => (
            edit.block_attributes(line)
                .marker()
                .filter(|s| !s.is_empty())
                .unwrap_or("1.")
                .len(),
            edit.block_attributes(line).space().unwrap_or(" "),
        ),
        BlockKind::Task { .. } => (1, edit.block_attributes(line).marker_space().unwrap_or(" ")),
        _ => return 0,
    };
    let marker_end = ind_w + marker_w;
    let sp = indent_width(space, marker_end);
    marker_end + if (1..=4).contains(&sp) { sp } else { 1 }
}

fn is_soft_break(edit: &TextEdit, line: usize) -> bool {
    edit.soft_breaks().binary_search(&(line as u32)).is_ok()
}

fn renumber_following(
    edit: &mut TextEdit,
    after_line: usize,
    base_level: u8,
    delim: char,
    start_num: u64,
) {
    let mut expected = start_num;
    let total = edit.blocks().len();
    for i in (after_line + 1)..total {
        if is_soft_break(edit, i) {
            continue;
        }
        let kind = edit.block_kind(i);
        let Some(lvl) = list_level(kind) else {
            break;
        };
        if lvl < base_level {
            break;
        }
        if lvl > base_level {
            continue;
        }
        if !matches!(kind, BlockKind::Numbered(_)) {
            break;
        }
        let cur_attrs = edit.block_attributes(i).clone();
        let cur_marker = cur_attrs.marker().filter(|s| !s.is_empty()).unwrap_or("1.");
        if !cur_marker.ends_with(delim) {
            break;
        }
        let next_marker = format_ordered_marker(cur_marker, expected, delim);
        if cur_marker != next_marker {
            edit.set_block_attributes(i, cur_attrs.with_marker(next_marker));
        }
        let Some(next_exp) = expected.checked_add(1) else {
            break;
        };
        expected = next_exp;
    }
}

fn previous_sibling_number(
    edit: &TextEdit,
    before_line: usize,
    level: u8,
    delim: char,
) -> Option<u64> {
    for j in (0..before_line).rev() {
        if is_soft_break(edit, j) {
            continue;
        }
        let kind = edit.block_kind(j);
        let Some(lvl) = list_level(kind) else {
            break;
        };
        if lvl < level {
            break;
        }
        if lvl > level {
            continue;
        }
        if matches!(kind, BlockKind::Numbered(_))
            && let Some((num, d)) = edit.block_attributes(j).marker().and_then(marker_number)
            && d == delim
        {
            return Some(num);
        }
        break;
    }
    None
}

fn strip_one_indent_step(indent: &str) -> String {
    indent
        .strip_prefix('\t')
        .or_else(|| indent.strip_prefix("  "))
        .or_else(|| indent.strip_prefix(' '))
        .unwrap_or(indent)
        .to_owned()
}

pub fn indent_at_cursor(edit: &mut TextEdit) -> bool {
    let (from, to) = edit.range();
    let first = edit.line_of_byte(from);
    let last = edit.line_of_byte(to);
    if first != last {
        if !(first..=last).any(|i| is_list(edit.block_kind(i))) {
            return false;
        }
        let mut changed = false;
        edit.undo_group(|e| {
            for i in first..=last {
                if is_list(e.block_kind(i)) && indent_single_line(e, i) {
                    changed = true;
                }
            }
        });
        return changed;
    }
    if !is_list(edit.block_kind(first)) {
        return false;
    }
    let mut changed = false;
    edit.undo_group(|e| {
        changed = indent_single_line(e, first);
    });
    changed
}

fn indent_single_line(edit: &mut TextEdit, line: usize) -> bool {
    let kind = edit.block_kind(line);
    let Some(lvl) = list_level(kind) else {
        return false;
    };
    let mut previous_child: Option<usize> = None;
    let mut parent_sibling: Option<usize> = None;
    for j in (0..line).rev() {
        if is_soft_break(edit, j) {
            continue;
        }
        let Some(above_lvl) = list_level(edit.block_kind(j)) else {
            break;
        };
        if above_lvl < lvl {
            break;
        }
        if above_lvl > lvl {
            if above_lvl == lvl + 1 && previous_child.is_none() {
                previous_child = Some(j);
            }
            continue;
        }
        parent_sibling = Some(j);
        break;
    }

    let Some(above) = parent_sibling else {
        // Top-level first item fallback: indent by two spaces without raising list level.
        if lvl == 0 {
            let cur_ind = effective_indent(edit, line);
            let attrs = edit
                .block_attributes(line)
                .clone()
                .with_indent(format!("  {cur_ind}"));
            edit.set_block_attributes(line, attrs);
            return true;
        }
        return false;
    };

    let new_lvl = lvl.saturating_add(1).min(MAX_LIST_LEVEL);
    let new_indent = if let Some(child) = previous_child {
        effective_indent(edit, child)
    } else {
        let above_ind = effective_indent(edit, above);
        let pad = content_column(edit, above).saturating_sub(indent_width(&above_ind, 0));
        format!("{above_ind}{}", " ".repeat(pad))
    };

    let old_attrs = edit.block_attributes(line).clone();
    let old_ind_len = effective_indent(edit, line).len();
    let mut new_attrs = old_attrs.clone().with_indent(new_indent.clone());
    let added_indent = new_indent
        .strip_prefix(&effective_indent(edit, line))
        .unwrap_or("  ")
        .to_owned();
    let old_delim = if matches!(kind, BlockKind::Numbered(_)) {
        let cur_marker = old_attrs.marker().filter(|s| !s.is_empty()).unwrap_or("1.");
        let delim = cur_marker
            .chars()
            .last()
            .filter(|c| matches!(c, '.' | ')'))
            .unwrap_or('.');
        let nested_marker = if let Some(child) = previous_child
            && matches!(edit.block_kind(child), BlockKind::Numbered(_))
            && let Some(child_marker) = edit.block_attributes(child).marker()
            && child_marker.ends_with(delim)
        {
            next_ordered_marker(child_marker)
        } else {
            format!("1{delim}")
        };
        new_attrs = new_attrs.with_marker(nested_marker);
        Some(delim)
    } else {
        None
    };

    edit.set_block_kind(line, with_list_level(kind, new_lvl));
    edit.set_block_attributes(line, new_attrs);

    // Keep any nested subtree under this line attached at +1 level.
    let _ = old_ind_len;
    for k in (line + 1)..edit.blocks().len() {
        let ck = edit.block_kind(k);
        let Some(cl) = list_level(ck) else {
            break;
        };
        if cl <= lvl {
            break;
        }
        let child_ind = effective_indent(edit, k);
        let child_attrs = edit
            .block_attributes(k)
            .clone()
            .with_indent(format!("{added_indent}{child_ind}"));
        edit.set_block_kind(
            k,
            with_list_level(ck, cl.saturating_add(1).min(MAX_LIST_LEVEL)),
        );
        edit.set_block_attributes(k, child_attrs);
    }

    if let Some(delim) = old_delim {
        let start_num = previous_sibling_number(edit, line, lvl, delim)
            .and_then(|n| n.checked_add(1))
            .unwrap_or(1);
        renumber_following(edit, line, lvl, delim, start_num);
    }
    true
}

pub fn outdent_at_cursor(edit: &mut TextEdit) -> bool {
    let (from, to) = edit.range();
    let first = edit.line_of_byte(from);
    let last = edit.line_of_byte(to);
    if first != last {
        if !(first..=last).any(|i| is_list(edit.block_kind(i))) {
            return false;
        }
        let mut changed = false;
        edit.undo_group(|e| {
            for i in first..=last {
                if is_list(e.block_kind(i)) && outdent_single_line(e, i) {
                    changed = true;
                }
            }
        });
        return changed;
    }
    if !is_list(edit.block_kind(first)) {
        return false;
    }
    let mut changed = false;
    edit.undo_group(|e| {
        changed = outdent_single_line(e, first);
    });
    changed
}

pub fn outdent_single_line(edit: &mut TextEdit, line: usize) -> bool {
    let kind = edit.block_kind(line);
    let Some(lvl) = list_level(kind) else {
        return false;
    };
    let cur_ind = effective_indent(edit, line);
    if lvl == 0 {
        if cur_ind.is_empty() {
            return false;
        }
        let attrs = edit
            .block_attributes(line)
            .clone()
            .with_indent(strip_one_indent_step(&cur_ind));
        edit.set_block_attributes(line, attrs);
        return true;
    }

    let mut parent: Option<usize> = None;
    for j in (0..line).rev() {
        if is_soft_break(edit, j) {
            continue;
        }
        let Some(above_lvl) = list_level(edit.block_kind(j)) else {
            break;
        };
        if above_lvl < lvl {
            parent = Some(j);
            break;
        }
    }

    let new_lvl = lvl - 1;
    let new_indent = parent
        .map(|p| effective_indent(edit, p))
        .unwrap_or_else(|| strip_one_indent_step(&cur_ind));
    let removed_indent = cur_ind.strip_prefix(&new_indent).unwrap_or("").to_owned();

    let old_attrs = edit.block_attributes(line).clone();
    let mut new_attrs = old_attrs.clone().with_indent(new_indent);
    let mut renumber_from: Option<(char, u64)> = None;

    if matches!(kind, BlockKind::Numbered(_))
        && let Some(p) = parent
        && matches!(edit.block_kind(p), BlockKind::Numbered(_))
    {
        let cur_marker = old_attrs.marker().filter(|s| !s.is_empty()).unwrap_or("1.");
        let cur_delim = cur_marker
            .chars()
            .last()
            .filter(|c| matches!(c, '.' | ')'))
            .unwrap_or('.');
        let p_marker = edit
            .block_attributes(p)
            .marker()
            .filter(|s| !s.is_empty())
            .unwrap_or("1.");
        if p_marker.ends_with(cur_delim) {
            let next_marker = next_ordered_marker(p_marker);
            if let Some((num, _)) = marker_number(&next_marker) {
                renumber_from = num.checked_add(1).map(|n| (cur_delim, n));
            }
            new_attrs = new_attrs.with_marker(next_marker);
        }
    }

    edit.set_block_kind(line, with_list_level(kind, new_lvl));
    edit.set_block_attributes(line, new_attrs);

    for k in (line + 1)..edit.blocks().len() {
        let ck = edit.block_kind(k);
        let Some(cl) = list_level(ck) else {
            break;
        };
        if cl <= lvl {
            break;
        }
        let child_ind = effective_indent(edit, k);
        let shifted = if !removed_indent.is_empty() {
            child_ind
                .strip_prefix(&removed_indent)
                .unwrap_or(&child_ind)
                .to_owned()
        } else {
            child_ind
        };
        let child_attrs = edit.block_attributes(k).clone().with_indent(shifted);
        edit.set_block_kind(k, with_list_level(ck, cl - 1));
        edit.set_block_attributes(k, child_attrs);
    }

    if let Some((delim, next_num)) = renumber_from {
        renumber_following(edit, line, new_lvl, delim, next_num);
    }
    true
}

pub fn enter_in_list(edit: &mut TextEdit) -> bool {
    let line_idx = edit.line_of_byte(edit.focus);
    let kind = edit.block_kind(line_idx);
    let Some(lvl) = list_level(kind) else {
        return false;
    };
    let line = edit.line_range(line_idx);

    if edit.anchor == edit.focus && line.is_empty() {
        if lvl == 0 {
            edit.undo_group(|e| {
                e.set_block_kind(line_idx, BlockKind::Paragraph);
                e.set_block_attributes(line_idx, BlockAttributes::default());
            });
            return true;
        }
        // Lift empty nested item one level, aligning with parent list kind and marker.
        let mut parent: Option<usize> = None;
        for j in (0..line_idx).rev() {
            if is_soft_break(edit, j) {
                continue;
            }
            let Some(above_lvl) = list_level(edit.block_kind(j)) else {
                break;
            };
            if above_lvl < lvl {
                parent = Some(j);
                break;
            }
        }
        let new_lvl = lvl - 1;
        edit.undo_group(|e| {
            if let Some(p) = parent {
                let p_kind = e.block_kind(p);
                let p_indent = effective_indent(e, p);
                match p_kind {
                    BlockKind::Bullet(_) => {
                        let marker = e
                            .block_attributes(p)
                            .marker()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("-")
                            .to_owned();
                        let attrs = BlockAttributes::default()
                            .with_indent(p_indent)
                            .with_marker(marker)
                            .with_space(" ");
                        e.set_block_kind(line_idx, BlockKind::Bullet(new_lvl));
                        e.set_block_attributes(line_idx, attrs);
                    }
                    BlockKind::Task { .. } => {
                        let marker_space = e
                            .block_attributes(p)
                            .marker_space()
                            .unwrap_or(" ")
                            .to_owned();
                        let attrs = BlockAttributes::default()
                            .with_indent(p_indent)
                            .with_marker_space(marker_space)
                            .with_content_space(" ");
                        e.set_block_kind(
                            line_idx,
                            BlockKind::Task {
                                level: new_lvl,
                                checked: false,
                            },
                        );
                        e.set_block_attributes(line_idx, attrs);
                    }
                    BlockKind::Numbered(_) => {
                        let p_marker = e
                            .block_attributes(p)
                            .marker()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("1.")
                            .to_owned();
                        let new_marker = next_ordered_marker(&p_marker);
                        let delim = new_marker
                            .chars()
                            .last()
                            .filter(|c| matches!(c, '.' | ')'))
                            .unwrap_or('.');
                        let next_num =
                            marker_number(&new_marker).and_then(|(n, _)| n.checked_add(1));
                        let attrs = BlockAttributes::default()
                            .with_indent(p_indent)
                            .with_marker(new_marker)
                            .with_space(" ");
                        e.set_block_kind(line_idx, BlockKind::Numbered(new_lvl));
                        e.set_block_attributes(line_idx, attrs);
                        if let Some(next_num) = next_num {
                            renumber_following(e, line_idx, new_lvl, delim, next_num);
                        }
                    }
                    _ => {
                        outdent_single_line(e, line_idx);
                    }
                }
            } else {
                outdent_single_line(e, line_idx);
            }
        });
        return true;
    }

    let old_attrs = edit.block_attributes(line_idx).clone();
    let old_indent = effective_indent(edit, line_idx);
    edit.undo_group(|e| {
        e.key(&KeyEvent::from(Key::Enter), None);
        let new_line = e.line_of_byte(e.focus);
        if new_line != line_idx {
            match e.block_kind(new_line) {
                BlockKind::Bullet(_) => {
                    let marker = old_attrs
                        .marker()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("-")
                        .to_owned();
                    let attrs = BlockAttributes::default()
                        .with_indent(old_indent)
                        .with_marker(marker)
                        .with_space(" ");
                    e.set_block_attributes(new_line, attrs);
                }
                BlockKind::Task { .. } => {
                    let marker_space = old_attrs.marker_space().unwrap_or(" ").to_owned();
                    let attrs = BlockAttributes::default()
                        .with_indent(old_indent)
                        .with_marker_space(marker_space)
                        .with_content_space(" ");
                    e.set_block_attributes(new_line, attrs);
                }
                BlockKind::Numbered(new_lvl) => {
                    let old_marker = old_attrs.marker().filter(|s| !s.is_empty()).unwrap_or("1.");
                    let new_marker = next_ordered_marker(old_marker);
                    let delim = new_marker
                        .chars()
                        .last()
                        .filter(|c| matches!(c, '.' | ')'))
                        .unwrap_or('.');
                    let next_num = marker_number(&new_marker).and_then(|(n, _)| n.checked_add(1));
                    let attrs = BlockAttributes::default()
                        .with_indent(old_indent)
                        .with_marker(new_marker)
                        .with_space(" ");
                    e.set_block_attributes(new_line, attrs);
                    if let Some(next_num) = next_num {
                        renumber_following(e, new_line, new_lvl, delim, next_num);
                    }
                }
                _ => {}
            }
        }
    });
    true
}
