//! Ported from KSG Code's composer-code-block.ts. Rune represents one fence as
//! contiguous Block::Code lines. Fence identity/attributes still need upstream support.
use rune::{Block, Key, KeyEvent, TextEdit};
use std::ops::Range;

fn block_range(edit: &TextEdit) -> Option<Range<usize>> {
    let (from, to) = edit.range();
    let first = edit.line_of_byte(from);
    let last = edit.line_of_byte(to);
    if edit.blocks()[first..=last]
        .iter()
        .any(|b| *b != Block::Code)
    {
        return None;
    }
    let start = (0..=first)
        .rev()
        .take_while(|i| edit.blocks()[*i] == Block::Code)
        .last()?;
    let end = (last..edit.blocks().len())
        .take_while(|i| edit.blocks()[*i] == Block::Code)
        .last()?;
    Some(edit.line_range(start).start..edit.line_range(end).end)
}

pub fn key(edit: &mut TextEdit, ev: &KeyEvent) -> bool {
    if ev.mods.cmd || ev.mods.ctrl {
        return false;
    }
    let Some(block) = block_range(edit) else {
        return false;
    };
    let (from, to) = edit.range();
    if ev.key == Key::Tab {
        indent(edit, ev.mods.shift);
        return true;
    }
    if ev.key == Key::Backspace && !ev.mods.alt && from == to && from == block.start {
        edit.undo_group(|e| {
            e.set_selection(block.start, block.end);
            e.set_block(Block::Paragraph);
            e.set_caret(block.start);
        });
        return true;
    }
    if ev.key != Key::Enter {
        return false;
    }
    let line = edit.line_range(edit.line_of_byte(from));
    if from == to && from == line.end && &edit.text[line.clone()] == "```" {
        // Only the canonical closing fence is lossless without a close attribute.
        let start = if line.start > block.start {
            line.start - 1
        } else {
            line.start
        };
        edit.undo_group(|e| {
            e.set_selection(start, line.end);
            e.insert("\n");
            let caret = e.focus;
            let last = e.line_of_byte(block.end - (line.end - start) + 1);
            let first = e.line_of_byte(caret);
            let mut blocks = e.blocks().to_vec();
            blocks[first..=last].fill(Block::Paragraph);
            e.set_blocks(blocks);
            e.set_caret(caret);
        });
        return true;
    }
    if from == to && from == block.end {
        let content = &edit.text[block.clone()];
        let trimmed = content.trim_end_matches([' ', '\t']);
        if let Some(rest) = trimmed.strip_suffix('\n') {
            let rest = rest.trim_end_matches([' ', '\t']);
            if let Some(rest) = rest.strip_suffix('\n') {
                let start = block.start + rest.len();
                edit.undo_group(|e| {
                    e.set_selection(start, from);
                    e.insert("\n");
                    e.set_block(Block::Paragraph);
                });
                return true;
            }
        }
    }
    let leading: String = edit.text[line.start..from]
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .collect();
    edit.insert(&format!("\n{leading}"));
    true
}

fn indent(edit: &mut TextEdit, out: bool) {
    let (from, to) = edit.range();
    if from == to && !out {
        edit.insert("  ");
        return;
    }
    let start = edit.line_range(edit.line_of_byte(from)).start;
    let end = edit
        .line_range(edit.line_of_byte(to.saturating_sub(usize::from(to > from))))
        .end;
    let lines: Vec<_> = edit.text[start..end].split('\n').collect();
    let next: Vec<_> = lines
        .iter()
        .map(|line| {
            if out {
                line.strip_prefix("  ")
                    .or_else(|| line.strip_prefix(['\t', ' ']))
                    .unwrap_or(line)
                    .to_string()
            } else if line.is_empty() {
                String::new()
            } else {
                format!("  {line}")
            }
        })
        .collect();
    let replacement = next.join("\n");
    if replacement == edit.text[start..end] {
        return;
    }
    let first_delta = next[0].len() as isize - lines[0].len() as isize;
    let total_delta = replacement.len() as isize - (end - start) as isize;
    let next_from = (from.saturating_add_signed(first_delta)).max(start);
    let next_to = (to.saturating_add_signed(total_delta)).max(next_from);
    edit.undo_group(|e| {
        e.set_selection(start, end);
        e.insert(&replacement);
        e.set_selection(next_from, next_to);
    });
}
