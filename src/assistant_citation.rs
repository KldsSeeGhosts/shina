//! Ported from KSG Code's `packages/contracts/src/assistantCitations.ts`,
//! `packages/shared/src/assistantCitations.ts`, `apps/web/src/lib/assistantTextSelection.ts`,
//! and `apps/web/src/lib/composerContextRecords.ts`.

use crate::composer_context::{
    AssistantCitation, ComposerContextData, ComposerContextRecord, ResponseSubagentRef,
    sanitize_composer_context_label, to_kind_scoped_composer_context_id,
};
use crate::model::Message;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

pub const ASSISTANT_CITATION_MAX_TEXT_LENGTH: usize = 8_000;
/// KSG's `ASSISTANT_CITATION_MAX_COMMENT_LENGTH`, counted in UTF-16 code units.
pub const ASSISTANT_CITATION_MAX_COMMENT_LENGTH: usize = 8_000;
pub const ASSISTANT_CITATION_CONTEXT_LENGTH: usize = 32;
pub const ASSISTANT_CITATION_MAX_ID_LENGTH: usize = 4_096;
pub const ASSISTANT_CITATION_LABEL_MAX_CHARS: usize = 64;
pub const RESPONSE_OPENING_LENGTH: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantTextSelector {
    pub text: String,
    pub start: u64,
    pub end: u64,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CitationBuildError {
    Empty,
    TooLong,
    InvalidId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantSelectionTarget {
    pub message_index: usize,
    pub message_id: String,
    pub chunk_from: Option<usize>,
}

pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

pub fn normalize_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ws = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(ch);
            in_ws = false;
        }
    }
    out
}

/// Convert a UTF-16 code-unit index in `s` into a UTF-8 byte index.
/// If `utf16_offset` lands in the middle of a surrogate pair, `bias_right`
/// chooses the end of that character (`true`) or its start (`false`).
fn utf16_to_byte(s: &str, utf16_offset: usize, bias_right: bool) -> usize {
    let mut units = 0usize;
    for (byte_idx, ch) in s.char_indices() {
        if units == utf16_offset {
            return byte_idx;
        }
        let next_units = units + ch.len_utf16();
        if utf16_offset < next_units {
            return if bias_right {
                byte_idx + ch.len_utf8()
            } else {
                byte_idx
            };
        }
        units = next_units;
    }
    s.len()
}

fn slice_utf16(s: &str, start_u16: usize, end_u16: usize) -> &str {
    let start_b = utf16_to_byte(s, start_u16, true);
    let end_b = utf16_to_byte(s, end_u16, false).max(start_b);
    &s[start_b..end_b]
}

/// Ported from KSG Code's `createAssistantTextSelector` (`assistantTextSelection.ts`).
pub fn create_assistant_text_selector(
    text: &str,
    raw_start: usize,
    raw_end: usize,
) -> Option<AssistantTextSelector> {
    if raw_start > raw_end
        || raw_end > text.len()
        || !text.is_char_boundary(raw_start)
        || !text.is_char_boundary(raw_end)
    {
        return None;
    }
    let quote = &text[raw_start..raw_end];
    if quote.trim().is_empty() {
        return None;
    }
    let normalized = normalize_whitespace(text);
    let mut start = utf16_len(&normalize_whitespace(&text[..raw_start]));
    if raw_start > 0
        && text[..raw_start]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
        && text[raw_start..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
    {
        start = start.saturating_sub(1);
    }
    let end = utf16_len(&normalize_whitespace(&text[..raw_end]));
    let normalized_len = utf16_len(&normalized);
    let prefix_start = start.saturating_sub(ASSISTANT_CITATION_CONTEXT_LENGTH);
    let suffix_end = (end + ASSISTANT_CITATION_CONTEXT_LENGTH).min(normalized_len);
    let prefix = slice_utf16(&normalized, prefix_start, start).to_owned();
    let suffix = slice_utf16(&normalized, end, suffix_end).to_owned();
    Some(AssistantTextSelector {
        text: quote.to_owned(),
        start: start as u64,
        end: end as u64,
        prefix,
        suffix,
    })
}

/// Ported from KSG Code's `assistantCitationLabel` (`packages/shared/src/assistantCitations.ts`).
pub fn assistant_citation_label(citation: &AssistantCitation) -> String {
    let source = citation
        .comment
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or(citation.text.as_str());
    let preview = normalize_whitespace(source);
    if utf16_len(&preview) <= ASSISTANT_CITATION_LABEL_MAX_CHARS {
        return preview;
    }
    let end = utf16_to_byte(&preview, ASSISTANT_CITATION_LABEL_MAX_CHARS, false);
    format!("{}…", &preview[..end])
}

/// Extracts plain rendered text from Markdown, mirroring KSG's `readAssistantText`.
pub fn markdown_rendered_text(markdown: &str) -> String {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut out = String::with_capacity(markdown.len());
    let mut separator = false;
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::BlockQuote(_)
                | Tag::CodeBlock(_)
                | Tag::List(_)
                | Tag::Item
                | Tag::Table(_)
                | Tag::TableHead
                | Tag::TableRow
                | Tag::TableCell => {
                    separator = true;
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::Table
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::TableCell => {
                    separator = true;
                }
                _ => {}
            },
            Event::Text(text) | Event::Code(text) => {
                if !text.is_empty() {
                    if separator && !out.is_empty() {
                        out.push('\n');
                    }
                    separator = false;
                    out.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak | Event::Rule => separator = true,
            _ => {}
        }
    }
    out
}

/// Locate `selected_text` inside `message_text` (or its rendered Markdown text) and return
/// `(source_text, raw_start, raw_end)` for `create_assistant_text_selector`.
pub fn locate_selection_in_message(
    message_text: &str,
    chunk_from: Option<usize>,
    selected_text: &str,
) -> (String, usize, usize) {
    let rendered = markdown_rendered_text(message_text);
    let source = if rendered.trim().is_empty() {
        message_text.to_owned()
    } else {
        rendered
    };
    if let Some(from) = chunk_from
        && from <= message_text.len()
        && message_text.is_char_boundary(from)
    {
        let prefix_rendered_len = markdown_rendered_text(&message_text[..from]).len();
        let search_from = prefix_rendered_len.min(source.len());
        let search_from = if source.is_char_boundary(search_from) {
            search_from
        } else {
            0
        };
        if let Some(rel) = source[search_from..].find(selected_text) {
            let start = search_from + rel;
            return (source, start, start + selected_text.len());
        }
    }
    if let Some(start) = source.find(selected_text) {
        return (source, start, start + selected_text.len());
    }
    if let Some(start) = message_text.find(selected_text) {
        return (message_text.to_owned(), start, start + selected_text.len());
    }
    // Fall back to whitespace-normalized match in rendered text.
    let norm_selected = normalize_whitespace(selected_text.trim());
    if !norm_selected.is_empty() {
        let mut norm_source = String::with_capacity(source.len());
        let mut raw_spans: Vec<(usize, usize)> = Vec::new();
        let mut in_ws = false;
        let mut ws_start = 0usize;
        for (idx, ch) in source.char_indices() {
            if ch.is_whitespace() {
                if !in_ws {
                    ws_start = idx;
                    in_ws = true;
                }
            } else {
                if in_ws {
                    norm_source.push(' ');
                    raw_spans.push((ws_start, idx));
                    in_ws = false;
                }
                let start_b = norm_source.len();
                norm_source.push(ch);
                let _ = start_b;
                raw_spans.push((idx, idx + ch.len_utf8()));
            }
        }
        if in_ws {
            norm_source.push(' ');
            raw_spans.push((ws_start, source.len()));
        }
        if let Some(norm_pos) = norm_source.find(&norm_selected) {
            let norm_end = norm_pos + norm_selected.len();
            let char_start = norm_source[..norm_pos].chars().count();
            let char_end = char_start + norm_source[norm_pos..norm_end].chars().count();
            if let (Some(first), Some(last)) = (
                raw_spans.get(char_start),
                char_end.checked_sub(1).and_then(|i| raw_spans.get(i)),
            ) {
                return (source, first.0, last.1);
            }
        }
    }
    (selected_text.to_owned(), 0, selected_text.len())
}

/// Ported from KSG Code's `withAssistantCitationComment`
/// (`packages/shared/src/assistantCitations.ts`): the comment is trimmed and a
/// blank one removes the field.
pub fn with_assistant_citation_comment(
    citation: &AssistantCitation,
    comment: &str,
) -> AssistantCitation {
    let mut next = citation.clone();
    let trimmed = comment.trim();
    next.comment = (!trimmed.is_empty()).then(|| trimmed.to_owned());
    next
}

pub fn build_assistant_citation(
    environment_id: &str,
    thread_id: &str,
    message_id: &str,
    source_text: &str,
    raw_start: usize,
    raw_end: usize,
) -> Result<AssistantCitation, CitationBuildError> {
    if environment_id.is_empty()
        || thread_id.is_empty()
        || message_id.is_empty()
        || utf16_len(environment_id) > ASSISTANT_CITATION_MAX_ID_LENGTH
        || utf16_len(thread_id) > ASSISTANT_CITATION_MAX_ID_LENGTH
        || utf16_len(message_id) > ASSISTANT_CITATION_MAX_ID_LENGTH
    {
        return Err(CitationBuildError::InvalidId);
    }
    let selector = create_assistant_text_selector(source_text, raw_start, raw_end)
        .ok_or(CitationBuildError::Empty)?;
    if utf16_len(&selector.text) > ASSISTANT_CITATION_MAX_TEXT_LENGTH {
        return Err(CitationBuildError::TooLong);
    }
    Ok(AssistantCitation {
        version: 1,
        environment_id: environment_id.to_owned(),
        thread_id: thread_id.to_owned(),
        message_id: message_id.to_owned(),
        text: selector.text,
        comment: None,
        start: selector.start,
        end: selector.end,
        prefix: selector.prefix,
        suffix: selector.suffix,
    })
}

pub fn build_assistant_citation_from_selection(
    environment_id: &str,
    thread_id: &str,
    message_id: &str,
    message_text: &str,
    chunk_from: Option<usize>,
    selected_text: &str,
) -> Result<AssistantCitation, CitationBuildError> {
    if selected_text.trim().is_empty() {
        return Err(CitationBuildError::Empty);
    }
    if utf16_len(selected_text) > ASSISTANT_CITATION_MAX_TEXT_LENGTH {
        return Err(CitationBuildError::TooLong);
    }
    let (source, raw_start, raw_end) =
        locate_selection_in_message(message_text, chunk_from, selected_text);
    build_assistant_citation(
        environment_id,
        thread_id,
        message_id,
        &source,
        raw_start,
        raw_end,
    )
}

pub fn citation_context_record(
    citation: AssistantCitation,
    item_id: &str,
    subagents: Option<Vec<ResponseSubagentRef>>,
) -> ComposerContextRecord {
    let producer_id = uuid::Uuid::new_v4().simple().to_string();
    let label = sanitize_composer_context_label(&assistant_citation_label(&citation), "response");
    ComposerContextRecord {
        version: 1,
        context_id: to_kind_scoped_composer_context_id("response", &producer_id),
        label,
        data: ComposerContextData::Response {
            citation,
            item_id: item_id.to_owned(),
            subagents: subagents.filter(|list| !list.is_empty()).map(|list| {
                list.into_iter()
                    .take(20)
                    .map(|s| ResponseSubagentRef {
                        thread_id: s.thread_id,
                        title: sanitize_composer_context_label(&s.title, "response"),
                        environment_id: s.environment_id,
                    })
                    .collect()
            }),
        },
    }
}

/// Ported from KSG Code's `captureAssistantMessageOpening` (`assistantTextSelection.ts`).
pub fn capture_assistant_message_opening(message_text: &str) -> Option<AssistantTextSelector> {
    let rendered = markdown_rendered_text(message_text);
    let text = if rendered.trim().is_empty() {
        message_text
    } else {
        rendered.as_str()
    };
    let trimmed_start = text.trim_start();
    if trimmed_start.trim().is_empty() {
        return None;
    }
    let raw_start = text.len() - trimmed_start.len();
    let trimmed_end = text.trim_end().len();
    let max_span = utf16_to_byte(
        &text[raw_start..trimmed_end],
        RESPONSE_OPENING_LENGTH,
        false,
    );
    let raw_end = (raw_start + max_span).min(trimmed_end);
    create_assistant_text_selector(text, raw_start, raw_end)
}

/// Ported from KSG Code's `responseContextRecord` (`composerContextRecords.ts`).
/// One record per response, so replying to it again reuses the same reference.
pub fn response_context_record(
    citation: AssistantCitation,
    item_id: &str,
    subagents: Option<Vec<ResponseSubagentRef>>,
) -> ComposerContextRecord {
    let context_id = to_kind_scoped_composer_context_id("response", &citation.message_id);
    let preview = slice_utf16(&citation.text, 0, ASSISTANT_CITATION_LABEL_MAX_CHARS);
    let label = sanitize_composer_context_label(preview, "response");
    ComposerContextRecord {
        version: 1,
        context_id,
        label,
        data: ComposerContextData::Response {
            citation,
            item_id: item_id.to_owned(),
            subagents: subagents.filter(|list| !list.is_empty()).map(|list| {
                list.into_iter()
                    .take(20)
                    .map(|s| ResponseSubagentRef {
                        thread_id: s.thread_id,
                        title: sanitize_composer_context_label(&s.title, "response"),
                        environment_id: s.environment_id,
                    })
                    .collect()
            }),
        },
    }
}

fn parse_assistant_body_path(path: &str, thread_id: usize) -> Option<(usize, usize)> {
    let prefix = format!("message-{thread_id}-");
    for segment in path.split('/') {
        let Some(rest) = segment.strip_prefix(&prefix) else {
            continue;
        };
        let Some((msg_str, from_str)) = rest.split_once("-body-") else {
            continue;
        };
        let Ok(msg_idx) = msg_str.parse::<usize>() else {
            continue;
        };
        let Ok(from) = from_str.parse::<usize>() else {
            continue;
        };
        return Some((msg_idx, from));
    }
    None
}

/// Validates that a Rune text selection lies completely inside a single assistant (`"Pi"`)
/// message of `thread_id`. Rejects empty/whitespace selections and selections spanning multiple
/// messages or non-assistant elements.
pub fn validate_single_assistant_selection(
    thread_id: usize,
    messages: &[Message],
    selected_paths: &[&str],
    selected_text: &str,
) -> Option<AssistantSelectionTarget> {
    if selected_text.trim().is_empty() || selected_paths.is_empty() {
        return None;
    }
    let mut target_idx: Option<usize> = None;
    let mut target_from: Option<usize> = None;
    for path in selected_paths {
        let (msg_idx, from) = parse_assistant_body_path(path, thread_id)?;
        let msg = messages.get(msg_idx)?;
        if msg.role != "Pi" {
            return None;
        }
        match target_idx {
            Some(prev) if prev != msg_idx => return None,
            Some(_) => {
                if target_from != Some(from) {
                    target_from = None;
                }
            }
            None => {
                target_idx = Some(msg_idx);
                target_from = Some(from);
            }
        }
    }
    let msg_idx = target_idx?;
    Some(AssistantSelectionTarget {
        message_index: msg_idx,
        message_id: messages[msg_idx].id.clone(),
        chunk_from: target_from,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, role: &'static str, text: &str) -> Message {
        Message {
            id: id.into(),
            role,
            text: text.into(),
            images: Vec::new(),
            context: None,
        }
    }

    #[test]
    fn selector_captures_normalized_utf16_offsets_and_context_limits() {
        let text = "Alpha   beta\n\ngamma 😀 delta epsilon zeta eta theta iota kappa lambda";
        let raw_start = text.find("delta").unwrap();
        let raw_end = raw_start + "delta".len();
        let selector = create_assistant_text_selector(text, raw_start, raw_end).unwrap();
        assert_eq!(selector.text, "delta");
        // "Alpha beta gamma 😀 " -> "😀" is 2 UTF-16 code units.
        let expected_before = "Alpha beta gamma 😀 ";
        let expected_start = utf16_len(expected_before) as u64;
        assert_eq!(selector.start, expected_start);
        assert_eq!(selector.end, expected_start + 5);
        assert_eq!(selector.prefix, expected_before);
        // Suffix is clamped to 32 UTF-16 units.
        assert_eq!(
            utf16_len(&selector.suffix),
            ASSISTANT_CITATION_CONTEXT_LENGTH
        );
        assert_eq!(selector.suffix, " epsilon zeta eta theta iota kap");
    }

    #[test]
    fn selector_does_not_split_surrogate_pairs_at_32_unit_context_boundary() {
        // Put a 4-byte emoji spanning the 32-unit boundary of prefix and suffix.
        let prefix_pad = "a".repeat(31);
        let suffix_pad = "b".repeat(31);
        let text = format!("head😀{prefix_pad}QUOTE{suffix_pad}😀tail");
        let raw_start = text.find("QUOTE").unwrap();
        let raw_end = raw_start + "QUOTE".len();
        let selector = create_assistant_text_selector(&text, raw_start, raw_end).unwrap();
        assert_eq!(selector.prefix, prefix_pad);
        assert_eq!(selector.suffix, suffix_pad);
        assert!(utf16_len(&selector.prefix) <= ASSISTANT_CITATION_CONTEXT_LENGTH);
        assert!(utf16_len(&selector.suffix) <= ASSISTANT_CITATION_CONTEXT_LENGTH);
    }

    #[test]
    fn selector_adjusts_start_inside_multi_whitespace_run() {
        let text = "hello   world";
        // Start at the second space inside the 3-space run.
        let selector = create_assistant_text_selector(text, 6, 13).unwrap();
        assert_eq!(selector.text, "  world");
        assert_eq!(selector.start, 5);
        assert_eq!(selector.end, 11);
    }

    #[test]
    fn citation_builder_enforces_max_text_length_and_truncates_label() {
        let exact = "é".repeat(ASSISTANT_CITATION_MAX_TEXT_LENGTH);
        let ok = build_assistant_citation("env-1", "1", "msg-1", &exact, 0, exact.len()).unwrap();
        assert_eq!(utf16_len(&ok.text), ASSISTANT_CITATION_MAX_TEXT_LENGTH);
        let lbl = assistant_citation_label(&ok);
        assert_eq!(
            lbl,
            format!("{}…", "é".repeat(ASSISTANT_CITATION_LABEL_MAX_CHARS))
        );

        let too_long = "é".repeat(ASSISTANT_CITATION_MAX_TEXT_LENGTH + 1);
        let err = build_assistant_citation("env-1", "1", "msg-1", &too_long, 0, too_long.len());
        assert_eq!(err, Err(CitationBuildError::TooLong));
    }

    #[test]
    fn opening_capture_trims_to_300_utf16_units_and_builds_response_record() {
        let long = format!("  {}  ", "x".repeat(400));
        let opening = capture_assistant_message_opening(&long).unwrap();
        assert_eq!(utf16_len(&opening.text), RESPONSE_OPENING_LENGTH);
        assert_eq!(opening.start, 0);
        assert_eq!(opening.end, RESPONSE_OPENING_LENGTH as u64);

        let code = format!("```\n  {}  \n```", "y".repeat(400));
        let code_opening = capture_assistant_message_opening(&code).unwrap();
        assert_eq!(utf16_len(&code_opening.text), RESPONSE_OPENING_LENGTH);
        assert_eq!(code_opening.start, 1);
        assert_eq!(code_opening.end, 1 + RESPONSE_OPENING_LENGTH as u64);

        let citation = AssistantCitation {
            version: 1,
            environment_id: "local".into(),
            thread_id: "1".into(),
            message_id: "m1".into(),
            text: opening.text,
            comment: None,
            start: opening.start,
            end: opening.end,
            prefix: opening.prefix,
            suffix: opening.suffix,
        };
        let record = response_context_record(citation, "m1", None);
        assert_eq!(record.context_id, "response_m1");
        assert_eq!(record.label, "x".repeat(ASSISTANT_CITATION_LABEL_MAX_CHARS));
    }

    #[test]
    fn validation_rejects_selection_spanning_two_messages_or_user_message() {
        let messages = vec![
            msg("m0", "You", "User prompt"),
            msg("m1", "Pi", "First assistant answer."),
            msg("m2", "Pi", "Second assistant answer."),
        ];

        // Single assistant message: accepted.
        let ok = validate_single_assistant_selection(
            7,
            &messages,
            &["conversation-7/chat-lane/turn-7-m1/message-7-1-body-0/p0"],
            "assistant",
        );
        assert_eq!(
            ok,
            Some(AssistantSelectionTarget {
                message_index: 1,
                message_id: "m1".into(),
                chunk_from: Some(0),
            })
        );

        // Selection spanning two assistant messages: rejected.
        assert_eq!(
            validate_single_assistant_selection(
                7,
                &messages,
                &[
                    "conversation-7/chat-lane/turn-7-m1/message-7-1-body-0/p0",
                    "conversation-7/chat-lane/turn-7-m2/message-7-2-body-0/p0",
                ],
                "First\nSecond",
            ),
            None
        );

        // Selection inside user message: rejected.
        assert_eq!(
            validate_single_assistant_selection(
                7,
                &messages,
                &["conversation-7/chat-lane/row-7-m0/message-7-0-text"],
                "User",
            ),
            None
        );

        // Empty or whitespace selection: rejected.
        assert_eq!(
            validate_single_assistant_selection(
                7,
                &messages,
                &["conversation-7/chat-lane/turn-7-m1/message-7-1-body-0/p0"],
                "   \n  ",
            ),
            None
        );
    }
}
