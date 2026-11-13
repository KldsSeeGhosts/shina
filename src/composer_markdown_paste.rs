//! KSG Code's buildTiptapContent and handlePaste, ported to Rune authored blocks.
use super::{draft, source};
use rune::{Block, BlockAttributes, BlockKind, MAX_LIST_LEVEL, MarkRange, TextEdit};
use std::{ops::Range, sync::LazyLock};

pub type Resolver<'a> = &'a dyn Fn(&str) -> Option<(String, String)>;

fn regex(source: &str) -> regex::Regex {
    regex::Regex::new(source).unwrap()
}

/// Ported from shared/composerInlineTokens.ts. The resolver decides which skills
/// are known. Mentions exclude package references and decorator calls.
pub fn tokens(text: &str) -> Vec<Range<usize>> {
    static TOKENS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex(r#"(^|\s)(@(?:"(?:\\.|[^"\\])*"|[^\s@"]+)|\p{Sc}[a-zA-Z0-9][a-zA-Z0-9:_-]*)"#)
    });
    static PACKAGE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex(r#"^[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*(?:/[^\s@"]+)*$"#));
    static DECORATOR: LazyLock<regex::Regex> =
        LazyLock::new(|| regex(r"^[\p{XID_Start}_$][\p{XID_Continue}.$\u{200c}\u{200d}]*\("));
    let mut ranges = Vec::new();
    for occ in crate::composer_context::collect_composer_context_references(text) {
        ranges.push(occ.start..occ.end);
    }
    static LINKS: LazyLock<regex::Regex> =
        LazyLock::new(|| regex(r"(^|\s)(\[(?:\\.|[^\]\\]){0,512}\]\([^\s)]+\))"));
    for capture in LINKS.captures_iter(text) {
        if !capture[1].chars().all(source::space) {
            continue;
        }
        let token = capture.get(2).unwrap();
        if text[token.end()..].chars().next().is_none_or(source::space)
            && mention_path(token.as_str()).is_some()
            && !ranges
                .iter()
                .any(|r| r.start < token.end() && token.start() < r.end)
        {
            ranges.push(token.range());
        }
    }
    for capture in TOKENS.captures_iter(text) {
        if !capture[1].chars().all(source::space) {
            continue;
        }
        let token = capture.get(2).unwrap();
        if !text[token.end()..].chars().next().is_none_or(source::space) {
            continue;
        }
        let value = token.as_str();
        if let Some(path) = value.strip_prefix('@') {
            if path.is_empty()
                || !path.starts_with('"') && (PACKAGE.is_match(path) || DECORATOR.is_match(path))
            {
                continue;
            }
        } else {
            let name = &value[value.chars().next().unwrap().len_utf8()..];
            if !name.bytes().any(|c| c.is_ascii_alphabetic()) {
                continue;
            }
            // KSG rejects currency amounts such as $20k and $20e3, even if a
            // similarly named skill exists.
            static AMOUNT: LazyLock<regex::Regex> =
                LazyLock::new(|| regex(r"^[0-9][0-9_]*(?:[kKmMbBtT]|[eE][0-9]+)?$"));
            if AMOUNT.is_match(name) {
                continue;
            }
        }
        if !ranges
            .iter()
            .any(|r| r.start < token.end() && token.start() < r.end)
        {
            ranges.push(token.range());
        }
    }
    ranges.sort_by_key(|r| r.start);
    ranges
}

fn unescape(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        result.push(if ch == '\\' {
            chars.next().unwrap_or(ch)
        } else {
            ch
        });
    }
    result
}

pub fn mention_path(source: &str) -> Option<String> {
    if let Some(path) = source.strip_prefix('@') {
        return Some(
            path.strip_prefix('"')
                .and_then(|p| p.strip_suffix('"'))
                .map_or_else(|| path.to_owned(), unescape),
        );
    }
    let (label, encoded) = source.strip_prefix('[')?.rsplit_once("](")?;
    let encoded = encoded.strip_suffix(')')?;
    let mut bytes = Vec::new();
    let mut at = 0;
    let mut valid = true;
    while at < encoded.len() {
        if encoded.as_bytes()[at] == b'%'
            && let Some(hex) = encoded.get(at + 1..at + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            bytes.push(byte);
            at += 3;
        } else {
            if encoded.as_bytes()[at] == b'%' {
                valid = false;
            }
            bytes.push(encoded.as_bytes()[at]);
            at += 1;
        }
    }
    let path = if valid {
        String::from_utf8(bytes).unwrap_or_else(|_| encoded.into())
    } else {
        encoded.into()
    };
    static SCHEME: LazyLock<regex::Regex> = LazyLock::new(|| regex(r"^[A-Za-z][A-Za-z0-9+.-]*:"));
    static DRIVE: LazyLock<regex::Regex> = LazyLock::new(|| regex(r"^[A-Za-z]:[\\/]"));
    let basename = path.rsplit(['/', '\\']).next()?;
    (!path.is_empty()
        && unescape(label) == basename
        && (!SCHEME.is_match(&path) || DRIVE.is_match(&path)))
    .then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ksg_token_boundaries_aliases_and_amounts() {
        for (text, sources) in [
            (
                "€my-skill £20 ¥20k ₹20e3 ₿known",
                vec!["€my-skill", "₿known"],
            ),
            (
                "@README.md @\"docs/My File.md\" @scope/package @decorator()",
                vec!["@README.md", "@\"docs/My File.md\""],
            ),
            (
                "[My File.md](docs/My%20File.md) [site](https://example.com)",
                vec!["[My File.md](docs/My%20File.md)"],
            ),
        ] {
            assert_eq!(
                tokens(text)
                    .iter()
                    .map(|r| &text[r.clone()])
                    .collect::<Vec<_>>(),
                sources
            );
        }
        assert_eq!(
            mention_path("[My File.md](docs/My%20File.md)").as_deref(),
            Some("docs/My File.md")
        );
        assert!(mention_path("[wrong](docs/file.md)").is_none());
        assert_eq!(
            mention_path("[x%20%XX](x%20%XX)").as_deref(),
            Some("x%20%XX")
        );
    }

    #[test]
    fn unrepresentable_blocks_keep_source_and_fences_stay_literal() {
        for text in [
            "+ item\n* other",
            "07)  item",
            "~~~py title=example\n**bold** @README.md $known\n~~~~ ",
            "```ts\nconst x = 1",
        ] {
            let edit = fragment(text, true, true, 0, &|_| {
                Some(("token".into(), "label".into()))
            });
            assert_eq!(super::super::markdown(&edit), text);
            assert!(edit.blocks().iter().all(|b| *b == Block::Paragraph));
            if text.starts_with("~~~") || text.starts_with("```") {
                assert!(edit.marks().is_empty());
            }
        }
    }
}

fn inline(text: &str, resolve: Resolver<'_>) -> TextEdit {
    let tokens: Vec<_> = tokens(text)
        .into_iter()
        .filter_map(|range| resolve(&text[range.clone()]).map(|pill| (range, pill)))
        .collect();
    let mut sentinel = '\u{fffc}';
    for candidate in 0xe000.. {
        if !text.contains(sentinel) {
            break;
        }
        sentinel = char::from_u32(candidate).unwrap();
    }
    let mut masked = String::new();
    let mut cursor = 0;
    for (range, _) in &tokens {
        masked.push_str(&text[cursor..range.start]);
        masked.push(sentinel);
        cursor = range.end;
    }
    masked.push_str(&text[cursor..]);
    let mut edit = source::document(&masked);
    let positions: Vec<_> = edit
        .text
        .match_indices(sentinel)
        .map(|(at, _)| at)
        .collect();
    for (at, (_, (id, label))) in positions.into_iter().zip(tokens).rev() {
        let inherited: Vec<_> = edit
            .marks()
            .iter()
            .filter(|m| m.start <= at && at + sentinel.len_utf8() <= m.end)
            .map(|m| m.mark.clone())
            .collect();
        edit.replace_with_pill(at..at + sentinel.len_utf8(), id, &label);
        // Rune's pill replacement intentionally inserts without inherited
        // marks. KSG assigns the placeholder's marks to the atom explicitly.
        let mut marks = edit.marks().to_vec();
        marks.extend(
            inherited
                .into_iter()
                .map(|mark| MarkRange::new(at..edit.focus, mark)),
        );
        edit.set_marks(marks);
    }
    edit
}

pub fn opening_fence(line: &str) -> Option<(&str, &str)> {
    let ch = line.chars().next()?;
    if !matches!(ch, '`' | '~') {
        return None;
    }
    let length = line.bytes().take_while(|b| *b == ch as u8).count();
    let (fence, info) = line.split_at(length);
    (length >= 3 && !(ch == '`' && info.contains('`'))).then_some((fence, info))
}

pub fn closes(line: &str, fence: &str) -> bool {
    let line = line.trim_end_matches([' ', '\t', '\r']);
    line.len() >= fence.len() && line.bytes().all(|b| b == fence.as_bytes()[0])
}

fn parse_rule(line: &str) -> bool {
    let mut chars = line.chars();
    let Some(marker) = chars.next() else {
        return false;
    };
    if !matches!(marker, '-' | '*' | '_') {
        return false;
    }
    let mut count = 1usize;
    for ch in chars {
        if ch == marker {
            count += 1;
        } else if !matches!(ch, ' ' | '\t') {
            return false;
        }
    }
    count >= 3
}

fn parse_heading(line: &str) -> Option<(u8, &str, &str)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    let space_len = rest
        .bytes()
        .take_while(|b| matches!(*b, b' ' | b'\t'))
        .count();
    if space_len == 0 {
        return None;
    }
    Some((hashes as u8, &rest[..space_len], &rest[space_len..]))
}

struct TaskPrefix<'a> {
    indent: &'a str,
    marker_space: &'a str,
    check_char: char,
    content_space: &'a str,
    content: &'a str,
}

fn parse_task(line: &str) -> Option<TaskPrefix<'_>> {
    let indent_len = line
        .bytes()
        .take_while(|b| matches!(*b, b' ' | b'\t'))
        .count();
    let indent = &line[..indent_len];
    let after_indent = &line[indent_len..];
    let after_dash = after_indent.strip_prefix('-')?;
    let marker_space_len = after_dash
        .bytes()
        .take_while(|b| matches!(*b, b' ' | b'\t'))
        .count();
    if marker_space_len == 0 {
        return None;
    }
    let marker_space = &after_dash[..marker_space_len];
    let after_space = &after_dash[marker_space_len..];
    let rest = after_space.strip_prefix('[')?;
    let check_char = rest.chars().next()?;
    if !matches!(check_char, ' ' | 'x' | 'X') {
        return None;
    }
    let after_box = rest[1..].strip_prefix(']')?;
    let content_space_len = after_box
        .bytes()
        .take_while(|b| matches!(*b, b' ' | b'\t'))
        .count();
    let content_space = &after_box[..content_space_len];
    let content = &after_box[content_space_len..];
    if !content.is_empty() && content_space.is_empty() {
        return None;
    }
    Some(TaskPrefix {
        indent,
        marker_space,
        check_char,
        content_space,
        content,
    })
}

pub(super) fn fragment(
    text: &str,
    blocks: bool,
    fences: bool,
    literal_length: usize,
    resolve: Resolver<'_>,
) -> TextEdit {
    let lines: Vec<_> = text.split('\n').collect();
    let mut parts: Vec<(TextEdit, Vec<(BlockKind, BlockAttributes)>)> = Vec::new();
    let mut index = 0;
    let mut numbers = [0usize; 4];
    let mut list_stack: Vec<(String, String)> = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if index == 0 && literal_length > 0 {
            list_stack.clear();
            let literal = &line[..literal_length.min(line.len())];
            let rest = inline(&line[literal.len()..], resolve);
            let mut part = draft(format!("{literal}{}", rest.text));
            part.set_marks(
                rest.marks()
                    .iter()
                    .map(|m| {
                        MarkRange::new(
                            literal.len() + m.start..literal.len() + m.end,
                            m.mark.clone(),
                        )
                    })
                    .collect(),
            );
            parts.push((
                part,
                vec![(BlockKind::Paragraph, BlockAttributes::default())],
            ));
            index += 1;
            continue;
        }
        if fences && let Some((fence, info)) = opening_fence(line) {
            list_stack.clear();
            let start = index;
            index += 1;
            while index < lines.len() && !closes(lines[index], fence) {
                index += 1;
            }
            let canonical = blocks
                && fence == "```"
                && info.is_empty()
                && lines.get(index) == Some(&"```")
                && parts
                    .last()
                    .is_none_or(|(_, ks)| ks.last().is_none_or(|(k, _)| *k != BlockKind::Code));
            if canonical {
                let mut part = draft(lines[start + 1..index].join("\n"));
                let count = part.text.bytes().filter(|b| *b == b'\n').count() + 1;
                part.set_blocks(vec![Block::Code; count]);
                parts.push((
                    part,
                    vec![(BlockKind::Code, BlockAttributes::default()); count],
                ));
            } else {
                // Do not style code or turn its tokens into pills, even when
                // the fence's attributes cannot be represented.
                for line in &lines[start..(index + usize::from(index < lines.len()))] {
                    parts.push((
                        draft(*line),
                        vec![(BlockKind::Paragraph, BlockAttributes::default())],
                    ));
                }
            }
            index += usize::from(index < lines.len());
            numbers.fill(0);
            continue;
        }
        let mut kind = BlockKind::Paragraph;
        let mut attrs = BlockAttributes::default();
        let mut content = line;
        if blocks {
            if parse_rule(line) {
                list_stack.clear();
                numbers.fill(0);
                kind = BlockKind::Rule;
                attrs = BlockAttributes::default().with_rule_source(line);
                content = "";
            } else if let Some((level, space, heading_content)) = parse_heading(line) {
                list_stack.clear();
                numbers.fill(0);
                kind = BlockKind::Heading { level };
                attrs = BlockAttributes::default().with_heading_space(space);
                content = heading_content;
            } else if let Some(text) = line.strip_prefix("> ")
                && !text.starts_with([' ', '\t'])
            {
                list_stack.clear();
                kind = BlockKind::Quote;
                content = text;
                numbers.fill(0);
            } else if let Some(task) = parse_task(line) {
                let indent = task.indent.to_owned();
                let key = "task".to_owned();
                loop {
                    let stack_len = list_stack.len();
                    let Some((top_indent, top_key)) = list_stack.last_mut() else {
                        list_stack.push((indent.clone(), key));
                        break;
                    };
                    if *top_indent == indent {
                        *top_key = key;
                        break;
                    }
                    if !top_indent.is_empty() && !indent.starts_with(top_indent.as_str()) {
                        if stack_len > 1 {
                            list_stack.pop();
                            continue;
                        }
                        *top_indent = indent.clone();
                        *top_key = key;
                        break;
                    }
                    list_stack.push((indent.clone(), key));
                    break;
                }
                let level = (list_stack.len() - 1).min(MAX_LIST_LEVEL as usize) as u8;
                kind = BlockKind::Task {
                    level,
                    checked: task.check_char != ' ',
                };
                attrs = BlockAttributes::default()
                    .with_indent(task.indent)
                    .with_marker_space(task.marker_space)
                    .with_content_space(task.content_space);
                if task.check_char == 'X' {
                    attrs = attrs.with_marker("X");
                }
                content = task.content;
                numbers.fill(0);
            } else {
                list_stack.clear();
                let indent = line.bytes().take_while(|b| *b == b' ').count();
                let level = indent / 4;
                if indent % 4 == 0 && level <= 3 {
                    let tail = &line[indent..];
                    if let Some(text) = tail.strip_prefix("- ")
                        && !text.starts_with([' ', '\t'])
                        && !(text.starts_with("[ ]")
                            || text.starts_with("[x]")
                            || text.starts_with("[X]"))
                    {
                        kind = BlockKind::Bullet(level as u8);
                        content = text;
                        numbers[level..].fill(0);
                    } else {
                        let prefix = format!("{}. ", numbers[level] + 1);
                        if let Some(text) = tail.strip_prefix(&prefix)
                            && !text.starts_with([' ', '\t'])
                        {
                            numbers[level] += 1;
                            numbers[level + 1..].fill(0);
                            kind = BlockKind::Numbered(level as u8);
                            content = text;
                        }
                    }
                }
                if kind == BlockKind::Paragraph {
                    numbers.fill(0);
                }
            }
        }
        let part = inline(content, resolve);
        parts.push((part, vec![(kind, attrs)]));
        index += 1;
    }
    let mut text = String::new();
    let mut marks = Vec::new();
    let mut logical_meta: Vec<(usize, usize, BlockKind, BlockAttributes)> = Vec::new();
    let mut line_cursor = 0usize;
    for (index, (part, meta)) in parts.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        let offset = text.len();
        text.push_str(&part.text);
        marks.extend(
            part.marks()
                .iter()
                .map(|m| MarkRange::new(m.start + offset..m.end + offset, m.mark.clone())),
        );
        let count = meta.len();
        if count > 0 {
            if meta
                .iter()
                .all(|(k, a)| *k == BlockKind::Code && *a == meta[0].1)
            {
                logical_meta.push((
                    line_cursor,
                    line_cursor + count - 1,
                    BlockKind::Code,
                    meta[0].1.clone(),
                ));
            } else {
                for (j, (kind, attrs)) in meta.iter().enumerate() {
                    if *kind != BlockKind::Paragraph || *attrs != BlockAttributes::default() {
                        logical_meta.push((line_cursor + j, line_cursor + j, *kind, attrs.clone()));
                    }
                }
            }
        }
        line_cursor += count;
    }
    let mut edit = draft(text);
    for (first_line, last_line, kind, attrs) in logical_meta {
        let range = edit.line_range(first_line).start..edit.line_range(last_line).end;
        let _ = edit.set_logical_block(range, kind, attrs);
    }
    edit.set_marks(marks);
    edit.clear_history();
    edit
}

#[cfg(test)]
pub fn paste(edit: &mut TextEdit, text: &str, resolve: Resolver<'_>) {
    paste_with_path_query(edit, text, false, resolve);
}

pub fn paste_with_path_query(
    edit: &mut TextEdit,
    text: &str,
    path_query: bool,
    resolve: Resolver<'_>,
) {
    let mut text = text.replace("\r\n", "\n").replace('\r', "\n");
    if edit.active_block() == Block::Code {
        edit.insert(&text);
        return;
    }
    let found = tokens(&text);
    if found.last().is_some_and(|r| {
        r.end == text.len()
            && !text[r.clone()].contains("](t3-context:")
            && resolve(&text[r.clone()]).is_some()
    }) {
        text.push(' ');
    }
    if found.first().is_some_and(|r| {
        r.start == 0
            && !text[r.clone()].contains("](t3-context:")
            && resolve(&text[r.clone()]).is_some()
    }) && edit.range().0 > 0
        && edit.text[..edit.range().0]
            .chars()
            .next_back()
            .is_some_and(|c| !source::space(c))
    {
        text.insert(0, ' ');
    }
    let literal_length = if path_query {
        text.chars()
            .take_while(|ch| !source::space(*ch))
            .map(char::len_utf8)
            .sum()
    } else {
        0
    };
    let (from, to) = edit.range();
    let line = edit.line_range(edit.line_of_byte(from));
    let last_line = edit.line_range(edit.line_of_byte(to));
    let first_index = edit.line_of_byte(from);
    let last_index = edit.line_of_byte(to);
    let blocks = edit.block_kind(first_index) == BlockKind::Paragraph;
    let mut fragment = fragment(&text, blocks, blocks, literal_length, resolve);
    let prefix = from > line.start;
    let suffix = to < last_line.end;
    if !prefix
        && first_index > 0
        && edit.block_kind(first_index - 1) == BlockKind::Code
        && fragment.block_kind(0) == BlockKind::Code
        || !suffix
            && edit.blocks().get(last_index + 1) == Some(&Block::Code)
            && fragment.block_kind(fragment.blocks().len() - 1) == BlockKind::Code
    {
        // Adjacent fences require identity that Rune cannot retain. Preserve
        // the authored fence rather than accidentally merging two code blocks.
        fragment = self::fragment(&text, false, blocks, literal_length, resolve);
    }
    let fragment_lines = fragment.blocks().len();
    let structural = blocks
        && (fragment_lines > 1
            || fragment.block_kind(0) != BlockKind::Paragraph
            || fragment.block_attributes(0) != &BlockAttributes::default());
    let before = if structural && prefix { "\n" } else { "" };
    let after = if structural && suffix { "\n" } else { "" };
    let replacement = format!("{before}{}{after}", fragment.text);
    let content_start = from + before.len();
    let content_end = content_start + fragment.text.len();
    let suffix_kind = edit.block_kind(last_index);
    let suffix_attrs = edit.block_attributes(last_index).clone();
    // Rune's HTML importer drops a leading space before a pill. Insert the
    // app-built fragment directly with public editing APIs, in one undo group.
    let inherit = literal_length == 0
        && fragment_lines == 1
        && fragment.block_kind(0) == BlockKind::Paragraph
        && fragment.block_attributes(0) == &BlockAttributes::default()
        && fragment.marks().is_empty();
    edit.undo_group(|edit| {
        edit.insert(&replacement);
        let end = from + replacement.len();
        if !inherit {
            let mut marks = Vec::new();
            for mark in edit.marks() {
                if mark.start >= end || mark.end <= from {
                    marks.push(mark.clone());
                } else {
                    if mark.start < from {
                        marks.push(MarkRange::new(mark.start..from, mark.mark.clone()));
                    }
                    if mark.end > end {
                        marks.push(MarkRange::new(end..mark.end, mark.mark.clone()));
                    }
                }
            }
            marks.extend(fragment.marks().iter().map(|m| {
                MarkRange::new(
                    content_start + m.start..content_start + m.end,
                    m.mark.clone(),
                )
            }));
            edit.set_marks(marks);
        }
        if blocks {
            let first = edit.line_of_byte(content_start);
            for lb in fragment.logical_blocks() {
                let b_first = fragment.line_of_byte(lb.range.start);
                let b_last = fragment.line_of_byte(lb.range.end);
                let range =
                    edit.line_range(first + b_first).start..edit.line_range(first + b_last).end;
                let _ = edit.set_logical_block(range, lb.kind, lb.attributes);
            }
            if structural && suffix {
                let suffix_line = edit.line_range(first + fragment_lines);
                let _ = edit.set_logical_block(suffix_line, suffix_kind, suffix_attrs);
            }
        }
        edit.set_caret(content_end);
    });
}
