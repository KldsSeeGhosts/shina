//! ClaudeAdapterV2.ts attachment limits. Paths remain available when native media won't fit.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_BYTES: u64 = 28 * 1024 * 1024;
pub const MAX_PAGES: u64 = 100;

#[derive(Clone, Copy, Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Usage {
    pub bytes: u64,
    pub pages: u64,
}

impl Usage {
    pub fn add(&mut self, other: Self) {
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.pages = self.pages.saturating_add(other.pages);
    }
    pub fn subtract(&mut self, other: Self) {
        self.bytes = self.bytes.saturating_sub(other.bytes);
        self.pages = self.pages.saturating_sub(other.pages);
    }
}

#[derive(Default)]
pub struct Budget {
    pub committed: Usage,
    pub pending: std::collections::HashMap<String, Usage>,
    pub compacting: Option<Usage>,
}

fn path(session: &str) -> Option<PathBuf> {
    use sha2::Digest;
    crate::store::directory().map(|directory| {
        directory.join("claude-attachment-budgets").join(format!(
            "{:x}.json",
            sha2::Sha256::digest(session.as_bytes())
        ))
    })
}

impl Budget {
    pub fn load(session: &str) -> Self {
        Self {
            committed: path(session)
                .and_then(|path| std::fs::read(path).ok())
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default(),
            ..Default::default()
        }
    }
    pub fn remaining(&self) -> Usage {
        let mut remaining = Usage {
            bytes: MAX_BYTES,
            pages: MAX_PAGES,
        };
        remaining.subtract(self.committed);
        for pending in self.pending.values() {
            remaining.subtract(*pending);
        }
        remaining
    }
    pub fn accept(&mut self, id: &str, session: &str) {
        if let Some(usage) = self.pending.remove(id) {
            self.committed.add(usage);
            self.save(session);
        }
    }
    pub fn save(&self, session: &str) {
        if let Some(path) = path(session)
            && std::fs::create_dir_all(path.parent().unwrap()).is_ok()
            && let Ok(bytes) = serde_json::to_vec(&self.committed)
        {
            let _ = crate::store::write_atomic(&path, &bytes);
        }
    }
    pub fn compacted(&mut self, partial: bool, session: &str) {
        if !partial {
            self.committed
                .subtract(self.compacting.unwrap_or(self.committed));
        }
        self.compacting = None;
        self.save(session);
    }
}

/// Only accept explicit, uncompressed PDF page trees whose /Count agrees with /Page.
pub fn pdf_pages(bytes: &[u8]) -> Option<u64> {
    if bytes.len() > 20 * 1024 * 1024
        || !bytes.starts_with(b"%PDF-")
        || !bytes[bytes.len().saturating_sub(1024)..]
            .windows(5)
            .any(|w| w == b"%%EOF")
    {
        return None;
    }
    fn delimiter(b: u8) -> bool {
        b <= 32 || b"()<>[]{}/%".contains(&b)
    }
    let (mut pages, mut count) = (0, 0);
    let (mut catalog, mut tree) = (false, false);
    let mut previous: &[u8] = b"";
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'%' {
            while i < bytes.len() && !b"\r\n".contains(&bytes[i]) {
                i += 1;
            }
            continue;
        }
        if byte <= 32 {
            i += 1;
            continue;
        }
        if previous == b"/Type" && byte != b'/' {
            return None;
        }
        if previous == b"/Kids" {
            if byte != b'[' {
                return None;
            }
            previous = b"";
        }
        if byte == b'(' {
            let mut depth = 1;
            i += 1;
            while i < bytes.len() && depth > 0 {
                match bytes[i] {
                    b'\\' => i += 1,
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
            if depth != 0 {
                return None;
            }
            previous = b"";
            continue;
        }
        if byte == b'<' {
            if bytes.get(i + 1) == Some(&b'<') {
                i += 2;
            } else {
                i += 1;
                while i < bytes.len() && bytes[i] != b'>' {
                    i += 1;
                }
                if i == bytes.len() {
                    return None;
                }
                i += 1;
            }
            previous = b"";
            continue;
        }
        if delimiter(byte) && byte != b'/' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < bytes.len() && !delimiter(bytes[i]) {
            i += 1;
        }
        if i - start > 64 {
            return None;
        }
        let token = &bytes[start..i];
        if token.contains(&b'#')
            || [b"/ObjStm".as_slice(), b"/XRef", b"/Encrypt", b"/Prev"].contains(&token)
        {
            return None;
        }
        if token == b"stream" {
            i += bytes[i..].windows(9).position(|w| w == b"endstream")? + 9;
            previous = b"";
            continue;
        }
        if previous == b"/Type" {
            match token {
                b"/Page" => pages += 1,
                b"/Catalog" => catalog = true,
                b"/Pages" => tree = true,
                _ => {}
            }
        }
        if previous == b"/Count" {
            if token.is_empty()
                || token.len() > 9
                || token[0] == b'0'
                || !token.iter().all(u8::is_ascii_digit)
            {
                return None;
            }
            let mut next = i;
            while next < bytes.len() {
                if bytes[next] <= 32 {
                    next += 1;
                } else if bytes[next] == b'%' {
                    while next < bytes.len() && !b"\r\n".contains(&bytes[next]) {
                        next += 1;
                    }
                } else {
                    break;
                }
            }
            if next < bytes.len() && !delimiter(bytes[next]) {
                return None;
            }
            count = count.max(std::str::from_utf8(token).ok()?.parse().ok()?);
        }
        previous = token;
    }
    (catalog && tree && pages > 0 && count == pages).then_some(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_budget_reserves_concurrent_inputs_and_compaction_preserves_later_ones() {
        let mut budget = Budget::default();
        budget.pending.insert(
            "first".into(),
            Usage {
                bytes: MAX_BYTES,
                pages: MAX_PAGES,
            },
        );
        assert_eq!(budget.remaining(), Usage::default());
        budget.accept("first", "budget-fixture");
        budget.compacting = Some(budget.committed);
        budget.pending.insert(
            "second".into(),
            Usage {
                bytes: 200,
                pages: 1,
            },
        );
        budget.accept("second", "budget-fixture");
        budget.compacted(false, "budget-fixture");
        assert_eq!(
            budget.committed,
            Usage {
                bytes: 200,
                pages: 1
            }
        );
        assert_eq!(Budget::load("budget-fixture").committed, budget.committed);
        budget.compacting = Some(budget.committed);
        budget.compacted(true, "budget-fixture");
        assert_eq!(
            budget.committed,
            Usage {
                bytes: 200,
                pages: 1
            }
        );
    }
    #[test]
    fn pdf_pages_only_trusts_an_explicit_agreeing_uncompressed_tree() {
        let pdf = b"%PDF-1.4\n1 0 obj << /Type /Catalog >>\n2 0 obj << /Type /Pages /Count 1 /Kids [3 0 R] >>\n3 0 obj << /Type /Page >>\n%%EOF";
        assert_eq!(pdf_pages(pdf), Some(1));
        assert_eq!(pdf_pages(&pdf[..pdf.len() - 5]), None);
        assert_eq!(pdf_pages(&[pdf.as_slice(), b"\n/Encrypt"].concat()), None);
        assert_eq!(pdf_pages(&[pdf.as_slice(), b"\n/ObjStm"].concat()), None);
    }
}
