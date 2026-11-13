//! Ported from KSG Code's `composer-markdown-source.ts` (and the inline source
//! helpers of `composer-rich-text-doc.ts`): editable Markdown source at the caret
//! for the four inline marks (`bold`, `italic`, `strike`, `code`).

use std::ops::Range;

use rune::{
    AppMark, Color, EditMap, EditorSelection, Mark, MarkRange, TextEdit, TextStyle,
    TransactionOptions, text::Span,
};

use crate::composer_rich;

pub const MARKDOWN_SOURCE_MARK: &str = "markdown-source";

const STYLE_MARKS: [Mark; 4] = [Mark::Bold, Mark::Italic, Mark::Strike, Mark::Code];
const ATOM_TEXT: &str = "\u{FFFC}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledMarker {
    pub at: usize,
    pub closing: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledRange {
    pub from: usize,
    pub to: usize,
    pub escaped_opener: bool,
    pub markers: Vec<StyledMarker>,
}

/// KSG's `MarkdownSourceState`: tracks whether the editor is in a detached
/// post-history state, which source region is currently revealed, and Part C
/// backtick pairs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownSourceState {
    pub detached: bool,
    pub revealed: Vec<Range<usize>>,
    pub typed_backtick_pairs: Vec<Range<usize>>,
    pub prev_text: String,
    pub pending_typed: bool,
}

fn is_style_mark(mark: &Mark) -> bool {
    matches!(mark, Mark::Bold | Mark::Italic | Mark::Strike | Mark::Code)
}

fn is_delimiter_byte(b: u8) -> bool {
    matches!(b, b'*' | b'_' | b'~' | b'`')
}

fn same_marks(left: Option<&[Mark]>, right: Option<&[Mark]>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(l), Some(r)) => l.len() == r.len() && l.iter().all(|m| r.contains(m)),
        _ => false,
    }
}

pub fn has_shown_source(edit: &TextEdit) -> bool {
    edit.app_marks()
        .any(|(_, m)| m.id() == MARKDOWN_SOURCE_MARK)
}

pub fn source_regions(edit: &TextEdit) -> Vec<Range<usize>> {
    let mut regions: Vec<Range<usize>> = edit
        .app_marks()
        .filter(|(_, m)| m.id() == MARKDOWN_SOURCE_MARK)
        .map(|(r, _)| r)
        .collect();
    regions.sort_by_key(|r| (r.start, r.end));
    regions
}

pub fn touches(anchor: usize, focus: usize, range: &Range<usize>) -> bool {
    if anchor == focus {
        range.start <= focus && focus <= range.end
    } else {
        anchor.min(focus) < range.end && anchor.max(focus) > range.start
    }
}

pub fn touches_markdown_source(edit: &TextEdit, from: usize, to: usize) -> bool {
    let (a, b) = (from.min(to), from.max(to));
    source_regions(edit).iter().any(|r| touches(a, b, r))
}

/// Ported from KSG Code's `withAdjacentDelimiters`.
/// Takes in delimiters typed beside the source, like a closing `**` retyped after its text.
pub fn with_adjacent_delimiters(edit: &TextEdit, region: Range<usize>) -> Range<usize> {
    let line_idx = edit.line_of_byte(region.start);
    let line = edit.line_range(line_idx);
    let bytes = edit.text.as_bytes();
    let has_any_mark = |pos: usize| edit.marks().iter().any(|m| m.start <= pos && pos < m.end);

    let mut from = region.start;
    let mut count = 0usize;
    while from > line.start && is_delimiter_byte(bytes[from - 1]) && !has_any_mark(from - 1) {
        from -= 1;
        count += 1;
    }
    if count > 0 {
        let mut backslashes = 0usize;
        let mut bs = from;
        while bs > line.start && bytes[bs - 1] == b'\\' && !has_any_mark(bs - 1) {
            bs -= 1;
            backslashes += 1;
        }
        if backslashes % 2 == 1 {
            from += 1;
        }
    }

    let mut to = region.end;
    let line_end_idx = edit.line_of_byte(region.end);
    let line_end = edit.line_range(line_end_idx).end;
    while to < line_end && is_delimiter_byte(bytes[to]) && !has_any_mark(to) {
        to += 1;
    }

    from..to
}

struct MaskedSegment {
    masked: Range<usize>,
    source: Range<usize>,
}

fn mask_pills(edit: &TextEdit, region: Range<usize>) -> (String, Vec<MaskedSegment>) {
    let mut pills: Vec<Range<usize>> = edit
        .pills()
        .map(|(r, _)| r)
        .filter(|r| r.start >= region.start && r.end <= region.end)
        .collect();
    pills.sort_by_key(|r| (r.start, r.end));

    let mut masked = String::new();
    let mut segments = Vec::new();
    let mut cursor = region.start;
    for pill in pills {
        if pill.start < cursor {
            continue;
        }
        if cursor < pill.start {
            let m_start = masked.len();
            masked.push_str(&edit.text[cursor..pill.start]);
            segments.push(MaskedSegment {
                masked: m_start..masked.len(),
                source: cursor..pill.start,
            });
        }
        let m_start = masked.len();
        masked.push_str(ATOM_TEXT);
        segments.push(MaskedSegment {
            masked: m_start..masked.len(),
            source: pill.start..pill.end,
        });
        cursor = pill.end;
    }
    if cursor < region.end {
        let m_start = masked.len();
        masked.push_str(&edit.text[cursor..region.end]);
        segments.push(MaskedSegment {
            masked: m_start..masked.len(),
            source: cursor..region.end,
        });
    }
    (masked, segments)
}

fn map_masked_offset(segments: &[MaskedSegment], offset: usize, region: &Range<usize>) -> usize {
    if offset == 0 {
        return region.start;
    }
    for seg in segments {
        if offset <= seg.masked.start {
            return seg.source.start;
        }
        if offset <= seg.masked.end {
            if seg.masked.len() == seg.source.len() {
                return seg.source.start + (offset - seg.masked.start);
            }
            return if offset == seg.masked.end {
                seg.source.end
            } else {
                seg.source.start
            };
        }
    }
    region.end
}

#[derive(Clone, Debug)]
pub struct UnitRun {
    pub range: Range<usize>,
    pub marks: Option<Vec<Mark>>,
}

/// Parses `region` with `composer_inline::parse`, keeping pills opaque and returning
/// contiguous byte runs in `edit.text` with either `Some(marks)` or `None` (delimiter).
pub fn region_units(edit: &TextEdit, region: Range<usize>) -> Vec<UnitRun> {
    if region.start >= region.end {
        return Vec::new();
    }
    let (masked, segments) = mask_pills(edit, region.clone());
    let pieces = composer_rich::parse_inline(&masked);
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for piece in pieces {
        if piece.range.start > cursor {
            let s = map_masked_offset(&segments, cursor, &region);
            let e = map_masked_offset(&segments, piece.range.start, &region);
            if s < e {
                out.push(UnitRun {
                    range: s..e,
                    marks: None,
                });
            }
        }
        let s = map_masked_offset(&segments, piece.range.start, &region);
        let e = map_masked_offset(&segments, piece.range.end, &region);
        if s < e {
            out.push(UnitRun {
                range: s..e,
                marks: Some(piece.marks),
            });
        }
        cursor = piece.range.end;
    }
    if cursor < masked.len() {
        let s = map_masked_offset(&segments, cursor, &region);
        let e = region.end;
        if s < e {
            out.push(UnitRun {
                range: s..e,
                marks: None,
            });
        }
    }
    out
}

/// Ported from KSG Code's `collectStyledRanges`.
pub fn collect_styled_ranges(edit: &TextEdit) -> Vec<StyledRange> {
    let mut ranges = Vec::new();
    for para in edit.paragraphs() {
        if edit.block_kind(para.index) == rune::BlockKind::Code {
            continue;
        }
        if para.range.is_empty() {
            continue;
        }
        // Skip paragraphs/ranges with legacy links overlapping styled runs.
        let has_link_in = |r: &Range<usize>| {
            edit.marks()
                .iter()
                .any(|m| matches!(m.mark, Mark::Link(_)) && m.start < r.end && r.start < m.end)
        };
        let runs = composer_rich::inline_runs_for_source(edit, para.range.clone());
        let mut current: Option<StyledRange> = None;
        let mut open_length: isize = 0;
        let mut md = String::new();
        for run in runs {
            let md_start = md.len();
            if !run.open.is_empty() {
                let mut backslashes = 0usize;
                for b in md.as_bytes().iter().rev() {
                    if *b == b'\\' {
                        backslashes += 1;
                    } else {
                        break;
                    }
                }
                let r = current.get_or_insert_with(|| StyledRange {
                    from: run.range.start,
                    to: run.range.start,
                    escaped_opener: backslashes % 2 != 0,
                    markers: Vec::new(),
                });
                r.markers.push(StyledMarker {
                    at: run.range.start,
                    closing: false,
                    text: run.open.clone(),
                });
            }
            md.push_str(&run.open);
            md.push_str(&run.md_text);
            md.push_str(&run.close);
            if let Some(r) = &mut current {
                r.to = run.range.end;
                if !run.close.is_empty() {
                    r.markers.push(StyledMarker {
                        at: r.to,
                        closing: true,
                        text: run.close.clone(),
                    });
                }
                open_length += run.open.len() as isize - run.close.len() as isize;
                if open_length <= 0 {
                    let finished = current.take().unwrap();
                    open_length = 0;
                    if !has_link_in(&(finished.from..finished.to)) {
                        ranges.push(finished);
                    }
                }
            }
            let _ = md_start;
        }
    }
    ranges
}

/// Ported from KSG Code's `revealsFaithfully`: whether the Markdown shown for `range`
/// parses back to the exact same styles.
pub fn reveals_faithfully(edit: &TextEdit, range: &StyledRange) -> bool {
    if range.escaped_opener || range.from >= range.to {
        return false;
    }
    let adjacent = with_adjacent_delimiters(edit, range.from..range.to);
    if adjacent.start != range.from || adjacent.end != range.to {
        return false;
    }

    let mut markers = range.markers.clone();
    markers.sort_by_key(|m| (m.at, !m.closing));

    let mut text = String::new();
    // Each byte (or atom) in `text` maps to `Option<Vec<Mark>>` (`None` = delimiter).
    let mut expected: Vec<(Range<usize>, Option<Vec<Mark>>)> = Vec::new();
    let mut next_marker = 0usize;
    let add_markers =
        |pos: usize,
         next: &mut usize,
         text: &mut String,
         expected: &mut Vec<(Range<usize>, Option<Vec<Mark>>)>| {
            while *next < markers.len() && markers[*next].at <= pos {
                let s = text.len();
                text.push_str(&markers[*next].text);
                expected.push((s..text.len(), None));
                *next += 1;
            }
        };

    let mut pills: Vec<Range<usize>> = edit
        .pills()
        .map(|(r, _)| r)
        .filter(|r| r.start >= range.from && r.end <= range.to)
        .collect();
    pills.sort_by_key(|r| (r.start, r.end));

    let marks_at = |pos: usize| -> Vec<Mark> {
        STYLE_MARKS
            .iter()
            .filter(|mark| {
                edit.marks()
                    .iter()
                    .any(|m| &m.mark == *mark && m.start <= pos && pos < m.end)
            })
            .cloned()
            .collect()
    };

    let mut cursor = range.from;
    let mut pill_idx = 0usize;
    while cursor < range.to {
        if pill_idx < pills.len() && pills[pill_idx].start == cursor {
            let pill = &pills[pill_idx];
            add_markers(cursor, &mut next_marker, &mut text, &mut expected);
            let s = text.len();
            text.push_str(ATOM_TEXT);
            expected.push((s..text.len(), Some(marks_at(cursor))));
            cursor = pill.end;
            pill_idx += 1;
        } else {
            let ch = edit.text[cursor..].chars().next().unwrap();
            add_markers(cursor, &mut next_marker, &mut text, &mut expected);
            let s = text.len();
            text.push(ch);
            expected.push((s..text.len(), Some(marks_at(cursor))));
            cursor += ch.len_utf8();
        }
    }
    add_markers(range.to, &mut next_marker, &mut text, &mut expected);

    // Build byte-level expected marks in `text` (one entry per byte, since all inserted
    // markers are ASCII and each atom/char entry in `expected` is uniform across its bytes).
    let mut expected_bytes: Vec<Option<&[Mark]>> = vec![None; text.len()];
    for (r, m) in &expected {
        let slice = m.as_deref();
        for slot in &mut expected_bytes[r.clone()] {
            *slot = slice;
        }
    }

    let mut actual_bytes: Vec<Option<&[Mark]>> = vec![None; text.len()];
    let pieces = composer_rich::parse_inline(&text);
    for piece in &pieces {
        for slot in &mut actual_bytes[piece.range.clone()] {
            *slot = Some(piece.marks.as_slice());
        }
    }

    expected_bytes
        .iter()
        .zip(actual_bytes.iter())
        .all(|(exp, act)| same_marks(*exp, *act))
}

fn remove_style_marks_in_range(marks: &[MarkRange], range: Range<usize>) -> Vec<MarkRange> {
    let mut out = Vec::with_capacity(marks.len());
    for m in marks {
        if !is_style_mark(&m.mark) || m.end <= range.start || m.start >= range.end {
            out.push(m.clone());
        } else {
            if m.start < range.start {
                out.push(MarkRange::new(m.start..range.start, m.mark.clone()));
            }
            if m.end > range.end {
                out.push(MarkRange::new(range.end..m.end, m.mark.clone()));
            }
        }
    }
    out
}

fn collapse_region_in_tx(edit: &mut TextEdit, region: Range<usize>) {
    let units = region_units(edit, region.clone());
    // Remove MARKDOWN_SOURCE_MARK inside `region`, keep any other marks (like pills or
    // non-conflicting styles added inside revealed source), and add parsed marks.
    let mut next_marks: Vec<MarkRange> = edit
        .marks()
        .iter()
        .filter(|m| !matches!(&m.mark, Mark::App(a) if a.id() == MARKDOWN_SOURCE_MARK))
        .cloned()
        .collect();
    let mut deletions = Vec::new();
    for unit in &units {
        match &unit.marks {
            Some(marks) => {
                for mark in marks {
                    next_marks.push(MarkRange::new(unit.range.clone(), mark.clone()));
                }
            }
            None => deletions.push(unit.range.clone()),
        }
    }
    edit.set_marks(next_marks);
    for del in deletions.into_iter().rev() {
        edit.replace_range(del, "");
    }
}

fn extended_source_regions(edit: &TextEdit) -> Vec<Range<usize>> {
    let raw_regions = source_regions(edit);
    let mut regions: Vec<Range<usize>> = Vec::new();
    for region in raw_regions {
        let extended = with_adjacent_delimiters(edit, region);
        if let Some(prev) = regions.last_mut()
            && extended.start <= prev.end
        {
            prev.end = prev.end.max(extended.end);
        } else {
            regions.push(extended);
        }
    }
    regions
}

fn needs_collapse_step(edit: &TextEdit, keep: impl Fn(&Range<usize>) -> bool) -> bool {
    let regions = extended_source_regions(edit);
    if regions.is_empty() {
        return false;
    }
    regions.iter().any(|region| {
        if keep(region) {
            !edit.app_marks().any(|(r, m)| {
                m.id() == MARKDOWN_SOURCE_MARK && r.start <= region.start && region.end <= r.end
            })
        } else {
            true
        }
    })
}

/// Ported from KSG Code's `collapseMarkdownSource`.
/// Parses shown source the selection does not keep back into marks, extending kept
/// regions across adjacent delimiters.
fn collapse_markdown_source_in_tx(
    edit: &mut TextEdit,
    keep: impl Fn(&Range<usize>) -> bool,
) -> bool {
    let regions = extended_source_regions(edit);
    if regions.is_empty() {
        return false;
    }
    let mut changed = false;
    for region in regions.into_iter().rev() {
        if keep(&region) {
            let already_covered = edit.app_marks().any(|(r, m)| {
                m.id() == MARKDOWN_SOURCE_MARK && r.start <= region.start && region.end <= r.end
            });
            if !already_covered {
                let mut marks = edit.marks().to_vec();
                marks.push(MarkRange::new(
                    region,
                    Mark::App(AppMark::new(MARKDOWN_SOURCE_MARK)),
                ));
                edit.set_marks(marks);
                changed = true;
            }
        } else {
            collapse_region_in_tx(edit, region);
            changed = true;
        }
    }
    changed
}

/// Ported from KSG Code's `caretTakesMarksBefore`.
pub fn caret_takes_marks_before(edit: &TextEdit) -> bool {
    let Some(edge) = styled_edge(edit) else {
        return false;
    };
    same_mark_vec(&edge.current, &edge.before)
}

struct StyledEdge {
    before: Vec<Mark>,
    #[allow(dead_code)]
    after: Vec<Mark>,
    current: Vec<Mark>,
}

fn style_marks_vec(marks: &[Mark]) -> Vec<Mark> {
    STYLE_MARKS
        .iter()
        .filter(|m| marks.contains(m))
        .cloned()
        .collect()
}

fn same_mark_vec(left: &[Mark], right: &[Mark]) -> bool {
    left.len() == right.len() && left.iter().all(|m| right.contains(m))
}

fn styled_edge(edit: &TextEdit) -> Option<StyledEdge> {
    if edit.anchor != edit.focus || matches!(edit.selection(), EditorSelection::Block(_)) {
        return None;
    }
    let pos = edit.focus;
    let line_idx = edit.line_of_byte(pos);
    if edit.block_kind(line_idx) == rune::BlockKind::Code {
        return None;
    }
    let line = edit.line_range(line_idx);
    let before: Vec<Mark> = if pos <= line.start {
        Vec::new()
    } else {
        STYLE_MARKS
            .iter()
            .filter(|mark| {
                edit.marks()
                    .iter()
                    .any(|m| &m.mark == *mark && m.start < pos && pos <= m.end)
            })
            .cloned()
            .collect()
    };
    let after: Vec<Mark> = if pos >= line.end {
        Vec::new()
    } else {
        STYLE_MARKS
            .iter()
            .filter(|mark| {
                edit.marks()
                    .iter()
                    .any(|m| &m.mark == *mark && m.start <= pos && pos < m.end)
            })
            .cloned()
            .collect()
    };
    if same_mark_vec(&before, &after) {
        return None;
    }
    let current = style_marks_vec(&edit.active_marks());
    Some(StyledEdge {
        before,
        after,
        current,
    })
}

/// Ported from KSG Code's `revealStyledText`: shows one styled run that the
/// collapsed caret touches as Markdown source.
fn reveal_styled_text(
    edit: &mut TextEdit,
    typed: bool,
    takes_before: bool,
) -> Option<Range<usize>> {
    if edit.anchor != edit.focus || matches!(edit.selection(), EditorSelection::Block(_)) {
        return None;
    }
    let anchor = edit.anchor;
    let focus = edit.focus;
    let range = collect_styled_ranges(edit)
        .into_iter()
        .find(|r| touches(anchor, focus, &(r.from..r.to)) && reveals_faithfully(edit, r))?;

    let stripped = remove_style_marks_in_range(edit.marks(), range.from..range.to);
    edit.set_marks(stripped);

    let mut markers = range.markers.clone();
    // Inserting from the end keeps earlier positions valid; at a shared position,
    // the opening marker goes in first so the closing one ends up before it.
    markers.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.closing.cmp(&b.closing)));

    struct Insertion {
        at: usize,
        len: usize,
        closing: bool,
    }
    let mut insertions = Vec::with_capacity(markers.len());
    let mut added_len = 0usize;
    for marker in markers {
        let len = marker.text.len();
        edit.replace_range(marker.at..marker.at, &marker.text);
        insertions.push(Insertion {
            at: marker.at,
            len,
            closing: marker.closing,
        });
        added_len += len;
    }
    let revealed = range.from..range.to + added_len;
    let mut marks = edit.marks().to_vec();
    marks.push(MarkRange::new(
        revealed.clone(),
        Mark::App(AppMark::new(MARKDOWN_SOURCE_MARK)),
    ));
    edit.set_marks(marks);

    // A caret that arrives at a styled edge stays outside its markers. One that
    // just typed styled text stays inside them, so the next character continues it.
    let map_pos = |pos: usize| -> usize {
        let mut next = pos;
        for ins in &insertions {
            let inside = typed
                && if ins.closing {
                    takes_before
                } else {
                    !takes_before
                };
            if ins.at < pos || (ins.at == pos && ins.closing != inside) {
                next += ins.len;
            }
        }
        next
    };
    let mapped = map_pos(focus);
    edit.set_caret(mapped);
    Some(revealed)
}

/// Ported from KSG Code's `normalizeMarkdownSource`.
pub fn normalize_markdown_source(state: &mut MarkdownSourceState, edit: &mut TextEdit) {
    if edit.is_composing() || edit.is_secure() {
        return;
    }
    let typed = std::mem::take(&mut state.pending_typed);
    let takes_before = caret_takes_marks_before(edit);
    let (anchor, focus) = (edit.anchor, edit.focus);
    let block_sel = matches!(edit.selection(), EditorSelection::Block(_));

    // Step 1: collapse any shown region the selection no longer touches.
    let keep = |region: &Range<usize>| !block_sel && touches(anchor, focus, region);
    if needs_collapse_step(edit, keep) {
        let _ = edit.transact(TransactionOptions::Transient, |e| {
            collapse_markdown_source_in_tx(e, keep);
        });
    }

    // Step 2: if no source region remains and selection is a collapsed caret,
    // reveal the qualifying styled region it touches (at most one at a time).
    if !has_shown_source(edit)
        && edit.anchor == edit.focus
        && !matches!(edit.selection(), EditorSelection::Block(_))
        && collect_styled_ranges(edit).iter().any(|r| {
            touches(edit.anchor, edit.focus, &(r.from..r.to)) && reveals_faithfully(edit, r)
        })
    {
        let _ = edit.transact(TransactionOptions::Transient, |e| {
            reveal_styled_text(e, typed, takes_before);
        });
    }

    state.revealed = source_regions(edit);
    state.prev_text = edit.text.clone();
}

/// Callback entry point registered with `El::on_input_selection_change`.
pub fn on_selection_change(state: &mut MarkdownSourceState, edit: &mut TextEdit) {
    normalize_markdown_source(state, edit);
}

/// Collapses any shown Markdown source in a transient transaction (for Enter, paste, etc.).
pub fn collapse_shown_markdown_source(edit: &mut TextEdit) -> Option<EditMap> {
    if !has_shown_source(edit) || edit.is_composing() || edit.is_secure() {
        return None;
    }
    edit.transact(TransactionOptions::Transient, |e| {
        collapse_markdown_source_in_tx(e, |_| false);
    })
    .ok()
}

/// Returns a collapsed clone of `edit` (plus its collapse `EditMap` if any) without
/// touching `edit` or its history. Used for sending, draft saving, copying, and queued edits.
pub fn collapsed_copy(edit: &TextEdit) -> (TextEdit, Option<EditMap>) {
    let mut copy = edit.clone();
    if !has_shown_source(&copy) {
        return (copy, None);
    }
    // Clear composing state on the copy so `transact` is never rejected if a send/save
    // occurs during composition.
    copy.cancel_composition();
    let map = copy
        .transact(TransactionOptions::Transient, |e| {
            collapse_markdown_source_in_tx(e, |_| false);
        })
        .ok();
    (copy, map)
}

/// Strips any `MARKDOWN_SOURCE_MARK` from a restored draft so restored drafts start collapsed.
pub fn strip_shown_source(edit: &mut TextEdit) {
    if has_shown_source(edit) {
        let _ = collapse_shown_markdown_source(edit);
        if has_shown_source(edit) {
            let marks = edit
                .marks()
                .iter()
                .filter(|m| !matches!(&m.mark, Mark::App(a) if a.id() == MARKDOWN_SOURCE_MARK))
                .cloned()
                .collect();
            edit.set_marks(marks);
        }
    }
}

fn map_range_to_display(edit: &TextEdit, start: usize, end: usize) -> Vec<(usize, usize)> {
    if start >= end {
        return Vec::new();
    }
    let Some((preedit, _)) = edit.preedit() else {
        return vec![(start, end)];
    };
    let (a, b) = edit.range();
    let del_len = b - a;
    let ins_len = preedit.len();
    if end <= a {
        vec![(start, end)]
    } else if start >= b {
        vec![(start - del_len + ins_len, end - del_len + ins_len)]
    } else {
        let mut out = Vec::new();
        if start < a {
            out.push((start, a));
        }
        if end > b {
            out.push((a + ins_len, end - del_len + ins_len));
        }
        out
    }
}

/// Ported from KSG Code's `previewDecorations`: renders revealed delimiters in `faint`
/// base font/size, and keeps the content between them styled with its marks.
pub fn preview_decorations(
    edit: &TextEdit,
    base_style: TextStyle,
    faint: Color,
    code_fg: Color,
    code_bg: Color,
) -> (Vec<Span>, Vec<(usize, usize, Option<Color>)>) {
    let regions = source_regions(edit);
    if regions.is_empty() || edit.is_secure() {
        return (Vec::new(), Vec::new());
    }
    let bold_weight = (base_style.weight + 250.0).clamp(600.0, 900.0);
    let mut spans = Vec::new();
    let mut strikes = Vec::new();
    for region in regions {
        for unit in region_units(edit, region) {
            let (s, e) = (unit.range.start, unit.range.end);
            match unit.marks {
                None => {
                    spans.push(Span::new(s, e).color(faint));
                }
                Some(marks) => {
                    for mark in marks {
                        match mark {
                            Mark::Bold => spans.push(Span::new(s, e).weight(bold_weight)),
                            Mark::Italic => spans.push(Span::new(s, e).italic()),
                            Mark::Code => {
                                spans.push(Span::new(s, e).mono().color(code_fg).bg(code_bg, 2.0));
                            }
                            Mark::Strike => {
                                for (ds, de) in map_range_to_display(edit, s, e) {
                                    strikes.push((ds, de, None));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    (spans, strikes)
}

#[cfg(test)]
#[path = "composer_source_tests.rs"]
mod tests;
