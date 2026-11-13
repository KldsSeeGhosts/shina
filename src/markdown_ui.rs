//! KSG Code's ChatMarkdown lineBreaks and MarkdownCodeBlock headers in Rune.
use super::*;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use rune_kit::syntax::{Syntax, SyntaxTheme};
use std::borrow::Cow;

const OPTIONS: Options = Options::ENABLE_STRIKETHROUGH
    .union(Options::ENABLE_TABLES)
    .union(Options::ENABLE_TASKLISTS);

/// Convert only Markdown soft breaks. The parser leaves fenced/indented code,
/// tables and existing hard breaks untouched. Stored messages never change.
pub(super) fn user_source(source: &str) -> Cow<'_, str> {
    let breaks: Vec<_> = Parser::new_ext(source, OPTIONS)
        .into_offset_iter()
        .filter_map(|(event, range)| matches!(event, Event::SoftBreak).then_some(range.start))
        .collect();
    if breaks.is_empty() {
        return Cow::Borrowed(source);
    }
    let mut rendered = String::with_capacity(source.len() + breaks.len() * 2);
    let mut from = 0;
    for at in breaks {
        rendered.push_str(&source[from..at]);
        rendered.push_str("  ");
        from = at;
    }
    rendered.push_str(&source[from..]);
    Cow::Owned(rendered)
}

fn code_frame(code: El, language: &str, source: String, identity: &str, p: Palette) -> El {
    col()
        .key(format!("code-{identity}"))
        .min_w(0.0)
        .radius(6.0)
        .clip_rounded(6.0)
        .border(1.0, p.line)
        .fill(p.code_bg)
        .child(
            row()
                .h(30.0)
                .pl(12.0)
                .pr(6.0)
                .gap(6.0)
                .center_y()
                .child(pixel_label(language, p.muted))
                .child(spacer())
                .child(
                    icon_button(
                        &format!("copy-code-{identity}"),
                        "Copy code",
                        "copy",
                        p,
                        |_| {},
                    )
                    .help("Copy code")
                    .on_press_tx(move |_: &mut State, tx| tx.copy(source.clone())),
                ),
        )
        .child(rule(p))
        .child(code)
}

fn ordinary(
    source: &str,
    style: &MarkdownStyle,
    streaming: bool,
    user: bool,
    identity: String,
    p: Palette,
    find: Option<(&str, Option<usize>)>,
) -> El {
    let mut md = Markdown::new(source)
        .streaming(streaming)
        .selectable(true)
        .copy_source(!user)
        .scroll_wide(true)
        .code_frame(move |code, frame| {
            code_frame(
                code,
                frame.language.unwrap_or("text"),
                frame.source.to_owned(),
                &format!("{identity}-{}", frame.index),
                p,
            )
        });
    if let Some((query, active_occ)) = find {
        let hits = crate::thread_find::find_markdown_occurrences(source, query);
        if !hits.is_empty() {
            md = md.highlights(hits.into_iter().enumerate().map(|(i, ranges)| {
                let is_active = Some(i) == active_occ;
                let mut hl = rune_kit::markdown::Highlight::new().current_match(is_active);
                if is_active {
                    hl = hl.key("thread-find-active");
                }
                (ranges, hl)
            }));
        }
    }
    md.render(style)
}

/// Ported from KSG Code's ChatMarkdown and chat/MarkdownCodeBlock.
pub(super) fn render_with_find(
    source: &str,
    p: Palette,
    dark: bool,
    wide: bool,
    user: bool,
    streaming: bool,
    identity: &str,
    find: Option<(&str, Option<usize>)>,
) -> El {
    let source = if user {
        user_source(source)
    } else {
        Cow::Borrowed(source)
    };
    // The dark coral wash is too faint on the code fill to read as a removed line, but 28%
    // was too loud. 17% of coral over `code_bg`, mixed in sRGB as a paint program would, is
    // about #2f2126. Rune's `mix` works in linear light and would give a much brighter red.
    let removed = if dark {
        mix_srgb(p.code_bg, p.coral, 0.17)
    } else {
        p.coral_bg
    };
    let style = MarkdownStyle::neutral(dark)
        .syntax_theme(syntax_theme(dark, p))
        .text(p.ink)
        .muted(p.muted)
        .body_font(if user {
            body_style(wide)
        } else {
            prose_style(wide)
        })
        // Rune's inline code chip has a fixed corner radius; only its fill is ours.
        .inline_code_bg(p.code_bg)
        .selection(p.coral_bg)
        .highlight_bg(p.brass.alpha(0.32))
        .highlight_current_bg(p.coral.alpha(0.42))
        .highlight_current_ring(Some((p.coral, 1.5)))
        // The header frame owns the visible border. Rune's inner card has none.
        .code_block(p.code_bg, Color::TRANSPARENT)
        .diff(p.sea_bg, removed)
        .code_block_font(
            TextStyle::new(12.0, 400.0)
                .family(rune::Family::SystemMono)
                .lh(18.0),
        );

    // Rune bypasses code_frame for diff/patch fences. Wrap those fences here,
    // retaining Rune's own diff renderer rather than changing its highlighting.
    let mut diffs = Vec::new();
    let mut active = None;
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(&source, OPTIONS).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let language = info.split_whitespace().next().unwrap_or("");
                if depth == 0 && matches!(language, "diff" | "patch") {
                    active = Some((range.start, language.to_owned(), String::new()));
                }
                depth += 1;
            }
            Event::Start(_) => depth += 1,
            Event::Text(text) => {
                if let Some((_, _, code)) = &mut active {
                    code.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((start, language, code)) = active.take() {
                    diffs.push((
                        start..range.end,
                        language,
                        code.trim_end_matches(['\r', '\n']).to_owned(),
                    ));
                }
                depth = depth.saturating_sub(1);
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if diffs.is_empty() {
        return ordinary(&source, &style, streaming, user, identity.into(), p, find);
    }
    let diff_style = style.clone().code_block_padding(12.0, 6.0);
    let mut content = col().w_full().min_w(0.0).gap(12.0).selection_group(true);
    let mut from = 0;
    let mut occ_base = 0usize;
    let chunk_find = |slice: &str, occ_base: &mut usize| -> Option<(&str, Option<usize>)> {
        let (query, active_occ) = find?;
        let count = crate::thread_find::find_markdown_occurrences(slice, query).len();
        let local_active = active_occ
            .and_then(|o| (o >= *occ_base && o < *occ_base + count).then_some(o - *occ_base));
        *occ_base += count;
        Some((query, local_active))
    };
    for (range, language, code) in diffs {
        if !source[from..range.start].trim().is_empty() {
            let slice = &source[from..range.start];
            let part_find = chunk_find(slice, &mut occ_base);
            content = content.child(ordinary(
                slice,
                &style,
                streaming,
                user,
                format!("{identity}-part-{from}"),
                p,
                part_find,
            ));
        }
        // Tinted rows run edge to edge, so the block keeps a 6 px pad above the first row and
        // below the last, in place of the 12 px that left an empty line under the header.
        let diff_slice = &source[range.clone()];
        let diff_find = chunk_find(diff_slice, &mut occ_base);
        let mut diff_md = Markdown::new(diff_slice)
            .streaming(streaming)
            .selectable(true)
            .copy_source(!user)
            .scroll_wide(true);
        if let Some((query, active_occ)) = diff_find {
            let hits = crate::thread_find::find_markdown_occurrences(diff_slice, query);
            if !hits.is_empty() {
                diff_md = diff_md.highlights(hits.into_iter().enumerate().map(|(i, ranges)| {
                    let is_active = Some(i) == active_occ;
                    let mut hl = rune_kit::markdown::Highlight::new().current_match(is_active);
                    if is_active {
                        hl = hl.key("thread-find-active");
                    }
                    (ranges, hl)
                }));
            }
        }
        let diff = diff_md.render(&diff_style);
        content = content.child(code_frame(
            diff,
            &language,
            code,
            &format!("{identity}-diff-{}", range.start),
            p,
        ));
        from = range.end;
    }
    if !source[from..].trim().is_empty() {
        let slice = &source[from..];
        let part_find = chunk_find(slice, &mut occ_base);
        content = content.child(ordinary(
            slice,
            &style,
            streaming,
            user,
            format!("{identity}-part-{from}"),
            p,
            part_find,
        ));
    }
    content
}

const SYNTAX_KINDS: [Syntax; 17] = [
    Syntax::Keyword,
    Syntax::String,
    Syntax::Number,
    Syntax::Comment,
    Syntax::Type,
    Syntax::Function,
    Syntax::Property,
    Syntax::Punctuation,
    Syntax::Inserted,
    Syntax::Deleted,
    Syntax::Parameter,
    Syntax::Heading,
    Syntax::Link,
    Syntax::Operator,
    Syntax::Regex,
    Syntax::Attribute,
    Syntax::Variable,
];

/// The contrast ratio of two opaque colours.
fn contrast(a: Color, b: Color) -> f32 {
    let (a, b) = (a.luminance(), b.luminance());
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Rune's token colours, with the light ones darkened until each reads at 4.5:1 on the code
/// fill. Identifiers, punctuation, operators and comments go no lighter than `#595f73`.
/// The dark theme already passes and stays as Rune draws it.
pub(super) fn syntax_theme(dark: bool, p: Palette) -> SyntaxTheme {
    let mut theme = SyntaxTheme::neutral(dark);
    if dark {
        return theme;
    }
    let quiet = contrast(rgb_bytes([0x59, 0x5f, 0x73]), p.code_bg);
    for kind in SYNTAX_KINDS {
        let want = match kind {
            Syntax::Punctuation
            | Syntax::Operator
            | Syntax::Variable
            | Syntax::Parameter
            | Syntax::Comment => quiet.max(4.5),
            _ => 4.5,
        };
        let mut colour = theme.color(kind);
        let mut step = 0.0;
        while contrast(colour, p.code_bg) < want && step < 1.0 {
            step += 0.02;
            colour = mix_srgb(theme.color(kind), Color::BLACK, step);
        }
        theme = theme.with(kind, colour);
    }
    theme
}

/// `t` of the way from `a` to `b`, mixed in sRGB-encoded space.
fn mix_srgb(a: Color, b: Color, t: f32) -> Color {
    use rune::color::{srgb_decode, srgb_encode};
    let channel = |a: f32, b: f32| {
        let (a, b) = (srgb_encode(a), srgb_encode(b));
        srgb_decode(a + (b - a) * t)
    };
    Color {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
        a: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_code_colours_read_at_4_5_to_1_on_the_code_fill() {
        let p = palette(false);
        let theme = syntax_theme(false, p);
        let quiet = contrast(rgb_bytes([0x59, 0x5f, 0x73]), p.code_bg);
        for kind in SYNTAX_KINDS {
            let ratio = contrast(theme.color(kind), p.code_bg);
            assert!(ratio >= 4.5, "{kind:?} is {ratio:.2}:1");
            if matches!(
                kind,
                Syntax::Punctuation
                    | Syntax::Operator
                    | Syntax::Variable
                    | Syntax::Parameter
                    | Syntax::Comment
            ) {
                assert!(ratio >= quiet - 0.01, "{kind:?} is {ratio:.2}:1");
            }
        }
    }

    /// Text that carries meaning: on-coral text on a coral key at rest and pressed (80% coral
    /// over the card, panel or sky), and every status word on each surface a status sits on,
    /// including the selected sidebar row.
    #[test]
    fn status_words_and_coral_keys_read_at_4_5_to_1_in_both_themes() {
        for dark in [false, true] {
            let p = palette(dark);
            let selected_row = p.bg.mix(p.row, p.row.a);
            let mut pairs = vec![("on coral, rest", p.on_coral, p.coral)];
            for (name, under) in [("card", p.card), ("panel", p.panel), ("bg", p.bg)] {
                pairs.push((name, p.on_coral, under.mix(p.coral, 0.8)));
            }
            for status in [
                Status::Running,
                Status::Waiting,
                Status::Failed,
                Status::Completed,
                Status::Ready,
                Status::Paused,
            ] {
                let ink = status_ink(status, p);
                for under in [p.bg, p.panel, p.card, selected_row] {
                    pairs.push(("status", ink, under));
                }
            }
            for (name, ink, under) in pairs {
                let ratio = contrast(ink, under);
                assert!(ratio >= 4.5, "{name} is {ratio:.2}:1, dark={dark}");
            }
        }
    }

    #[test]
    fn the_dark_removed_line_is_a_quiet_red_not_a_loud_one() {
        let p = palette(true);
        let hex = mix_srgb(p.code_bg, p.coral, 0.17).hex();
        let channel = |at: usize| i32::from_str_radix(&hex[at..at + 2], 16).unwrap();
        for (at, want) in [(1, 0x2f), (3, 0x21), (5, 0x26)] {
            assert!((channel(at) - want).abs() <= 6, "{hex}");
        }
    }
    #[test]
    fn user_breaks_preserve_code_and_markdown_structure() {
        let source = "step one\nstep two\n\n```rust\nlet first = 1;\nlet second = 2;\n```\n\n- alpha\n- beta\n\nlast\r\nline";
        let rendered = user_source(source);
        assert!(rendered.starts_with("step one  \nstep two"));
        assert!(rendered.contains("```rust\nlet first = 1;\nlet second = 2;\n```"));
        assert!(rendered.contains("- alpha\n- beta"));
        assert!(rendered.ends_with("last  \r\nline"));
        assert!(matches!(user_source("already  \nbroken"), Cow::Borrowed(_)));
    }
}
