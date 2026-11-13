//! Per-line syntax highlighting for source code attachment previews.

use super::Palette;
use super::markdown_ui::syntax_theme;

const MAX_HIGHLIGHTED_LINE_BYTES: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorSpan {
    pub start: usize,
    pub end: usize,
    pub color: rune::Color,
}

pub fn language_for(name: &str) -> Option<&'static str> {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    let lang = match ext.as_str() {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "py" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "java" => "java",
        "kt" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "php" => "php",
        "cs" => "csharp",
        "sh" | "bash" | "zsh" => "bash",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "html" | "htm" => "html",
        "css" => "css",
        "sql" => "sql",
        "md" => "markdown",
        "lua" => "lua",
        "zig" => "zig",
        _ => return None,
    };
    rune_kit::syntax::known(lang).then_some(lang)
}

pub fn line_spans(language: &str, line: &str, dark: bool, p: Palette) -> Vec<ColorSpan> {
    if line.is_empty()
        || line.len() > MAX_HIGHLIGHTED_LINE_BYTES
        || !rune_kit::syntax::known(language)
    {
        return Vec::new();
    }
    let theme = syntax_theme(dark, p);
    let highlights = rune_kit::syntax::highlight(Some(language), line);
    let mut out = Vec::with_capacity(highlights.len());
    let mut cursor = 0;
    for (range, paint) in highlights.iter() {
        let start = range.start;
        let end = range.end;
        if start >= end
            || end > line.len()
            || start < cursor
            || !line.is_char_boundary(start)
            || !line.is_char_boundary(end)
        {
            continue;
        }
        cursor = end;
        out.push(ColorSpan {
            start,
            end,
            color: theme.paint(*paint),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_for_maps_extensions_and_rejects_unknowns() {
        assert_eq!(language_for("main.rs"), Some("rust"));
        assert_eq!(language_for("App.TSX"), Some("tsx"));
        assert_eq!(language_for("noext"), None);
        assert_eq!(language_for("data.csv"), None);
        assert_eq!(language_for("x.unknownext"), None);
    }

    #[test]
    fn line_spans_highlights_rust_line_with_valid_sorted_spans() {
        let line = "fn main() {}";
        let spans = line_spans("rust", line, false, super::super::palette(false));
        assert!(!spans.is_empty());
        let mut prev_end = 0;
        for span in &spans {
            assert!(span.start < span.end);
            assert!(span.end <= line.len());
            assert!(span.start >= prev_end);
            assert!(line.is_char_boundary(span.start));
            assert!(line.is_char_boundary(span.end));
            prev_end = span.end;
        }
    }

    #[test]
    fn line_spans_respects_multibyte_char_boundaries() {
        let line = "let s = \"héllo\";";
        let spans = line_spans("rust", line, true, super::super::palette(true));
        assert!(!spans.is_empty());
        for span in &spans {
            assert!(span.start < span.end);
            assert!(span.end <= line.len());
            assert!(line.is_char_boundary(span.start));
            assert!(line.is_char_boundary(span.end));
        }
    }

    #[test]
    fn line_spans_skips_lines_over_byte_limit() {
        let line = "a".repeat(3_000);
        assert!(line_spans("rust", &line, false, super::super::palette(false)).is_empty());
    }
}
