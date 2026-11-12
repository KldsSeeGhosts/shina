//! Direct port of KSG Code's composer-rich-text.ts. This is deliberately not
//! CommonMark. In particular escapes stay in the text and only single ticks style code.
use rune::{Mark, MarkRange, TextEdit};
use std::ops::Range;

pub fn space(ch: char) -> bool {
    matches!(ch, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
        | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub range: Range<usize>,
    pub marks: Vec<Mark>,
}

struct Frame {
    delimiter: String,
    mark: Mark,
    start: usize,
    pieces: Vec<Piece>,
}

fn push(root: &mut Vec<Piece>, stack: &mut [Frame], piece: Piece) {
    if piece.range.is_empty() {
        return;
    }
    let pieces = stack.last_mut().map_or(root, |frame| &mut frame.pieces);
    if let Some(last) = pieces.last_mut()
        && last.range.end == piece.range.start
        && last.marks == piece.marks
    {
        last.range.end = piece.range.end;
    } else {
        pieces.push(piece);
    }
}

pub fn parse(text: &str) -> Vec<Piece> {
    let mut root = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut index = 0;
    let plain = |range| Piece {
        range,
        marks: vec![],
    };
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap();
        let next = index + ch.len_utf8();
        if ch == '\\' {
            let end = next + text[next..].chars().next().map_or(0, char::len_utf8);
            push(&mut root, &mut stack, plain(index..end));
            index = end;
            continue;
        }
        if ch == '`' {
            let length = text[index..].bytes().take_while(|c| *c == b'`').count();
            let ticks = &text[index..index + length];
            let close = text[index + length..]
                .find(ticks)
                .map(|i| index + length + i);
            if let Some(close) = close
                && length == 1
                && close > index + 1
                && !text[index..close].contains('\n')
                && text.as_bytes().get(close + 1) != Some(&b'`')
            {
                push(
                    &mut root,
                    &mut stack,
                    Piece {
                        range: index + 1..close,
                        marks: vec![Mark::Code],
                    },
                );
                index = close + 1;
            } else {
                push(&mut root, &mut stack, plain(index..index + length));
                index += length;
            }
            continue;
        }
        if !matches!(ch, '*' | '_' | '~') {
            push(&mut root, &mut stack, plain(index..next));
            index = next;
            continue;
        }
        let end = index + text[index..].bytes().take_while(|c| *c == ch as u8).count();
        let before = text[..index].chars().next_back();
        let after = text[end..].chars().next();
        let word = |ch: char| ch.is_ascii_alphanumeric() || ch == '_';
        let can_close =
            before.is_some_and(|c| !space(c)) && (ch != '_' || !after.is_some_and(word));
        let can_open = after.is_some_and(|c| !space(c)) && (ch != '_' || !before.is_some_and(word));
        while index < end {
            let nested_bold = stack.last().is_some_and(|f| f.mark == Mark::Italic)
                && end - index == 2
                && can_open
                && matches!(ch, '*' | '_')
                && !stack.iter().any(|f| f.mark == Mark::Bold);
            if !nested_bold
                && can_close
                && stack.last().is_some_and(|f| {
                    text[index..].starts_with(&f.delimiter) && index + f.delimiter.len() <= end
                })
            {
                let frame = stack.pop().unwrap();
                for mut piece in frame.pieces {
                    piece.marks.insert(0, frame.mark.clone());
                    push(&mut root, &mut stack, piece);
                }
                index += frame.delimiter.len();
            } else if can_open && (ch != '~' || end - index >= 2) {
                let length = if ch == '~' || end - index >= 2 { 2 } else { 1 };
                let mark = if ch == '~' {
                    Mark::Strike
                } else if length == 2 {
                    Mark::Bold
                } else {
                    Mark::Italic
                };
                if stack.iter().any(|f| f.mark == mark) {
                    push(&mut root, &mut stack, plain(index..end));
                    index = end;
                    continue;
                }
                stack.push(Frame {
                    delimiter: ch.to_string().repeat(length),
                    mark,
                    start: index,
                    pieces: vec![],
                });
                index += length;
            } else {
                push(&mut root, &mut stack, plain(index..end));
                index = end;
            }
        }
    }
    while let Some(frame) = stack.pop() {
        push(
            &mut root,
            &mut stack,
            plain(frame.start..frame.start + frame.delimiter.len()),
        );
        for piece in frame.pieces {
            push(&mut root, &mut stack, piece);
        }
    }
    root
}

pub fn document(source: &str) -> TextEdit {
    let mut text = String::new();
    let mut marks = Vec::new();
    for piece in parse(source) {
        let start = text.len();
        text.push_str(&source[piece.range]);
        for mark in piece.marks {
            marks.push(MarkRange::new(start..text.len(), mark));
        }
    }
    let mut edit = super::draft(text);
    edit.set_marks(marks);
    edit
}

pub const ORDER: [Mark; 4] = [Mark::Strike, Mark::Bold, Mark::Italic, Mark::Code];

pub fn delimiter(mark: &Mark) -> &'static str {
    match mark {
        Mark::Bold => "**",
        Mark::Italic => "*",
        Mark::Strike => "~~",
        Mark::Code => "`",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ksg_inline_parser_cases() {
        for (source, expected) in [
            ("plain text", "plain text"),
            ("snake_case", "snake_case"),
            ("unmatched **", "unmatched **"),
            ("** spaced **", "** spaced **"),
            ("\\*literal\\*", "\\*literal\\*"),
            ("***both***", "***both***"),
            ("*a **b** c*", "*a **b** c*"),
            ("**a `code` c**", "**a `code` c**"),
            ("*a**b*****c**", "*a**b*****c**"),
            ("`**not bold**`", "`**not bold**`"),
            ("__bold__", "**bold**"),
            ("_italic_", "*italic*"),
        ] {
            let edit = document(source);
            assert_eq!(super::super::markdown(&edit), expected, "{source}");
        }
        let edit = document("**a `code` c**");
        assert_eq!(edit.text, "a code c");
        assert!(edit.marks().contains(&MarkRange::new(0..8, Mark::Bold)));
        assert!(edit.marks().contains(&MarkRange::new(2..6, Mark::Code)));
        let repeated = "*".repeat(10_000);
        let edit = document(&format!("{repeated}text{repeated}"));
        assert_eq!(
            edit.text,
            format!("{}text{}", &repeated[2..], &repeated[2..])
        );
    }
}
