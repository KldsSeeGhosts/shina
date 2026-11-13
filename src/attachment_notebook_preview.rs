//! Ported from KSG `files/extendedDocumentPreview.ts` `openNotebook` and the
//! `imageDimensions` readers behind `base64ImageFits`: strict UTF-8, notebook
//! format 4 only, saved outputs, and every parser bound. Nothing is executed.
use serde_json::Value;
use std::sync::Arc;

/// KSG's input bound, checked before decoding, as `openNotebook`'s 8 MB check.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_CELLS: usize = 500;
/// KSG bounds cell sources and text outputs in UTF-16 units, like `notebookText`.
const MAX_TEXT: usize = 256_000;
const MAX_OUTPUTS: usize = 100;
/// KSG bounds image base64 text in UTF-16 units too.
const MAX_IMAGE_BASE64: usize = 4 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_777_216;
/// `IMAGE_DIMENSIONS_HEADER_BYTES`: the readers see this many decoded bytes.
const HEADER_BYTES: usize = 256 * 1024;
const HEADER_BASE64: usize = 4 * HEADER_BYTES.div_ceil(3);

#[derive(Debug, PartialEq, Eq)]
pub struct Notebook {
    pub language: String,
    pub cells: Vec<NotebookCell>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKind {
    Markdown,
    Code,
    Raw,
}

impl CellKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Code => "code",
            Self::Raw => "raw",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct NotebookCell {
    pub kind: CellKind,
    pub source: String,
    pub execution_count: Option<u64>,
    pub outputs: Vec<NotebookOutput>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NotebookOutput {
    /// stderr and error outputs carry `danger`, like Jupyter's red output text.
    Text { text: String, danger: bool },
    /// Decoded image bytes and their declared pixel size, already bounded.
    Image {
        mime: &'static str,
        bytes: Arc<[u8]>,
        size: [u32; 2],
    },
    /// An output whose only rich form is HTML. Shina has no embedded browser.
    Html,
}

pub fn parse(bytes: &[u8]) -> Result<Notebook, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Notebook preview limited to files under 8 MB.".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        "This file is not UTF-8 text. Open it in another app to view its contents.".to_owned()
    })?;
    let value: Value =
        serde_json::from_str(text).map_err(|_| "This notebook is not valid JSON.".to_owned())?;
    let notebook = value.as_object();
    let cells = notebook
        .and_then(|n| n.get("cells"))
        .and_then(Value::as_array);
    if notebook
        .and_then(|n| n.get("nbformat"))
        .and_then(Value::as_f64)
        != Some(4.0)
        || cells.is_none_or(|cells| cells.len() > MAX_CELLS)
    {
        return Err("Preview supports version 4 notebooks with up to 500 cells.".into());
    }
    let metadata = notebook.and_then(|n| n.get("metadata"));
    let cells = cells.unwrap();
    let mut parsed = Vec::with_capacity(cells.len().min(MAX_CELLS));
    for value in cells {
        parsed.push(cell(value)?);
    }
    Ok(Notebook {
        language: language(metadata),
        cells: parsed,
    })
}

fn cell(value: &Value) -> Result<NotebookCell, String> {
    let cell = value.as_object();
    let outputs = cell
        .and_then(|c| c.get("outputs"))
        .and_then(Value::as_array);
    if outputs.is_some_and(|outputs| outputs.len() > MAX_OUTPUTS) {
        return Err("A notebook cell has too many outputs to preview.".into());
    }
    let kind = match cell
        .and_then(|c| c.get("cell_type"))
        .and_then(Value::as_str)
    {
        Some("markdown") => CellKind::Markdown,
        Some("code") => CellKind::Code,
        _ => CellKind::Raw,
    };
    let mut parsed = Vec::new();
    for value in outputs.into_iter().flatten() {
        if let Some(output) = output(value)? {
            parsed.push(output);
        }
    }
    Ok(NotebookCell {
        kind,
        source: text(cell.and_then(|c| c.get("source")), MAX_TEXT)?,
        execution_count: cell
            .and_then(|c| c.get("execution_count"))
            .and_then(Value::as_u64),
        outputs: parsed,
    })
}

fn output(value: &Value) -> Result<Option<NotebookOutput>, String> {
    let output = value.as_object();
    let data = output
        .and_then(|o| o.get("data"))
        .and_then(Value::as_object);
    let kind = output
        .and_then(|o| o.get("output_type"))
        .and_then(Value::as_str);
    let danger = kind == Some("error")
        || (kind == Some("stream")
            && output.and_then(|o| o.get("name")).and_then(Value::as_str) == Some("stderr"));
    if let Some(data) = data {
        // KSG picks the first usable image in this order, then HTML, then text.
        for mime in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            if let Some(encoded) = data.get(mime).filter(|v| !v.is_null()) {
                let mut base64 = text(Some(encoded), MAX_IMAGE_BASE64)?;
                base64.retain(|c| !c.is_whitespace());
                if let Some((bytes, size)) = image(&base64) {
                    return Ok(Some(NotebookOutput::Image { mime, bytes, size }));
                }
            }
        }
        let html = text(data.get("text/html"), MAX_TEXT)?;
        if !html.trim().is_empty() {
            // KSG mounts the HTML in a bounded frame. Shina has no embedded
            // browser, so it shows the text fallback KSG uses for HTML it
            // declines, or a notice when there is none.
            let plain = text(fallback(output, Some(data)), MAX_TEXT).unwrap_or_default();
            return Ok(Some(if plain.trim().is_empty() {
                NotebookOutput::Html
            } else {
                NotebookOutput::Text {
                    text: plain,
                    danger,
                }
            }));
        }
    }
    let value = fallback(output, data);
    let mut result = text(value, MAX_TEXT)?;
    if kind == Some("error") {
        result = strip_ansi(&result);
        if result.trim().is_empty() {
            let ename = output
                .and_then(|o| o.get("ename"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let evalue = output
                .and_then(|o| o.get("evalue"))
                .and_then(Value::as_str)
                .unwrap_or("");
            result = format!("{ename}: {evalue}");
        }
    }
    Ok((!result.trim().is_empty()).then_some(NotebookOutput::Text {
        text: result,
        danger,
    }))
}

/// KSG's text fallback for an output: `text`, then `data["text/plain"]`, then
/// `traceback`; null and undefined skip like JavaScript's `??`.
fn fallback<'a>(
    output: Option<&'a serde_json::Map<String, Value>>,
    data: Option<&'a serde_json::Map<String, Value>>,
) -> Option<&'a Value> {
    output
        .and_then(|o| o.get("text"))
        .filter(|v| !v.is_null())
        .or_else(|| {
            data.and_then(|d| d.get("text/plain"))
                .filter(|v| !v.is_null())
        })
        .or_else(|| {
            output
                .and_then(|o| o.get("traceback"))
                .filter(|v| !v.is_null())
        })
}

/// KSG `notebookText`: a string, or an array of strings joined; anything else
/// is empty. The bound counts UTF-16 units, like KSG's `string.length`.
fn text(value: Option<&Value>, limit: usize) -> Result<String, String> {
    let text = match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    };
    if text.encode_utf16().count() > limit {
        return Err("A notebook cell is too large to preview.".into());
    }
    Ok(text)
}

fn language(metadata: Option<&Value>) -> String {
    let name = metadata
        .and_then(Value::as_object)
        .and_then(|m| m.get("language_info"))
        .and_then(Value::as_object)
        .and_then(|l| l.get("name"));
    let name = match name {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => "text".to_owned(),
    };
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '-'))
        .take(40)
        .collect()
}

/// KSG `base64ImageFits` plus the decode the data URL would have deferred: the
/// charset and padding are valid, and the declared dimensions fit the bound.
fn image(base64: &str) -> Option<(Arc<[u8]>, [u32; 2])> {
    let bytes = base64.as_bytes();
    let body = bytes
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'+' || **b == b'/')
        .count();
    if bytes.len() - body > 2 || !bytes[body..].iter().all(|&b| b == b'=') {
        return None;
    }
    if bytes.len() % 4 == 1 || (bytes.len() - body > 0 && bytes.len() % 4 != 0) {
        return None;
    }
    let head = decode(&base64[..base64.len().min(HEADER_BASE64)])?;
    let size = dimensions(&head)?;
    if u64::from(size[0]) * u64::from(size[1]) > MAX_PIXELS {
        return None;
    }
    Some((decode(base64)?.into(), size))
}

/// Forgiving base64, like `atob`: padding may be absent, and a short tail of
/// two or three characters decodes to one or two bytes.
fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let end = bytes.len() - bytes.iter().rev().take_while(|&&b| b == b'=').count();
    let mut out = Vec::with_capacity(end / 4 * 3 + 3);
    for chunk in bytes[..end].chunks(4) {
        let acc = match *chunk {
            [a, b, c, d] => value(a)? << 18 | value(b)? << 12 | value(c)? << 6 | value(d)?,
            [a, b, c] => value(a)? << 12 | value(b)? << 6 | value(c)?,
            [a, b] => value(a)? << 6 | value(b)?,
            _ => return None,
        };
        match chunk.len() {
            4 => out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]),
            3 => out.extend_from_slice(&[(acc >> 10) as u8, (acc >> 2) as u8]),
            _ => out.push((acc >> 4) as u8),
        }
    }
    Some(out)
}

fn value(byte: u8) -> Option<u32> {
    let value = match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    };
    Some(u32::from(value))
}

/// KSG `readImageDimensions`: PNG, GIF, WebP or JPEG declared size, with EXIF
/// rotation swapping the JPEG axes. Zero or missing dimensions reject it.
fn dimensions(b: &[u8]) -> Option<[u32; 2]> {
    let size = png(b)
        .or_else(|| gif(b))
        .or_else(|| webp(b))
        .or_else(|| jpeg(b));
    size.filter(|size| size[0] > 0 && size[1] > 0)
}

fn be_u16(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(
        b.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn le_u16(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        b.get(at..at + 2)?.try_into().ok()?,
    )))
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn png(b: &[u8]) -> Option<[u32; 2]> {
    if b.len() < 24
        || b[0] != 0x89
        || b[1] != 0x50
        || b[2] != 0x4e
        || b[3] != 0x47
        || b[12] != 0x49
        || b[13] != 0x48
        || b[14] != 0x44
        || b[15] != 0x52
    {
        return None;
    }
    Some([be_u32(b, 16)?, be_u32(b, 20)?])
}

fn gif(b: &[u8]) -> Option<[u32; 2]> {
    if b.len() < 10 || b[0] != 0x47 || b[1] != 0x49 || b[2] != 0x46 || b[3] != 0x38 {
        return None;
    }
    Some([le_u16(b, 6)?, le_u16(b, 8)?])
}

fn webp(b: &[u8]) -> Option<[u32; 2]> {
    if b.len() < 16
        || b[0] != 0x52
        || b[1] != 0x49
        || b[2] != 0x46
        || b[3] != 0x46
        || b[8] != 0x57
        || b[9] != 0x45
        || b[10] != 0x42
        || b[11] != 0x50
    {
        return None;
    }
    match &b[12..16] {
        b"VP8 " => {
            if b.len() < 30 {
                return None;
            }
            Some([le_u16(b, 26)? & 0x3fff, le_u16(b, 28)? & 0x3fff])
        }
        b"VP8L" => {
            if b.len() < 25 {
                return None;
            }
            let packed = le_u32(b, 21)?;
            Some([(packed & 0x3fff) + 1, ((packed >> 14) & 0x3fff) + 1])
        }
        b"VP8X" => {
            if b.len() < 30 {
                return None;
            }
            Some([
                (u32::from(b[24]) | u32::from(b[25]) << 8 | u32::from(b[26]) << 16) + 1,
                (u32::from(b[27]) | u32::from(b[28]) << 8 | u32::from(b[29]) << 16) + 1,
            ])
        }
        _ => None,
    }
}

fn jpeg(b: &[u8]) -> Option<[u32; 2]> {
    if b.len() < 4 || b[0] != 0xff || b[1] != 0xd8 {
        return None;
    }
    let mut offset = 2;
    let mut rotated = false;
    while offset + 9 <= b.len() {
        if b[offset] != 0xff {
            return None;
        }
        let marker = b[offset + 1];
        if marker == 0xff {
            offset += 1;
            continue;
        }
        if (0xc0..=0xcf).contains(&marker) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc {
            let height = be_u16(b, offset + 5)?;
            let width = be_u16(b, offset + 7)?;
            return Some(if rotated {
                [height, width]
            } else {
                [width, height]
            });
        }
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            offset += 2;
            continue;
        }
        let length = be_u16(b, offset + 2)? as usize;
        if marker == 0xe1 && !rotated {
            rotated = exif_swaps_axes(b, offset + 4, offset + 2 + length);
        }
        offset += 2 + length;
    }
    None
}

fn exif_swaps_axes(b: &[u8], start: usize, end: usize) -> bool {
    let end = end.min(b.len());
    if end < start + 14 || b.get(start..start + 4) != Some(b"Exif") {
        return false;
    }
    let tiff = start + 6;
    let little = b.get(tiff) == Some(&0x49) && b.get(tiff + 1) == Some(&0x49);
    if !little && !(b.get(tiff) == Some(&0x4d) && b.get(tiff + 1) == Some(&0x4d)) {
        return false;
    }
    let (u16_at, u32_at): (
        fn(&[u8], usize) -> Option<u32>,
        fn(&[u8], usize) -> Option<u32>,
    ) = if little {
        (le_u16, le_u32)
    } else {
        (be_u16, be_u32)
    };
    let Some(ifd) = u32_at(b, tiff + 4).map(|offset| tiff + offset as usize) else {
        return false;
    };
    if ifd + 2 > end {
        return false;
    }
    let Some(entries) = u16_at(b, ifd) else {
        return false;
    };
    for i in 0..entries as usize {
        let entry = ifd + 2 + i * 12;
        if entry + 12 > end {
            return false;
        }
        if u16_at(b, entry) == Some(0x0112) {
            let orientation = u16_at(b, entry + 8).unwrap_or(0);
            return (5..=8).contains(&orientation);
        }
    }
    false
}

/// Jupyter tracebacks carry ANSI colour and weight escapes. Strip CSI and OSC
/// sequences; the text stays readable without a terminal.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                let mut escape = false;
                for c in chars.by_ref() {
                    if c == '\u{7}' || (escape && c == '\\') {
                        break;
                    }
                    escape = c == '\u{1b}';
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(4, 2, image::Rgba([60, 80, 100, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    /// A PNG header only: enough for the dimension reader, without a fixture.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![
            0x89, 0x50, 0x4e, 0x47, 0, 0, 0, 0, 0, 0, 0, 0, 0x49, 0x48, 0x44, 0x52,
        ];
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    fn open(document: Value) -> Result<Notebook, String> {
        parse(document.to_string().as_bytes())
    }

    #[test]
    fn parses_v4_cells_sources_counts_and_saved_outputs() {
        let png = png();
        let image = crate::attachments::encode_base64(&png);
        let notebook = open(json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {"language_info": {"name": "python3.12"}},
            "cells": [
                {"cell_type": "markdown", "metadata": {},
                 "source": ["# Title\n", "a [link](https://example.com)"]},
                {"cell_type": "code", "execution_count": 7, "metadata": {},
                 "source": "print(1)",
                 "outputs": [
                    {"output_type": "stream", "name": "stdout", "text": ["hello\n", "world\n"]},
                    {"output_type": "stream", "name": "stderr", "text": ["warn\n"]},
                    {"output_type": "execute_result", "execution_count": 7,
                     "data": {"text/plain": "42"}, "metadata": {}},
                    {"output_type": "display_data",
                     "data": {"image/png": image}, "metadata": {}},
                    {"output_type": "display_data",
                     "data": {"text/html": "<b>Results</b>", "text/plain": "Results"},
                     "metadata": {}},
                    {"output_type": "display_data",
                     "data": {"text/html": "<i>only html</i>"}, "metadata": {}},
                    {"output_type": "error", "ename": "ValueError", "evalue": "bad",
                     "traceback": ["\u{1b}[0;31mValueError\u{1b}[0m: bad"]},
                    {"output_type": "error", "ename": "KeyError", "evalue": "'k'",
                     "traceback": []},
                    {"output_type": "stream", "name": "stdout", "text": ["   "]},
                 ]},
                {"cell_type": "raw", "metadata": {}, "source": ["raw", " text"]},
                {"cell_type": "code", "source": ["print(2)"]},
            ],
        }))
        .unwrap();
        assert_eq!(notebook.language, "python312");
        assert_eq!(notebook.cells.len(), 4);
        assert_eq!(notebook.cells[0].kind, CellKind::Markdown);
        assert_eq!(
            notebook.cells[0].source,
            "# Title\na [link](https://example.com)"
        );
        assert_eq!(notebook.cells[2].kind, CellKind::Raw);
        assert_eq!(notebook.cells[2].source, "raw text");
        assert_eq!(notebook.cells[2].execution_count, None);
        assert_eq!(notebook.cells[3].execution_count, None);
        let code = &notebook.cells[1];
        assert_eq!(code.kind, CellKind::Code);
        assert_eq!(code.execution_count, Some(7));
        assert_eq!(
            code.outputs[..3],
            [
                NotebookOutput::Text {
                    text: "hello\nworld\n".into(),
                    danger: false
                },
                NotebookOutput::Text {
                    text: "warn\n".into(),
                    danger: true
                },
                NotebookOutput::Text {
                    text: "42".into(),
                    danger: false
                },
            ]
        );
        match &code.outputs[3] {
            NotebookOutput::Image { mime, bytes, size } => {
                assert_eq!(*mime, "image/png");
                assert_eq!(*size, [4, 2]);
                assert_eq!(bytes.as_ref(), png.as_slice());
            }
            other => panic!("expected an image output, got {other:?}"),
        }
        assert_eq!(
            code.outputs[4..],
            [
                NotebookOutput::Text {
                    text: "Results".into(),
                    danger: false
                },
                NotebookOutput::Html,
                NotebookOutput::Text {
                    text: "ValueError: bad".into(),
                    danger: true
                },
                NotebookOutput::Text {
                    text: "KeyError: 'k'".into(),
                    danger: true
                },
            ]
        );
    }

    #[test]
    fn rejects_notebooks_outside_version_4_and_past_500_cells() {
        let version = "Preview supports version 4 notebooks with up to 500 cells.";
        assert_eq!(
            open(json!({"nbformat": 3, "cells": []})).unwrap_err(),
            version
        );
        assert_eq!(
            open(json!({"nbformat": 4, "cells": {"a": 1}})).unwrap_err(),
            version
        );
        assert_eq!(open(json!([1, 2, 3])).unwrap_err(), version);
        assert_eq!(
            open(json!({"nbformat": 4, "cells": vec![json!({}); 501]})).unwrap_err(),
            version
        );
        assert_eq!(
            parse(b"not json").unwrap_err(),
            "This notebook is not valid JSON."
        );
        assert_eq!(
            parse(&[0xff, 0xfe]).unwrap_err(),
            "This file is not UTF-8 text. Open it in another app to view its contents."
        );
    }

    #[test]
    fn enforces_input_source_output_and_count_bounds_with_ksg_messages() {
        assert_eq!(
            parse(&vec![b' '; MAX_BYTES + 1]).unwrap_err(),
            "Notebook preview limited to files under 8 MB."
        );
        assert_eq!(
            open(json!({"nbformat": 4, "cells": [
                {"cell_type": "markdown", "source": ["x".repeat(MAX_TEXT + 1)]},
            ]}))
            .unwrap_err(),
            "A notebook cell is too large to preview."
        );
        assert_eq!(
            open(json!({"nbformat": 4, "cells": [
                {"cell_type": "code", "source": "",
                 "outputs": [{"output_type": "stream", "name": "stdout",
                              "text": ["x".repeat(MAX_TEXT + 1)]}]},
            ]}))
            .unwrap_err(),
            "A notebook cell is too large to preview."
        );
        assert_eq!(
            open(json!({"nbformat": 4, "cells": [
                {"cell_type": "code", "source": "",
                 "outputs": [{"output_type": "display_data",
                              "data": {"image/png": "a".repeat(MAX_IMAGE_BASE64 + 1)}}]},
            ]}))
            .unwrap_err(),
            "A notebook cell is too large to preview."
        );
        assert_eq!(
            open(json!({"nbformat": 4, "cells": [
                {"cell_type": "code", "source": "",
                 "outputs": vec![json!({"output_type": "stream", "text": ["x"]}); 101]},
            ]}))
            .unwrap_err(),
            "A notebook cell has too many outputs to preview."
        );
    }

    #[test]
    fn bounds_output_images_by_pixels_and_base64_shape() {
        // Over the pixel bound the output falls through to the text/plain form.
        let over = crate::attachments::encode_base64(&png_header(4097, 4097));
        let notebook = open(json!({"nbformat": 4, "cells": [
            {"cell_type": "code", "source": "",
             "outputs": [{"output_type": "display_data",
                          "data": {"image/png": over, "text/plain": "alt"}}]},
        ]}))
        .unwrap();
        assert_eq!(
            notebook.cells[0].outputs,
            [NotebookOutput::Text {
                text: "alt".into(),
                danger: false
            }]
        );
        // Exactly at the bound it stays an image.
        let edge = crate::attachments::encode_base64(&png_header(4096, 4096));
        let notebook = open(json!({"nbformat": 4, "cells": [
            {"cell_type": "code", "source": "",
             "outputs": [{"output_type": "display_data", "data": {"image/png": edge}}]},
        ]}))
        .unwrap();
        assert_eq!(
            notebook.cells[0].outputs,
            [NotebookOutput::Image {
                mime: "image/png",
                bytes: png_header(4096, 4096).into(),
                size: [4096, 4096],
            }]
        );
        // Malformed base64 is not an image; with no alternative it drops out.
        let notebook = open(json!({"nbformat": 4, "cells": [
            {"cell_type": "code", "source": "",
             "outputs": [{"output_type": "display_data", "data": {"image/png": "A"}}]},
        ]}))
        .unwrap();
        assert!(notebook.cells[0].outputs.is_empty());
    }
}
