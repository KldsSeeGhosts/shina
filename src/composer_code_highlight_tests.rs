//! Tests for `crate::composer_code_highlight`, the port of KSG Code's
//! `composerCodeBlockHighlight.ts`.

use super::*;
use crate::composer_rich;
use rune::CodeAttributes;
use std::cell::Cell;

fn edit_with(md: &str) -> TextEdit {
    let mut edit = composer_rich::draft("");
    composer_rich::paste(&mut edit, md, &|_| None);
    edit
}

fn code_block(edit: &TextEdit, nth: usize) -> LogicalBlock {
    edit.logical_blocks()
        .filter(|b| b.kind == BlockKind::Code)
        .nth(nth)
        .expect("a code block")
}

fn now() -> Instant {
    Instant::now()
}

/// Composer spans for a Rust block equal the transcript highlighter's output for
/// the same code, offset to the block's position in the draft.
#[test]
fn rust_block_spans_match_the_transcript_highlighter_at_the_blocks_offset() {
    let edit = edit_with("before\n\n```rust\nfn main() {\n    let a = 1;\n}\n```\n\nafter");
    let mut h = ComposerCodeHighlight::default();
    let theme = SyntaxTheme::neutral(false);
    let spans = h.spans(&edit, false, &theme, now());
    let block = code_block(&edit, 0);
    let code = &edit.text[block.range.clone()];
    let want = theme.spans(
        &rune_kit::syntax::highlight(Some("rust"), code),
        block.range.start,
    );
    assert!(!want.is_empty());
    assert_eq!(spans, want);
}

/// KSG leaves unknown languages and plain fences without decorations.
#[test]
fn unknown_and_empty_languages_draw_plain() {
    for info in ["", "text", "cobol"] {
        let edit = edit_with(&format!("```{info}\nlet a = 1;\n```"));
        let mut h = ComposerCodeHighlight::default();
        let spans = h.spans(&edit, false, &SyntaxTheme::neutral(false), now());
        assert!(spans.is_empty(), "info {info:?} should draw plain");
    }
}

/// A fence over KSG's 20,000-character cap draws plain; one at the cap colours.
#[test]
fn blocks_past_twenty_thousand_characters_stay_plain() {
    let edit = edit_with(&format!("```rust\n//{}\n```", "x".repeat(19_999)));
    let theme = SyntaxTheme::neutral(false);
    assert!(
        ComposerCodeHighlight::default()
            .spans(&edit, false, &theme, now())
            .is_empty()
    );

    let edit = edit_with(&format!("```rust\n//{}\n```", "x".repeat(19_997)));
    assert!(
        !ComposerCodeHighlight::default()
            .spans(&edit, false, &theme, now())
            .is_empty()
    );
}

/// Editing one block, or prose outside every block, must not retokenize the others.
#[test]
fn an_edit_in_one_block_does_not_retokenize_the_other() {
    let mut edit = edit_with("```rust\nlet a = 1;\n```\n\ntext\n\n```python\nb = 1\n```\n\nend");
    let mut h = ComposerCodeHighlight::default();
    let theme = SyntaxTheme::neutral(false);
    let runs = Cell::new(0);
    let counting = |lang: Option<&str>, code: &str| {
        runs.set(runs.get() + 1);
        rune_kit::syntax::highlight(lang, code)
    };
    h.spans_with(&edit, false, &theme, now(), &counting);
    assert_eq!(runs.get(), 2);

    // Type inside the Rust block: only it misses its signature.
    let at = code_block(&edit, 0).range.start;
    edit.replace_range(at..at + 3, "let c");
    h.spans_with(
        &edit,
        false,
        &theme,
        now() + Duration::from_millis(5),
        &counting,
    );
    assert_eq!(runs.get(), 3, "only the edited block retokenizes");

    // A prose edit retokenizes neither block.
    let end = edit.text.len();
    edit.replace_range(end..end, "!");
    h.spans_with(
        &edit,
        false,
        &theme,
        now() + Duration::from_millis(10),
        &counting,
    );
    assert_eq!(runs.get(), 3, "a prose edit retokenizes neither block");
}

/// No colour span may leave its block's content range.
#[test]
fn spans_stay_inside_the_blocks_content() {
    let edit = edit_with("```rust\nlet a = 1;\n```\n\ntext\n\n```python\nb = 1\n```");
    let mut h = ComposerCodeHighlight::default();
    let spans = h.spans(&edit, false, &SyntaxTheme::neutral(false), now());
    assert!(!spans.is_empty());
    let bounds: Vec<Range<usize>> = edit
        .logical_blocks()
        .filter(|b| b.kind == BlockKind::Code)
        .map(|b| b.range)
        .collect();
    for sp in &spans {
        let (s, e) = (sp.start as usize, sp.end as usize);
        assert!(
            bounds.iter().any(|r| s >= r.start && e <= r.end),
            "span {s}..{e} escaped every block in {bounds:?}"
        );
    }
}

/// A long fence keeps its mapped colours right after an edit and refreshes once
/// KSG's 300 ms pause has passed, without a further edit.
#[test]
fn a_long_block_keeps_mapped_spans_until_the_pause_passes() {
    let code = format!("fn f() {{ let x = \"{}\"; }}", "y".repeat(2_500));
    let mut edit = edit_with(&format!("```rust\n{code}\n```"));
    let mut h = ComposerCodeHighlight::default();
    let theme = SyntaxTheme::neutral(false);
    let runs = Cell::new(0);
    let counting = |lang: Option<&str>, code: &str| {
        runs.set(runs.get() + 1);
        rune_kit::syntax::highlight(lang, code)
    };
    let t0 = now();
    let first = h.spans_with(&edit, false, &theme, t0, &counting);
    assert!(!first.is_empty());
    assert_eq!(runs.get(), 1, "the first frame tokenizes even a long block");

    // Type inside the block: the colours stay, mapped through the edit.
    let at = code_block(&edit, 0).range.start + 10;
    edit.replace_range(at..at, "z");
    let t1 = t0 + Duration::from_millis(50);
    let mapped = h.spans_with(&edit, false, &theme, t1, &counting);
    assert_eq!(runs.get(), 1, "typing in a long fence defers the refresh");
    assert_eq!(mapped.len(), first.len(), "the mapped colours stay");
    assert!(
        mapped.iter().zip(&first).all(|(m, f)| m.start >= f.start),
        "an insertion cannot pull a span earlier"
    );
    assert!(h.deferred_refresh_due(t1).is_some());

    // Once the pause passes, the next frame tokenizes again.
    let t2 = t1 + Duration::from_millis(300);
    assert_eq!(h.deferred_refresh_due(t2), Some(Duration::ZERO));
    let fresh = h.spans_with(&edit, false, &theme, t2, &counting);
    assert_eq!(runs.get(), 2, "the pause ending refreshes the block");
    assert!(!fresh.is_empty());
    assert!(h.deferred_refresh_due(t2).is_none());
}

/// Short fences retokenize on every edit; only long ones defer.
#[test]
fn a_short_block_refreshes_on_every_edit() {
    let mut edit = edit_with("```rust\nlet a = 1;\n```");
    let mut h = ComposerCodeHighlight::default();
    let theme = SyntaxTheme::neutral(false);
    let runs = Cell::new(0);
    let counting = |lang: Option<&str>, code: &str| {
        runs.set(runs.get() + 1);
        rune_kit::syntax::highlight(lang, code)
    };
    h.spans_with(&edit, false, &theme, now(), &counting);
    let at = code_block(&edit, 0).range.end;
    edit.replace_range(at..at, "\nlet b = 2;");
    h.spans_with(
        &edit,
        false,
        &theme,
        now() + Duration::from_millis(5),
        &counting,
    );
    assert_eq!(runs.get(), 2);
    assert!(h.deferred_refresh_due(now()).is_none());
}

/// The language picker's `update_logical_block` changes attributes without text:
/// the fingerprint still notices, and the block recolours on the next frame.
#[test]
fn picking_another_language_recolours_the_block() {
    let mut edit = edit_with("```rust\nlet a = 1;\n```");
    let mut h = ComposerCodeHighlight::default();
    let theme = SyntaxTheme::neutral(false);
    let rust = h.spans(&edit, false, &theme, now());
    assert!(!rust.is_empty());

    let block = code_block(&edit, 0);
    let code = block.attributes.code().unwrap();
    let info = crate::composer_code_languages::with_info_string_language(code.info(), "python");
    let attrs = block.attributes.clone().with_code(
        CodeAttributes::new(code.fence(), &info).with_close(code.close().map(str::to_owned)),
    );
    edit.update_logical_block(block.id, block.kind, attrs);

    let spans = h.spans(&edit, false, &theme, now() + Duration::from_millis(5));
    let code = &edit.text[block.range.clone()];
    let want = theme.spans(
        &rune_kit::syntax::highlight(Some("python"), code),
        block.range.start,
    );
    assert_eq!(spans, want);
}

/// A theme switch invalidates every signature, so colours repaint at once.
#[test]
fn switching_themes_repaints_every_block() {
    let edit = edit_with("```rust\nlet a = 1;\n```");
    let mut h = ComposerCodeHighlight::default();
    let light = h.spans(&edit, false, &SyntaxTheme::neutral(false), now());
    let dark = h.spans(&edit, true, &SyntaxTheme::neutral(true), now());
    assert_eq!(light.len(), dark.len());
    assert_ne!(
        light.iter().map(|s| s.color).collect::<Vec<_>>(),
        dark.iter().map(|s| s.color).collect::<Vec<_>>(),
    );
}
