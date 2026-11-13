//! KSG largeText.worker.ts and shared/delimitedPreview.ts.
use std::{
    fs::File,
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_ROWS: usize = 5_000_000;
pub const MAX_COLUMNS: usize = 1024;
pub const MAX_RECORD: usize = 1024 * 1024;
pub const FIRST_SCREEN: usize = 16 * 1024;

pub struct Document {
    bytes: Vec<u8>,
    offsets: Vec<usize>,
    delimiter: Option<u8>,
    pub columns: usize,
    pub complete: bool,
}

pub fn delimiter(name: &str, mime: &str) -> Option<u8> {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match mime.as_str() {
        "text/csv" => Some(b','),
        "text/tab-separated-values" => Some(b'\t'),
        ""
        | "text/plain"
        | "application/octet-stream"
        | "binary/octet-stream"
        | "application/unknown"
        | "video/mp2t"
        | "video/vnd.dlna.mpeg-tts" => {
            let name = name.to_ascii_lowercase();
            if name.ends_with(".csv") {
                Some(b',')
            } else if name.ends_with(".tsv") {
                Some(b'\t')
            } else {
                None
            }
        }
        _ => None,
    }
}

impl Document {
    pub fn load(path: &Path, delimiter: Option<u8>, cancel: &AtomicBool) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let expected = file.metadata().map_err(|e| e.to_string())?.len();
        if expected > MAX_BYTES as u64 {
            return Err("Text previews support files up to 256 MB.".into());
        }
        let mut bytes = Vec::new();
        let mut chunk = [0; FIRST_SCREEN];
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Preview cancelled".into());
            }
            let read = file.read(&mut chunk).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            if bytes.len() + read > MAX_BYTES {
                return Err("Text previews support files up to 256 MB.".into());
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        if bytes.len() as u64 != expected {
            return Err("The file download was incomplete. Close it and try again.".into());
        }
        Self::index(bytes, delimiter, true)
    }

    pub fn index(bytes: Vec<u8>, delimiter: Option<u8>, complete: bool) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Text previews support files up to 256 MB.".into());
        }
        let mut document = Self {
            bytes,
            offsets: vec![0],
            delimiter,
            columns: 1,
            complete,
        };
        let mut state = 0;
        let mut field_start = true;
        let mut cr = false;
        let mut row_columns = 1;
        for (pos, &byte) in document.bytes.iter().enumerate() {
            if matches!((pos, byte), (0, 239) | (1, 187) | (2, 191)) {
                continue;
            }
            if byte == 0 {
                return Err("This file contains binary data.".into());
            }
            if pos - document.offsets.last().copied().unwrap() > MAX_RECORD {
                return Err("A line or record exceeds the 1 MB preview limit.".into());
            }
            if cr {
                cr = false;
                if byte == 10 {
                    *document.offsets.last_mut().unwrap() = pos + 1;
                    continue;
                }
            }
            if let Some(delimiter) = delimiter {
                if state == 1 {
                    if byte == b'"' {
                        state = 2;
                    }
                    continue;
                }
                if state == 2 {
                    if byte == b'"' {
                        state = 1;
                        continue;
                    }
                    state = 0;
                }
                if byte == b'"' && field_start {
                    state = 1;
                    field_start = false;
                    continue;
                }
                if byte == delimiter {
                    row_columns += 1;
                    field_start = true;
                    continue;
                }
            }
            if matches!(byte, b'\n' | b'\r') {
                if document.offsets.len() > MAX_ROWS {
                    return Err("Text previews support up to 5 million rows.".into());
                }
                document.offsets.push(pos + 1);
                document.columns = document.columns.max(row_columns).min(MAX_COLUMNS);
                row_columns = 1;
                field_start = true;
                cr = byte == b'\r';
            } else {
                field_start = false;
            }
        }
        if complete && document.bytes.len() > *document.offsets.last().unwrap() {
            if document.offsets.len() > MAX_ROWS {
                return Err("Text previews support up to 5 million rows.".into());
            }
            document.offsets.push(document.bytes.len());
            document.columns = document.columns.max(row_columns).min(MAX_COLUMNS);
        }
        Ok(document)
    }

    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }
    pub fn text(&self) -> Result<&str, String> {
        std::str::from_utf8(&self.bytes)
            .map(|s| s.strip_prefix('\u{feff}').unwrap_or(s))
            .map_err(|_| {
                "This file is not UTF-8 text. Open it in another app to view its contents.".into()
            })
    }
    pub fn row(&self, index: usize, table: bool) -> Result<Vec<String>, String> {
        let bytes = self
            .bytes
            .get(self.offsets[index]..self.offsets[index + 1])
            .ok_or("Missing preview row.")?;
        let text = std::str::from_utf8(bytes).map_err(
            |_| "This file is not UTF-8 text. Open it in another app to view its contents.",
        )?;
        let text = if index % 32 == 0 {
            text.strip_prefix('\u{feff}').unwrap_or(text)
        } else {
            text
        };
        if !table || self.delimiter.is_none() {
            return Ok(vec![text.trim_end_matches(['\r', '\n']).into()]);
        }
        let delimiter = char::from(self.delimiter.unwrap());
        let mut result = Vec::new();
        let mut cell = String::new();
        let mut quoted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '"' {
                if quoted && chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                    continue;
                }
                if quoted || cell.is_empty() {
                    quoted = !quoted;
                    continue;
                }
            }
            if !quoted && (c == delimiter || matches!(c, '\r' | '\n')) {
                if result.len() < MAX_COLUMNS {
                    result.push(std::mem::take(&mut cell));
                } else {
                    cell.clear();
                }
                if c != delimiter {
                    return Ok(result);
                }
            } else {
                cell.push(c);
            }
        }
        if result.len() < MAX_COLUMNS {
            result.push(cell);
        }
        Ok(result)
    }
}

pub fn decode_small(bytes: &[u8], truncated: bool) -> Result<String, String> {
    if bytes.contains(&0) {
        return Err("This file contains binary data and cannot be shown as text.".into());
    }
    let bytes = match std::str::from_utf8(bytes) {
        Ok(text) => return Ok(text.strip_prefix('\u{feff}').unwrap_or(text).into()),
        Err(error) if truncated && error.error_len().is_none() => &bytes[..error.valid_up_to()],
        Err(_) => {
            return Err(
                "This file is not UTF-8 text. Open it in another app to view its contents.".into(),
            );
        }
    };
    Ok(std::str::from_utf8(bytes)
        .unwrap()
        .strip_prefix('\u{feff}')
        .unwrap_or(std::str::from_utf8(bytes).unwrap())
        .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ksg_text_index_csv_quoting_utf8_and_record_bounds() {
        let doc = Document::index(
            b"\xef\xbb\xbfname,value\r\n\"multi\nline\",\"a\"\"b\"\r\nlast,2".to_vec(),
            Some(b','),
            true,
        )
        .unwrap();
        assert_eq!(doc.len(), 3);
        assert_eq!(doc.columns, 2);
        assert_eq!(doc.row(0, true).unwrap(), ["name", "value"]);
        assert_eq!(doc.row(1, true).unwrap(), ["multi\nline", "a\"b"]);
        assert_eq!(doc.row(2, true).unwrap(), ["last", "2"]);
        assert!(Document::index(b"binary\0".to_vec(), None, true).is_err());
        assert!(Document::index(vec![b'a'; MAX_RECORD + 2], None, true).is_err());
        assert!(decode_small(b"\xff", false).is_err());
        assert_eq!(decode_small(b"a\xe7\x8c", true).unwrap(), "a");
        assert!(decode_small(b"a\xe7\x8c", false).is_err());
    }
}
