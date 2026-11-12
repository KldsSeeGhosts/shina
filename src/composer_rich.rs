//! Rune document editing and Markdown transport, ported from KSG Code's
//! ComposerPromptEditorTiptap.tsx and composer-rich-text-doc.ts.
use rune::{Block, Key, KeyEvent, Mark, TextEdit};

pub fn draft(text: impl Into<String>) -> TextEdit {
    TextEdit::new(text).rich().allow_marks([
        Mark::Bold,
        Mark::Italic,
        Mark::Strike,
        Mark::Code,
        Mark::Link(String::new()),
    ])
}

#[derive(Default)]
pub struct Rules {
    converted: Option<TextEdit>,
    suppress: bool,
}

fn same_document(a: &TextEdit, b: &TextEdit) -> bool {
    a.text == b.text && a.marks() == b.marks() && a.blocks() == b.blocks()
}

impl Rules {
    /// Only app-owned chords and input-rule reversal need interception. Rune owns list Enter,
    /// indentation, numbering, text movement and ordinary undo.
    pub fn key(&mut self, edit: &mut TextEdit, ev: &KeyEvent) -> bool {
        if edit.is_composing() {
            return false;
        }
        self.suppress = ev.is_command('z') || ev.is_command('y');
        if ev.key == Key::Backspace
            && !ev.mods.cmd
            && !ev.mods.ctrl
            && !ev.mods.alt
            && self.converted.take().is_some_and(|after| {
                same_document(&after, edit)
                    && edit.anchor == after.anchor
                    && edit.focus == after.focus
            })
        {
            edit.undo();
            return true;
        }
        if !matches!(ev.key, Key::Modifier { .. }) {
            self.converted = None;
        }
        for (ch, mark) in [('b', Mark::Bold), ('i', Mark::Italic), ('e', Mark::Code)] {
            if ev.is_command(ch) && !ev.mods.shift && !ev.mods.alt {
                edit.toggle_mark(mark);
                return true;
            }
        }
        if ev.key == Key::Enter && !ev.mods.cmd && !ev.mods.ctrl && !ev.mods.alt {
            let line = edit.line_range(edit.line_of_byte(edit.focus));
            if edit.anchor == edit.focus
                && edit.focus == line.end
                && edit.active_block() == Block::Paragraph
                && &edit.text[line.clone()] == "```"
            {
                edit.undo_group(|e| {
                    e.set_selection(line.start, line.end);
                    e.insert("");
                    e.set_block(Block::Code);
                });
                self.converted = Some(edit.clone());
                return true;
            }
        }
        false
    }

    /// Called after a text change, including IME commits. Undo must not reapply an input rule.
    pub fn changed(&mut self, edit: &mut TextEdit) {
        if std::mem::take(&mut self.suppress)
            || edit.is_composing()
            || edit.anchor != edit.focus
            || edit.active_block() != Block::Paragraph
        {
            return;
        }
        let line = edit.line_range(edit.line_of_byte(edit.focus));
        let prefix = &edit.text[line.start..edit.focus];
        let block = match prefix {
            "- " | "* " => Block::Bullet(0),
            "1. " => Block::Numbered(0),
            "> " => Block::Quote,
            _ => return,
        };
        let end = edit.focus;
        edit.undo_group(|e| {
            e.set_selection(line.start, end);
            e.insert("");
            e.set_block(block);
        });
        self.converted = Some(edit.clone());
    }
}

fn fence(text: &str, minimum: usize) -> String {
    let longest = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(minimum.max(longest + 1))
}

fn mark_delimiters(mark: &Mark, content: &str) -> (String, String) {
    let delimiter = match mark {
        Mark::Bold => "**",
        Mark::Italic => "*",
        Mark::Strike => "~~",
        Mark::Code => {
            let ticks = fence(content, 1);
            let padding = content.starts_with('`')
                || content.ends_with('`')
                || !content.trim().is_empty()
                    && (content.starts_with(' ') || content.ends_with(' '));
            return if padding {
                (format!("{ticks} "), format!(" {ticks}"))
            } else {
                (ticks.clone(), ticks)
            };
        }
        Mark::Link(url) => return ("[".into(), format!("]({})", url.replace(')', "\\)"))),
        _ => "",
    };
    (delimiter.into(), delimiter.into())
}

fn inline(
    edit: &TextEdit,
    range: std::ops::Range<usize>,
    target_machine: Option<&str>,
    machine_name: &dyn Fn(&str) -> String,
) -> String {
    let pills: Vec<_> = edit
        .pills()
        .filter(|(r, _)| r.start >= range.start && r.end <= range.end)
        .filter_map(|(r, id)| crate::composer_tokens::Token::decode(id).map(|token| (r, token)))
        .collect();
    if !pills.is_empty() {
        let mut out = String::new();
        let mut cursor = range.start;
        for (pill, token) in pills {
            out.push_str(&inline(
                edit,
                cursor..pill.start,
                target_machine,
                machine_name,
            ));
            out.push_str(&token.source_for(target_machine, machine_name));
            cursor = pill.end;
        }
        out.push_str(&inline(
            edit,
            cursor..range.end,
            target_machine,
            machine_name,
        ));
        return out;
    }
    let mut bounds = vec![range.start, range.end];
    let marks: Vec<_> = edit
        .marks()
        .iter()
        .filter_map(|m| {
            let mut start = m.start.max(range.start);
            let mut end = m.end.min(range.end);
            if start >= end {
                return None;
            }
            if matches!(m.mark, Mark::Bold | Mark::Italic | Mark::Strike) {
                let text = &edit.text[start..end];
                start += text.len() - text.trim_start().len();
                end = end.saturating_sub(text.len() - text.trim_end().len());
            }
            (start < end).then(|| rune::MarkRange::new(start..end, m.mark.clone()))
        })
        .collect();
    for m in &marks {
        bounds.extend([m.start.max(range.start), m.end.min(range.end)]);
    }
    bounds.sort_unstable();
    bounds.dedup();
    let mut out = String::new();
    let mut active: Vec<usize> = Vec::new();
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let mut next: Vec<_> = (0..marks.len())
            .filter(|&i| marks[i].start <= start && marks[i].end >= end)
            .collect();
        // Longer marks contain shorter ones. Keep retained prefixes to avoid needless delimiters.
        next.sort_by_key(|&i| (std::cmp::Reverse(marks[i].end), i));
        let common = active.iter().zip(&next).take_while(|(a, b)| a == b).count();
        for &i in active[common..].iter().rev() {
            let m = &marks[i];
            out.push_str(&mark_delimiters(&m.mark, &edit.text[m.start..m.end]).1);
        }
        for &i in &next[common..] {
            let m = &marks[i];
            out.push_str(&mark_delimiters(&m.mark, &edit.text[m.start..m.end]).0);
        }
        out.push_str(&edit.text[start..end]);
        active = next;
    }
    for i in active.into_iter().rev() {
        let m = &marks[i];
        out.push_str(&mark_delimiters(&m.mark, &edit.text[m.start..m.end]).1);
    }
    out
}

pub fn markdown(edit: &TextEdit) -> String {
    markdown_for(edit, None, &|machine| machine.to_owned())
}

pub fn markdown_for(
    edit: &TextEdit,
    target_machine: Option<&str>,
    machine_name: &dyn Fn(&str) -> String,
) -> String {
    let paragraphs = edit.paragraphs();
    let mut out = Vec::new();
    let mut numbers = [0usize; 4];
    let mut at = 0;
    while at < paragraphs.len() {
        let p = &paragraphs[at];
        if at > 0 && p.block != Block::Quote && paragraphs[at - 1].block == Block::Quote {
            // Without a blank line Markdown treats a following paragraph as part of the quote.
            out.push(String::new());
        }
        if p.block == Block::Code {
            let start = p.range.start;
            let mut end = p.range.end;
            at += 1;
            while at < paragraphs.len() && paragraphs[at].block == Block::Code {
                end = paragraphs[at].range.end;
                at += 1;
            }
            let content = &edit.text[start..end];
            let ticks = fence(content, 3);
            out.push(format!("{ticks}\n{content}\n{ticks}"));
            numbers.fill(0);
            continue;
        }
        let prefix = match p.block {
            Block::Bullet(level) => {
                numbers[level as usize..].fill(0);
                format!("{}- ", "    ".repeat(level as usize))
            }
            Block::Numbered(level) => {
                if !p.soft_break {
                    numbers[level as usize] += 1;
                    numbers[level as usize + 1..].fill(0);
                }
                if p.soft_break {
                    "    ".repeat(level as usize + 1)
                } else {
                    format!(
                        "{}{}. ",
                        "    ".repeat(level as usize),
                        numbers[level as usize]
                    )
                }
            }
            Block::Quote => {
                numbers.fill(0);
                "> ".into()
            }
            _ => {
                numbers.fill(0);
                String::new()
            }
        };
        let prefix = if p.soft_break && matches!(p.block, Block::Bullet(_)) {
            "    ".repeat(p.block.list_level().unwrap_or(0) as usize + 1)
        } else {
            prefix
        };
        out.push(format!(
            "{prefix}{}",
            inline(edit, p.range.clone(), target_machine, machine_name)
        ));
        at += 1;
    }
    out.join("\n")
}

pub fn has_format(edit: &TextEdit) -> bool {
    !edit.marks().is_empty()
        || edit.blocks().iter().any(|b| *b != Block::Paragraph)
        || !edit.soft_breaks().is_empty()
}

pub fn restore(text: &str, html: Option<&str>) -> TextEdit {
    match html {
        Some(html) => {
            let edit = TextEdit::from_html(html);
            if edit.text == text {
                edit.allow_marks([
                    Mark::Bold,
                    Mark::Italic,
                    Mark::Strike,
                    Mark::Code,
                    Mark::Link(String::new()),
                ])
            } else {
                draft(text)
            }
        }
        None => draft(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(edit: &mut TextEdit, rules: &mut Rules, key: Key) {
        let event = KeyEvent {
            key,
            mods: Default::default(),
        };
        if !rules.key(edit, &event) {
            edit.key(&event, None);
            rules.changed(edit);
        }
    }

    #[test]
    fn list_rules_continue_indent_outdent_end_and_reverse_once() {
        for (prefix, block) in [
            ("- ", Block::Bullet(0)),
            ("* ", Block::Bullet(0)),
            ("1. ", Block::Numbered(0)),
            ("> ", Block::Quote),
        ] {
            let mut edit = draft("");
            let mut rules = Rules::default();
            key(&mut edit, &mut rules, Key::Text(prefix.into()));
            assert_eq!(edit.active_block(), block);
            key(&mut edit, &mut rules, Key::Backspace);
            assert_eq!(edit.text, prefix);
            assert_eq!(edit.active_block(), Block::Paragraph);
            edit.replace_text("");
            key(&mut edit, &mut rules, Key::Text(prefix.into()));
            key(&mut edit, &mut rules, Key::Text("first".into()));
            key(&mut edit, &mut rules, Key::Enter);
            assert_eq!(edit.blocks(), &[block, block]);
            key(&mut edit, &mut rules, Key::Enter);
            assert_eq!(edit.active_block(), Block::Paragraph);
        }
        let mut edit = draft("one\ntwo\nthree");
        edit.set_blocks(vec![Block::Numbered(0); 3]);
        edit.set_caret(5);
        edit.indent_list();
        assert_eq!(markdown(&edit), "1. one\n    1. two\n2. three");
        edit.outdent_list();
        assert_eq!(markdown(&edit), "1. one\n2. two\n3. three");
    }

    #[test]
    fn serialization_reproduces_blocks_nested_marks_and_safe_code_fences() {
        let mut edit = draft("bold italic\nitem\nquoted\nx`y\n```\nend");
        edit.set_selection(0, 11);
        edit.toggle_mark(Mark::Bold);
        edit.set_selection(5, 11);
        edit.toggle_mark(Mark::Italic);
        edit.set_blocks(vec![
            Block::Paragraph,
            Block::Bullet(0),
            Block::Quote,
            Block::Code,
            Block::Code,
            Block::Paragraph,
        ]);
        assert_eq!(
            markdown(&edit),
            "**bold *italic***\n- item\n> quoted\n\n````\nx`y\n```\n````\nend"
        );
        let restored = restore(&edit.text, Some(&edit.to_html()));
        assert_eq!(markdown(&restored), markdown(&edit));
        assert_eq!(restored.marks(), edit.marks());
        assert_eq!(restored.blocks(), edit.blocks());
        assert!(restore("old plain draft", None).is_rich());
        for block in [
            Block::Bullet(0),
            Block::Numbered(0),
            Block::Quote,
            Block::Code,
        ] {
            let mut empty = draft("");
            empty.set_block(block);
            let loaded = restore("", Some(&empty.to_html()));
            assert_eq!(loaded.blocks(), empty.blocks(), "{}", empty.to_html());
        }
    }

    #[test]
    fn code_input_rule_and_unsupported_markers() {
        let mut edit = draft("```");
        let mut rules = Rules::default();
        key(&mut edit, &mut rules, Key::Enter);
        assert_eq!(edit.active_block(), Block::Code);
        key(&mut edit, &mut rules, Key::Text("code".into()));
        key(&mut edit, &mut rules, Key::Enter);
        key(&mut edit, &mut rules, Key::Enter);
        assert_eq!(edit.active_block(), Block::Paragraph);
        for text in ["# heading", "---", "- [ ] task"] {
            let mut edit = draft(text);
            rules.changed(&mut edit);
            assert_eq!(edit.active_block(), Block::Paragraph);
            assert_eq!(markdown(&edit), text);
        }
    }

    #[test]
    fn formatting_does_not_put_whitespace_inside_emphasis_delimiters() {
        let mut edit = draft(" bold ");
        edit.set_selection(0, edit.text.len());
        edit.toggle_mark(Mark::Bold);
        assert_eq!(markdown(&edit), " **bold** ");
        let mut code = draft(" ");
        code.set_selection(0, 1);
        code.toggle_mark(Mark::Code);
        assert_eq!(markdown(&code), "` `");
    }
}
