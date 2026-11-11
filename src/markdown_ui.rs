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
        .radius(8.0)
        .clip_rounded(8.0)
        .border(0.5, p.line)
        .fill(p.code_bg)
        .child(
            row()
                .h(30.0)
                .pl(12.0)
                .pr(6.0)
                .gap(6.0)
                .center_y()
                .child(label(
                    language,
                    TextStyle::new(11.0, 400.0).family(rune::Family::SystemMono),
                    p.muted,
                ))
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
    let style = MarkdownStyle::neutral(dark)
        .text(p.ink)
        .muted(p.muted)
        .body_font(if user {
            body_style(wide)
        } else {
            prose_style(wide)
        })
        .inline_code_bg(if dark {
            Color::WHITE.alpha(0.07)
        } else {
            Color::BLACK.alpha(0.055)
        })
        .selection(p.teal_bg)
        // The header frame owns the visible border. Rune's inner card has none.
        .code_block(p.code_bg, Color::TRANSPARENT)
        .diff(p.teal.alpha(0.12), p.danger.alpha(0.10))
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
        let diff = Markdown::new(&source[range.clone()])
            .streaming(streaming)
            .selectable(true)
            .copy_source(!user)
            .scroll_wide(true)
            .render(&style);
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

#[cfg(test)]
mod tests {
    use super::*;
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
