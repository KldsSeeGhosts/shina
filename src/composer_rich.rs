//! Rune document editing and Markdown transport, ported from KSG Code's
//! ComposerPromptEditorTiptap.tsx and composer-rich-text-doc.ts.
use rune::{Block, Key, KeyEvent, Mark, TextEdit};

#[path = "composer_code.rs"]
mod code;
#[path = "composer_markdown_paste.rs"]
mod pasted;
#[path = "composer_inline.rs"]
mod source;

#[cfg(test)]
#[path = "composer_edit_tests.rs"]
mod parity_tests;

pub fn draft(text: impl Into<String>) -> TextEdit {
    TextEdit::new(text).rich().allow_marks([
        Mark::Bold,
        Mark::Italic,
        Mark::Strike,
        Mark::Code,
        Mark::Link(String::new()),
    ])
}

pub use pasted::mention_path as pasted_mention_path;
#[cfg(test)]
pub use pasted::paste;
pub use pasted::paste_with_path_query;

/// The editor document, not its expanded Markdown transport. Ported from KSG Code's
/// snapshotComposerSend/restoreComposerDraft. Attachments exclude files derived from pills.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub text: String,
    pub html: String,
    pub attachments: Vec<crate::attachments::Attachment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<crate::composer_context::ComposerContextRecord>,
}

impl Snapshot {
    pub fn capture(edit: &TextEdit, attachments: &[crate::attachments::Attachment]) -> Self {
        Self::capture_with_context(edit, attachments, &[])
    }

    pub fn capture_with_context(
        edit: &TextEdit,
        attachments: &[crate::attachments::Attachment],
        context: &[crate::composer_context::ComposerContextRecord],
    ) -> Self {
        Self {
            text: edit.text.clone(),
            html: edit.to_html(),
            attachments: attachments.to_vec(),
            context: context.to_vec(),
        }
    }

    pub fn editor(&self) -> TextEdit {
        restore(&self.text, Some(&self.html))
    }
}

/// Preserve both documents, including marks, blocks, soft breaks and self-contained pills.
/// Shina's existing failure recovery prepends the submission to edits made after Send.
pub fn prepend(submitted: &TextEdit, later: &TextEdit) -> TextEdit {
    let focused = later.focused;
    let mut edit = if submitted.text.is_empty() {
        later.clone()
    } else if later.text.is_empty() {
        submitted.clone()
    } else {
        let text = format!("{}\n\n{}", submitted.text, later.text);
        restore(
            &text,
            Some(&format!(
                "{}<p></p>{}",
                submitted.to_html(),
                later.to_html()
            )),
        )
    };
    edit.focused = focused;
    edit.set_caret(edit.text.len());
    edit
}

#[derive(Default)]
pub struct Rules {
    converted: Option<TextEdit>,
    suppress: bool,
}

fn same_document(a: &TextEdit, b: &TextEdit) -> bool {
    a.text == b.text
        && a.marks() == b.marks()
        && a.blocks() == b.blocks()
        && a.soft_breaks() == b.soft_breaks()
}

impl Rules {
    pub fn pasted(&mut self) {
        self.converted = None;
        self.suppress = true;
    }

    /// Ported from ComposerPromptEditorTiptap and the Composer* mark extensions.
    pub fn key(&mut self, edit: &mut TextEdit, ev: &KeyEvent) -> bool {
        if edit.is_composing() {
            self.suppress = false;
            return false;
        }
        self.suppress = matches!(ev.key, Key::Backspace | Key::Delete)
            || ev.is_command('z')
            || ev.is_command('y')
            || ev.is_command('x');
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
        for (ch, shift, mark) in [
            ('b', None, Mark::Bold),
            ('i', None, Mark::Italic),
            ('e', Some(false), Mark::Code),
            ('s', Some(true), Mark::Strike),
        ] {
            if ev.is_command(ch)
                && shift.is_none_or(|shift| shift == ev.mods.shift)
                && !ev.mods.alt
                && !(cfg!(target_os = "macos") && ev.mods.ctrl)
            {
                edit.toggle_mark(mark);
                return true;
            }
        }
        if let Key::Text(text) = &ev.key
            && !ev.mods.cmd
            && !ev.mods.ctrl
            && surround(edit, text)
        {
            return true;
        }
        if ev.key == Key::Text("`".into()) && open_code_before_text(edit) {
            return true;
        }
        if code::key(edit, ev) {
            return true;
        }
        if ev.key == Key::Enter && !ev.mods.cmd && !ev.mods.ctrl {
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
            // KSG intercepts Shift-Enter too and splits a block with its marks,
            // rather than invoking Tiptap's hard-break keymap.
            if edit.active_block() == Block::Paragraph {
                edit.insert("\n");
            } else {
                edit.key(&KeyEvent::from(Key::Enter), None);
            }
            return true;
        }
        false
    }

    /// Called after a text change, including IME commits. Undo must not reapply an input rule.
    pub fn changed(&mut self, edit: &mut TextEdit) {
        if std::mem::take(&mut self.suppress)
            || edit.is_composing()
            || edit.anchor != edit.focus
            || edit.active_block() == Block::Code
        {
            return;
        }
        if input_mark(edit) {
            self.converted = Some(edit.clone());
            return;
        }
        if edit.active_block() != Block::Paragraph {
            return;
        }
        let line = edit.line_range(edit.line_of_byte(edit.focus));
        let prefix = &edit.text[line.start..edit.focus];
        let (block, marker_end) = match prefix {
            "* " => (Block::Bullet(0), edit.focus),
            "1. " => (Block::Numbered(0), edit.focus),
            "> " => (Block::Quote, edit.focus),
            _ if prefix.starts_with("- ")
                && prefix.len() == 2 + prefix[2..].chars().next().map_or(0, char::len_utf8)
                && prefix[2..]
                    .chars()
                    .next()
                    .is_some_and(|c| !source::space(c) && c != '[') =>
            {
                (Block::Bullet(0), line.start + 2)
            }
            _ => return,
        };
        let end = edit.focus;
        edit.undo_group(|e| {
            e.set_selection(line.start, marker_end);
            e.insert("");
            e.set_block(block);
            e.set_caret(end - (marker_end - line.start));
        });
        self.converted = Some(edit.clone());
    }
}

fn open_code_before_text(edit: &mut TextEdit) -> bool {
    if edit.anchor != edit.focus || edit.active_block() == Block::Code {
        return false;
    }
    let from = edit.focus;
    let line = edit.line_range(edit.line_of_byte(from));
    if edit.text[..from].ends_with('`') {
        return false;
    }
    let after = &edit.text[from..line.end];
    let Some(length) = after.find('`') else {
        return false;
    };
    if length == 0
        || after.as_bytes().get(length + 1) == Some(&b'`')
        || edit
            .marks()
            .iter()
            .any(|m| m.start < from + length + 1 && from < m.end)
    {
        return false;
    }
    edit.undo_group(|e| {
        e.set_selection(from + length, from + length + 1);
        e.insert("");
        e.set_selection(from, from + length);
        e.set_mark(Mark::Code);
        e.set_caret(from);
        e.set_mark(Mark::Code);
    });
    true
}

pub fn editor_key_before_commands(thread: &mut crate::model::Thread, event: &KeyEvent) -> bool {
    if thread.draft.is_composing()
        || event.mods.cmd
        || event.mods.ctrl
        || !matches!(event.key, Key::Enter | Key::Tab)
    {
        return false;
    }
    let edit = &thread.draft;
    let line = edit.line_range(edit.line_of_byte(edit.focus));
    let code_key = edit.active_block() == Block::Code
        || event.key == Key::Enter
            && edit.anchor == edit.focus
            && edit.focus == line.end
            && edit.active_block() == Block::Paragraph
            && &edit.text[line] == "```";
    if code_key && thread.composer_rules.key(&mut thread.draft, event) {
        thread.typing_guard.on_type(std::time::Instant::now());
        thread.update_submission_error();
        true
    } else {
        false
    }
}

/// KSG's handleTextInput leaves styled text and chips to normal replacement. Plain
/// selections keep their selection, with either a round-trippable mark or literal wrappers.
fn surround(edit: &mut TextEdit, opener: &str) -> bool {
    let closer = match opener {
        "(" => ")",
        "[" => "]",
        "{" => "}",
        "'" => "'",
        "\"" => "\"",
        "“" => "”",
        "`" => "`",
        "<" => ">",
        "«" => "»",
        "*" => "*",
        "_" => "_",
        _ => return false,
    };
    let (from, to) = edit.range();
    if from == to || edit.marks().iter().any(|m| m.start < to && m.end > from) {
        return false;
    }
    let mark = match opener {
        "`" => Some(Mark::Code),
        "*" | "_" => Some(Mark::Italic),
        _ => None,
    };
    if let Some(mark) = mark
        && edit.active_block() != Block::Code
        && !edit.text[from..to].contains('\n')
    {
        let mut candidate = edit.clone();
        candidate.set_mark(mark.clone());
        let line = edit.line_range(edit.line_of_byte(from));
        let serialized = inline(&candidate, line.clone(), None, &str::to_owned);
        let restored = source::document(&serialized);
        let expected_marks: Vec<_> = candidate
            .marks()
            .iter()
            .filter_map(|m| {
                let start = m.start.max(line.start);
                let end = m.end.min(line.end);
                (start < end).then(|| {
                    rune::MarkRange::new(start - line.start..end - line.start, m.mark.clone())
                })
            })
            .collect();
        if restored.text == edit.text[line] && restored.marks() == expected_marks {
            edit.set_mark(mark);
            return true;
        }
    }
    edit.undo_group(|e| {
        e.set_caret(to);
        e.insert(closer);
        e.set_caret(from);
        e.insert(opener);
        e.set_selection(from + opener.len(), to + opener.len());
    });
    true
}

/// The Tiptap 3.31.3 input regexes used by KSG. Unlike its paste parser these
/// require whitespace before emphasis, reject nested delimiter characters, and
/// accept leading/trailing spaces in nonempty content. `_italic_` is disabled.
fn input_mark(edit: &mut TextEdit) -> bool {
    let end = edit.focus;
    let line = edit.line_range(edit.line_of_byte(end));
    let prefix = &edit.text[line.start..end];
    for (delimiter, mark) in [
        ("`", Mark::Code),
        ("~~", Mark::Strike),
        ("**", Mark::Bold),
        ("__", Mark::Bold),
        ("*", Mark::Italic),
    ] {
        if !prefix.ends_with(delimiter) || prefix.len() <= 2 * delimiter.len() {
            continue;
        }
        let content_end = prefix.len() - delimiter.len();
        let Some(start) = prefix[..content_end].rfind(delimiter) else {
            continue;
        };
        let content_start = start + delimiter.len();
        let content = &prefix[content_start..content_end];
        if content.is_empty() || content.contains(delimiter.chars().next().unwrap()) {
            continue;
        }
        let before = prefix[..start].chars().next_back();
        if mark == Mark::Code {
            if before == Some('`') {
                continue;
            }
            let mut ticks = 0;
            for (at, ch) in prefix[..start].char_indices() {
                let pos = line.start + at;
                if edit.pills().any(|(r, _)| r.contains(&pos)) {
                    ticks = 0;
                } else if ch == '`'
                    && prefix.as_bytes().get(at.wrapping_sub(1)) != Some(&b'`')
                    && prefix.as_bytes().get(at + 1) != Some(&b'`')
                    && !edit
                        .marks()
                        .iter()
                        .any(|m| m.mark == Mark::Code && m.start <= pos && pos < m.end)
                {
                    ticks += 1;
                }
            }
            if ticks % 2 == 1 {
                continue;
            }
        } else if before.is_some_and(|c| !source::space(c)) || content.chars().all(source::space) {
            continue;
        }
        let start = line.start + start;
        let content_start = line.start + content_start;
        let content_end = line.start + content_end;
        // Only the delimiters must be outside existing code. Code inside emphasis is allowed.
        if mark != Mark::Code
            && edit.marks().iter().any(|m| {
                m.mark == Mark::Code
                    && (m.start < content_start && start < m.end
                        || m.start < end && content_end < m.end)
            })
        {
            continue;
        }
        edit.undo_group(|e| {
            e.set_selection(content_end, end);
            e.insert("");
            e.set_selection(start, content_start);
            e.insert("");
            let marked_end = content_end - delimiter.len();
            e.set_selection(start, marked_end);
            e.set_mark(mark.clone());
            e.set_caret(marked_end);
            e.remove_mark(&mark);
        });
        return true;
    }
    false
}

fn fence(text: &str, minimum: usize) -> String {
    let longest = text
        .split('\n')
        .filter(|line| pasted::closes(line, "```"))
        .map(|line| line.bytes().take_while(|b| *b == b'`').count())
        .max()
        .unwrap_or(0);
    "`".repeat(minimum.max(longest + 1))
}

/// Ported from composer-rich-text-doc.ts appendInlineRuns, including per-mark
/// whitespace trimming, stable nesting and atom marks. KSG uses single backticks.
fn inline(
    edit: &TextEdit,
    range: std::ops::Range<usize>,
    target_machine: Option<&str>,
    machine_name: &dyn Fn(&str) -> String,
) -> String {
    // KSG disables links in new composer documents. Preserve URLs in legacy
    // Shina snapshots rather than silently losing them on their next send.
    let links: Vec<_> = edit
        .marks()
        .iter()
        .filter_map(|m| {
            let Mark::Link(url) = &m.mark else {
                return None;
            };
            let start = m.start.max(range.start);
            let end = m.end.min(range.end);
            (start < end).then_some((start..end, url.clone()))
        })
        .collect();
    if !links.is_empty() {
        let mut without = edit.clone();
        without.set_marks(
            edit.marks()
                .iter()
                .filter(|m| !matches!(m.mark, Mark::Link(_)))
                .cloned()
                .collect(),
        );
        let mut out = String::new();
        let mut cursor = range.start;
        for (link, url) in links {
            out.push_str(&inline(
                &without,
                cursor..link.start,
                target_machine,
                machine_name,
            ));
            out.push('[');
            out.push_str(&inline(
                &without,
                link.clone(),
                target_machine,
                machine_name,
            ));
            out.push_str(&format!("]({})", url.replace(')', "\\)")));
            cursor = link.end;
        }
        out.push_str(&inline(
            &without,
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
        .filter(|m| m.start < range.end && range.start < m.end)
        .collect();
    for m in &marks {
        bounds.extend([m.start.max(range.start), m.end.min(range.end)]);
    }
    bounds.sort_unstable();
    bounds.dedup();
    struct Run {
        text: String,
        marks: Vec<Mark>,
        whitespace: bool,
    }
    let mut runs: Vec<Run> = Vec::new();
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let active: Vec<_> = source::ORDER
            .into_iter()
            .filter(|mark| {
                marks
                    .iter()
                    .any(|m| m.start <= start && end <= m.end && m.mark == *mark)
            })
            .collect();
        let context_ref = marks.iter().find_map(|m| {
            (m.start <= start && end <= m.end)
                .then(|| m.mark.pill_id())
                .flatten()
                .and_then(crate::composer_context::parse_composer_context_href)
        });
        if let Some((kind, context_id)) = context_ref {
            runs.push(Run {
                text: crate::composer_context::format_composer_context_reference(
                    &kind,
                    &context_id,
                    &edit.text[start..end],
                ),
                marks: active,
                whitespace: false,
            });
            continue;
        }
        let token = marks.iter().find_map(|m| {
            (m.start <= start && end <= m.end)
                .then(|| m.mark.pill_id())
                .flatten()
                .and_then(crate::composer_tokens::Token::decode)
        });
        if let Some(token) = token {
            runs.push(Run {
                text: token.source_for(target_machine, machine_name),
                marks: active,
                whitespace: false,
            });
            continue;
        }
        let text = &edit.text[start..end];
        if active.contains(&Mark::Code) {
            runs.push(Run {
                text: text.into(),
                marks: active,
                whitespace: false,
            });
            continue;
        }
        let trimmed_start = text.len() - text.trim_start_matches(source::space).len();
        let trimmed_end = trimmed_start.max(text.trim_end_matches(source::space).len());
        let mut at = 0;
        for bound in [trimmed_start, trimmed_end, text.len()] {
            if bound > at {
                let text = &text[at..bound];
                runs.push(Run {
                    text: text.into(),
                    marks: active.clone(),
                    whitespace: text.chars().all(source::space),
                });
            }
            at = bound;
        }
    }
    for mark in source::ORDER {
        if mark == Mark::Code {
            continue;
        }
        for reverse in [false, true] {
            let mut content = false;
            for offset in 0..runs.len() {
                let index = if reverse {
                    runs.len() - 1 - offset
                } else {
                    offset
                };
                let run = &mut runs[index];
                if !run.marks.contains(&mark) {
                    content = false;
                } else if run.whitespace {
                    if !content {
                        run.marks.retain(|m| *m != mark);
                    }
                } else {
                    content = true;
                }
            }
        }
    }
    let mut ends = [None; 4];
    for index in (0..runs.len()).rev() {
        for (kind, mark) in source::ORDER.iter().enumerate() {
            if runs[index].marks.contains(mark) {
                ends[kind].get_or_insert(index);
            } else {
                ends[kind] = None;
            }
        }
        runs[index].marks.sort_by_key(|mark| {
            let kind = source::ORDER.iter().position(|m| m == mark).unwrap();
            (std::cmp::Reverse(ends[kind]), kind)
        });
    }
    for index in 1..runs.len() {
        let retained: Vec<_> = runs[index - 1]
            .marks
            .iter()
            .take_while(|mark| runs[index].marks.contains(mark))
            .cloned()
            .collect();
        let remaining: Vec<_> = runs[index]
            .marks
            .iter()
            .filter(|m| !retained.contains(m))
            .cloned()
            .collect();
        runs[index].marks = retained.into_iter().chain(remaining).collect();
    }
    let mut out = String::new();
    let mut active: Vec<Mark> = Vec::new();
    for run in runs {
        let common = active
            .iter()
            .zip(&run.marks)
            .take_while(|(a, b)| a == b)
            .count();
        for mark in active[common..].iter().rev() {
            out.push_str(source::delimiter(mark));
        }
        for mark in &run.marks[common..] {
            out.push_str(source::delimiter(mark));
        }
        out.push_str(&run.text);
        active = run.marks;
    }
    for mark in active.iter().rev() {
        out.push_str(source::delimiter(mark));
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
        if at > 0
            && p.block == Block::Paragraph
            && !p.range.is_empty()
            && paragraphs[at - 1].block == Block::Quote
        {
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
            let newline = if content.is_empty() { "" } else { "\n" };
            out.push(format!("{ticks}{newline}{content}\n{ticks}"));
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

pub fn markdown_range(edit: &TextEdit, range: std::ops::Range<usize>) -> String {
    let start = range.start.min(edit.text.len());
    let end = range.end.min(edit.text.len()).max(start);
    if start == end {
        return String::new();
    }
    let paragraphs = edit.paragraphs();
    let mut out = Vec::new();
    let mut at = 0;
    let mut numbers = [0usize; 4];
    while at < paragraphs.len() {
        let p = &paragraphs[at];
        if p.range.end < start || p.range.start > end {
            at += 1;
            continue;
        }
        // Only treat a single-line partial selection inside a paragraph as plain inline markdown.
        if p.range.start <= start && end <= p.range.end && !p.soft_break {
            if p.block == Block::Code {
                return edit.text[start..end].to_owned();
            }
            return inline(edit, start..end, None, &|machine| machine.to_owned());
        }
        let sub_start = p.range.start.max(start);
        let sub_end = p.range.end.min(end);
        if sub_start > sub_end {
            at += 1;
            continue;
        }
        if p.block == Block::Code {
            let mut code_end = sub_end;
            at += 1;
            while at < paragraphs.len()
                && paragraphs[at].block == Block::Code
                && paragraphs[at].range.start <= end
            {
                code_end = paragraphs[at].range.end.min(end);
                at += 1;
            }
            let content = &edit.text[sub_start..code_end];
            let ticks = fence(content, 3);
            let newline = if content.is_empty() { "" } else { "\n" };
            out.push(format!("{ticks}{newline}{content}\n{ticks}"));
            numbers.fill(0);
            continue;
        }
        let prefix = if sub_start == p.range.start {
            match p.block {
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
            }
        } else {
            String::new()
        };
        out.push(format!(
            "{prefix}{}",
            inline(edit, sub_start..sub_end, None, &|machine| machine
                .to_owned())
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

/// Rebuilds a rich `TextEdit` from canonical Markdown containing `t3-context://v1/...`
/// references when no editor snapshot was sent (e.g., headless RPC clients).
pub fn draft_from_canonical(text: &str) -> TextEdit {
    let refs = crate::composer_context::collect_composer_context_references(text);
    if refs.is_empty() {
        return draft(text);
    }
    let mut plain = String::with_capacity(text.len());
    let mut pills = Vec::new();
    let mut cursor = 0;
    for occ in refs {
        plain.push_str(&text[cursor..occ.start]);
        let start = plain.len();
        plain.push_str(&occ.label);
        let end = plain.len();
        pills.push((
            start..end,
            crate::composer_context::format_composer_context_href(&occ.kind, &occ.context_id),
            occ.label,
        ));
        cursor = occ.end;
    }
    plain.push_str(&text[cursor..]);
    let mut edit = draft(plain);
    for (range, id, label) in pills.into_iter().rev() {
        edit.replace_with_pill(range, id, &label);
    }
    edit
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
        let mut edit = draft("");
        let mut rules = Rules::default();
        key(&mut edit, &mut rules, Key::Text("- ".into()));
        assert_eq!(edit.active_block(), Block::Paragraph);
        key(&mut edit, &mut rules, Key::Text("a".into()));
        assert_eq!(edit.active_block(), Block::Bullet(0));
        assert_eq!(markdown(&edit), "- a");
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
            "**bold *italic***\n- item\n> quoted\n````\nx`y\n```\n````\nend"
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
