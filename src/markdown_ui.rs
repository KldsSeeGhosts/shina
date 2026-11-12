//! KSG Code's ChatMarkdown lineBreaks and MarkdownCodeBlock headers in Rune.
use super::*;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::borrow::Cow;

const OPTIONS: Options = Options::ENABLE_STRIKETHROUGH
    .union(Options::ENABLE_TABLES)
    .union(Options::ENABLE_TASKLISTS);

/// Convert only Markdown soft breaks. The parser leaves fenced/indented code,
/// tables and existing hard breaks untouched. Stored messages never change.
fn user_source(source: &str) -> Cow<'_, str> {
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
) -> El {
    Markdown::new(source)
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
        })
        .render(style)
}

/// Ported from KSG Code's ChatMarkdown and chat/MarkdownCodeBlock.
pub(super) fn render(
    source: &str,
    p: Palette,
    dark: bool,
    wide: bool,
    user: bool,
    streaming: bool,
    identity: &str,
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
        return ordinary(&source, &style, streaming, user, identity.into(), p);
    }
    let diff_style = style.clone().code_block_padding(12.0, 6.0);
    let mut content = col().w_full().min_w(0.0).gap(12.0).selection_group(true);
    let mut from = 0;
    for (range, language, code) in diffs {
        if !source[from..range.start].trim().is_empty() {
            content = content.child(ordinary(
                &source[from..range.start],
                &style,
                streaming,
                user,
                format!("{identity}-part-{from}"),
                p,
            ));
        }
        // Tinted rows run edge to edge, so the block keeps a 6 px pad above the first row and
        // below the last, in place of the 12 px that left an empty line under the header.
        let diff = Markdown::new(&source[range.clone()])
            .streaming(streaming)
            .selectable(true)
            .copy_source(!user)
            .scroll_wide(true)
            .render(&diff_style);
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
        content = content.child(ordinary(
            &source[from..],
            &style,
            streaming,
            user,
            format!("{identity}-part-{from}"),
            p,
        ));
    }
    content
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
