//! Ported from KSG Code's composer-code-block.ts, using Rune's logical blocks
//! and CodeAttributes so adjacent identical fences stay distinct and keep metadata.
use rune::{BlockAttributes, BlockKind, Key, KeyEvent, LogicalBlock, TextEdit};

fn logical_code_block(edit: &TextEdit) -> Option<LogicalBlock> {
    let (from, to) = edit.range();
    let first = edit.line_of_byte(from);
    let last = edit.line_of_byte(to);
    edit.logical_blocks().find(|b| {
        b.kind == BlockKind::Code
            && edit.line_of_byte(b.range.start) <= first
            && last <= edit.line_of_byte(b.range.end)
    })
}

pub fn key(edit: &mut TextEdit, ev: &KeyEvent) -> bool {
    if ev.mods.cmd || ev.mods.ctrl {
        return false;
    }
    let Some(lb) = logical_code_block(edit) else {
        return false;
    };
    let block = lb.range.clone();
    let (from, to) = edit.range();
    if ev.key == Key::Tab {
        indent(edit, ev.mods.shift);
        return true;
    }
    if ev.key == Key::Backspace && !ev.mods.alt && from == to && from == block.start {
        let first_line = edit.line_of_byte(block.start);
        let last_line = edit.line_of_byte(block.end);
        edit.undo_group(|e| {
            for i in first_line..=last_line {
                let r = e.line_range(i);
                let _ = e.set_logical_block(r, BlockKind::Paragraph, BlockAttributes::default());
            }
            e.set_caret(block.start);
        });
        return true;
    }
    if ev.key != Key::Enter {
        return false;
    }
    let line = edit.line_range(edit.line_of_byte(from));
    let code_attrs = lb.attributes.code().cloned().unwrap_or_default();
    let fence = code_attrs.fence().to_owned();
    let line_text = &edit.text[line.clone()];
    if from == to && from == line.end && super::pasted::closes(line_text, &fence) {
        let closing_line = line_text.to_owned();
        let start = if line.start > block.start {
            line.start - 1
        } else {
            line.start
        };
        let removed_len = line.end - start;
        let updated_attrs = BlockAttributes::default()
            .with_code(code_attrs.with_close(Some(format!("\n{closing_line}"))));
        edit.undo_group(|e| {
            e.set_selection(start, line.end);
            e.insert("\n");
            let caret = e.focus;
            let last = e.line_of_byte(block.end - removed_len + 1);
            let first = e.line_of_byte(caret);
            e.update_logical_block(lb.id, BlockKind::Code, updated_attrs);
            for i in first..=last {
                let r = e.line_range(i);
                let _ = e.set_logical_block(r, BlockKind::Paragraph, BlockAttributes::default());
            }
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
                let mut updated = code_attrs;
                if updated.close().is_none() {
                    updated = updated.with_close(Some(format!("\n{fence}")));
                }
                let updated_attrs = BlockAttributes::default().with_code(updated);
                edit.undo_group(|e| {
                    e.update_logical_block(lb.id, BlockKind::Code, updated_attrs);
                    e.set_selection(start, from);
                    e.insert("\n");
                    let p_line = e.line_of_byte(e.focus);
                    let r = e.line_range(p_line);
                    let _ =
                        e.set_logical_block(r, BlockKind::Paragraph, BlockAttributes::default());
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
