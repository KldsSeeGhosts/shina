//! KSG binaryPreview.worker.ts archive branch and client-runtime/zip/boundedZip.
use std::{
    collections::BTreeMap,
    io::Read,
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 50 * 1024 * 1024;
const ENTRY_MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_ENTRIES: usize = 2048;
const MAX_RATIO: usize = 200;
const EXPANSION_ERROR: &str =
    "The expanded archive exceeds the preview size or compression ratio limit.";
const TIMEOUT: &str = "This file took too long to preview. Close it and try again.";

pub struct Entry {
    pub path: String,
    pub bytes: Vec<u8>,
    pub directory: bool,
    pub modified: Option<String>,
}
pub struct Archive {
    pub entries: Vec<Entry>,
}

fn date(seconds: f64) -> Option<String> {
    if !seconds.is_finite() || seconds.abs() > 8_640_000_000_000.0 {
        return None;
    }
    chrono::DateTime::from_timestamp_millis((seconds * 1000.0).trunc() as i64)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn read_bounded(mut reader: impl Read, limit: usize, deadline: Instant) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 16 * 1024];
    loop {
        if Instant::now() > deadline {
            return Err(TIMEOUT.into());
        }
        let count = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(count) > limit {
            return Err(EXPANSION_ERROR.into());
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

impl Archive {
    pub fn parse(name: &str, bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Preview limited to files under 50 MB.".into());
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".zip")
            || lower.ends_with(".jar")
            || bytes.starts_with(b"PK\x03\x04")
            || bytes.starts_with(b"PK\x05\x06")
        {
            return zip(bytes, deadline);
        }
        if lower.ends_with(".gz") || lower.ends_with(".tgz") || bytes.first() == Some(&0x1f) {
            let expanded = read_bounded(
                flate2::read::GzDecoder::new(bytes),
                MAX_BYTES.min(bytes.len().saturating_mul(MAX_RATIO)),
                deadline,
            )?;
            let path = if lower.ends_with(".gz") {
                &name[..name.len() - 3]
            } else {
                name
            };
            if lower.ends_with(".tar.gz")
                || lower.ends_with(".tgz")
                || path.to_ascii_lowercase().ends_with(".tar")
            {
                return tar(&expanded, deadline);
            }
            return Ok(Self {
                entries: vec![Entry {
                    path: if path.is_empty() { "contents" } else { path }.into(),
                    bytes: expanded,
                    directory: false,
                    modified: None,
                }],
            });
        }
        tar(bytes, deadline)
    }

    pub fn entry(&self, path: &str) -> Result<&[u8], String> {
        let entry = self
            .entries
            .iter()
            .rev()
            .find(|e| e.path == path)
            .ok_or("This archive entry is unavailable.")?;
        if entry.bytes.len() > ENTRY_MAX_BYTES {
            return Err("Entry previews are limited to 10 MB.".into());
        }
        Ok(&entry.bytes)
    }
}

fn tar(bytes: &[u8], deadline: Instant) -> Result<Archive, String> {
    let mut entries = Vec::new();
    let mut offset = 0usize;
    let mut long_path = None;
    let mut global = BTreeMap::<String, String>::new();
    let mut local = BTreeMap::<String, String>::new();
    while offset.saturating_add(512) <= bytes.len() {
        if Instant::now() > deadline {
            return Err(TIMEOUT.into());
        }
        let header = &bytes[offset..offset + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        if entries.len() >= MAX_ENTRIES {
            return Err("This archive contains more than 2,048 entries.".into());
        }
        let text = |start: usize, len: usize| {
            String::from_utf8_lossy(&header[start..start + len])
                .split('\0')
                .next()
                .unwrap_or("")
                .to_owned()
        };
        let number = |start: usize, len: usize| -> Result<usize, String> {
            let text = text(start, len);
            let text = text.trim();
            if !text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
                return Err("This TAR uses an unsupported numeric format.".into());
            }
            usize::from_str_radix(if text.is_empty() { "0" } else { text }, 8)
                .map_err(|_| "This TAR uses an unsupported numeric format.".into())
        };
        let checksum: usize = header
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    b as usize
                }
            })
            .sum();
        if checksum != number(148, 8)? {
            return Err("The TAR header is damaged.".into());
        }
        let kind = header[156];
        let metadata = matches!(kind, b'L' | b'x' | b'g');
        let attributes: BTreeMap<_, _> = if metadata {
            BTreeMap::new()
        } else {
            global
                .iter()
                .chain(local.iter())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };
        let size = if let Some(value) = attributes.get("size").filter(|v| !v.is_empty()) {
            if !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err("An archive entry is too large or damaged.".into());
            }
            value
                .parse()
                .map_err(|_| "An archive entry is too large or damaged.")?
        } else {
            number(124, 12)?
        };
        let prefix = if text(257, 6) == "ustar" && text(263, 2) == "00" {
            text(345, 155)
        } else {
            String::new()
        };
        let path = attributes
            .get("path")
            .filter(|v| !v.is_empty())
            .cloned()
            .or_else(|| long_path.clone())
            .unwrap_or_else(|| {
                [prefix, text(0, 100)]
                    .into_iter()
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>()
                    .join("/")
            });
        if !metadata {
            long_path = None;
            local.clear();
        }
        if path.encode_utf16().count() > 1024
            || size > MAX_BYTES
            || size > bytes.len().saturating_sub(offset + 512)
        {
            return Err("An archive entry is too large or damaged.".into());
        }
        let content = &bytes[offset + 512..offset + 512 + size];
        if kind == b'L' {
            long_path = Some(
                String::from_utf8_lossy(content)
                    .split('\0')
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            );
        } else if matches!(kind, b'x' | b'g') {
            if size > 65536 {
                return Err("Archive metadata is too large.".into());
            }
            let mut cursor = 0;
            while cursor < content.len() {
                let bad = || "The TAR metadata is damaged.".to_owned();
                let separator = content[cursor..]
                    .iter()
                    .position(|&b| b == b' ')
                    .map(|i| i + cursor)
                    .ok_or_else(bad)?;
                if separator - cursor > 10 {
                    return Err(bad());
                }
                let digits = &content[cursor..separator];
                if digits.first().is_none_or(|b| !(b'1'..=b'9').contains(b))
                    || !digits.iter().all(u8::is_ascii_digit)
                {
                    return Err(bad());
                }
                let length: usize = std::str::from_utf8(digits)
                    .map_err(|_| bad())?
                    .parse()
                    .map_err(|_| bad())?;
                let end = cursor.checked_add(length).ok_or_else(bad)?;
                if end > content.len() || end <= separator + 1 || content[end - 1] != b'\n' {
                    return Err(bad());
                }
                let record = String::from_utf8_lossy(&content[separator + 1..end - 1]);
                let (key, value) = record
                    .split_once('=')
                    .filter(|(key, _)| !key.is_empty())
                    .ok_or_else(bad)?;
                if matches!(key, "path" | "size" | "mtime") {
                    let target = if kind == b'x' {
                        &mut local
                    } else {
                        &mut global
                    };
                    if kind == b'g' && value.is_empty() {
                        target.remove(key);
                    } else {
                        target.insert(key.into(), value.into());
                    }
                }
                cursor = end;
            }
        } else if matches!(kind, 0 | b'0' | b'5') {
            let modified = if let Some(value) = attributes.get("mtime").filter(|v| !v.is_empty()) {
                static MTIME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                    regex::Regex::new(r"^[+-]?\d+(?:\.\d+)?$").unwrap()
                });
                if !MTIME.is_match(value) {
                    return Err("The TAR metadata is damaged.".into());
                }
                value.parse::<f64>().unwrap_or(f64::NAN)
            } else {
                number(136, 12)? as f64
            };
            entries.push(Entry {
                path,
                bytes: content.to_vec(),
                directory: kind == b'5',
                modified: date(modified),
            });
        }
        offset += 512 + size.div_ceil(512) * 512;
    }
    if entries.is_empty() {
        return Err("This archive has no readable entries.".into());
    }
    Ok(Archive { entries })
}

fn u16_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or("This ZIP directory is damaged.")?
            .try_into()
            .unwrap(),
    ) as usize)
}
fn u32_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or("This ZIP directory is damaged.")?
            .try_into()
            .unwrap(),
    ) as usize)
}
fn u64_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    usize::try_from(u64::from_le_bytes(
        bytes
            .get(at..at + 8)
            .ok_or("Unsupported ZIP64 directory.")?
            .try_into()
            .unwrap(),
    ))
    .map_err(|_| "This ZIP64 directory is too large or damaged.".into())
}
fn resolve(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

fn zip(bytes: &[u8], deadline: Instant) -> Result<Archive, String> {
    let bad = || "This is not a readable ZIP archive.".to_owned();
    let end = (bytes.len().saturating_sub(65557)..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&at| bytes.get(at..at + 4) == Some(b"PK\x05\x06"))
        .ok_or_else(bad)?;
    if end + 22 + u16_at(bytes, end + 20)? != bytes.len() {
        return Err(bad());
    }
    let mut count = u16_at(bytes, end + 10)?;
    let mut size = u32_at(bytes, end + 12)?;
    let mut start = u32_at(bytes, end + 16)?;
    let mut directory_end = end;
    if [4, 6, 8, 10]
        .into_iter()
        .any(|field| u16_at(bytes, end + field) == Ok(65535))
        || size == u32::MAX as usize
        || start == u32::MAX as usize
    {
        let record = end.checked_sub(76).ok_or("Unsupported ZIP64 directory.")?;
        let locator = end - 20;
        if bytes.get(locator..locator + 4) != Some(b"PK\x06\x07")
            || bytes.get(record..record + 4) != Some(b"PK\x06\x06")
            || u64_at(bytes, record + 4)? != 44
            || u32_at(bytes, record + 16)? != 0
            || u32_at(bytes, record + 20)? != 0
            || u32_at(bytes, locator + 4)? != 0
            || u32_at(bytes, locator + 16)? != 1
            || u64_at(bytes, locator + 8)? != record
            || bytes[locator + 1..].windows(4).any(|w| w == b"PK\x06\x07")
        {
            return Err("Unsupported ZIP64 directory.".into());
        }
        count = u64_at(bytes, record + 32)?;
        size = u64_at(bytes, record + 40)?;
        start = u64_at(bytes, record + 48)?;
        if count > MAX_ENTRIES
            || size > record
            || start > record
            || start.checked_add(size) != Some(record)
            || u64_at(bytes, record + 24)? != count
        {
            return Err("This ZIP64 directory is too large or damaged.".into());
        }
        directory_end = record;
    } else if u16_at(bytes, end + 4)? != 0 || u16_at(bytes, end + 6)? != 0 {
        return Err("Multi-volumes zip are not supported".into());
    }
    if count > MAX_ENTRIES {
        return Err("This archive contains more than 2,048 entries.".into());
    }
    let actual_start = directory_end.checked_sub(size).ok_or_else(bad)?;
    if start > actual_start {
        return Err("This ZIP directory is damaged.".into());
    }
    // KSG checkZipDirectory validates the entire central directory before
    // allocating entries or starting any decompression, including a false EOCD count.
    let mut cursor = actual_start;
    let mut directory_records = 0usize;
    while cursor < directory_end {
        if cursor + 46 > directory_end || bytes.get(cursor..cursor + 4) != Some(b"PK\x01\x02") {
            return Err("This ZIP directory is damaged.".into());
        }
        directory_records += 1;
        if directory_records > MAX_ENTRIES {
            return Err("This archive contains more than 2,048 entries.".into());
        }
        let name_len = u16_at(bytes, cursor + 28)?;
        if name_len > 1024 {
            return Err("An archive path is too long to preview.".into());
        }
        cursor += 46 + name_len + u16_at(bytes, cursor + 30)? + u16_at(bytes, cursor + 32)?;
        if cursor > directory_end {
            return Err("This ZIP directory is damaged.".into());
        }
    }
    let shift = actual_start - start;
    let mut at = actual_start;
    struct Member<'a> {
        path: String,
        data: &'a [u8],
        length: usize,
        method: usize,
        directory: bool,
        modified: Option<String>,
    }
    let mut members = Vec::<Member<'_>>::new();
    let mut records = 0usize;
    while at < directory_end {
        if Instant::now() > deadline {
            return Err(TIMEOUT.into());
        }
        records += 1;
        if records > MAX_ENTRIES {
            return Err("This archive contains more than 2,048 entries.".into());
        }
        if at + 46 > directory_end || bytes.get(at..at + 4) != Some(b"PK\x01\x02") {
            return Err("This ZIP directory is damaged.".into());
        }
        let name_len = u16_at(bytes, at + 28)?;
        if name_len > 1024 {
            return Err("An archive path is too long to preview.".into());
        }
        let extra_len = u16_at(bytes, at + 30)?;
        let end = at + 46 + name_len + extra_len + u16_at(bytes, at + 32)?;
        if end > directory_end {
            return Err("This ZIP directory is damaged.".into());
        }
        if u16_at(bytes, at + 8)? & 1 != 0 {
            return Err("Encrypted zip are not supported".into());
        }
        let mut compressed = u32_at(bytes, at + 20)?;
        let mut length = u32_at(bytes, at + 24)?;
        let mut local = u32_at(bytes, at + 42)?;
        let encoded_name = &bytes[at + 46..at + 46 + name_len];
        let mut unicode_name = None;
        let mut extra_at = at + 46 + name_len;
        while extra_at + 4 <= at + 46 + name_len + extra_len {
            let tag = u16_at(bytes, extra_at)?;
            let len = u16_at(bytes, extra_at + 2)?;
            let end = extra_at + 4 + len;
            if end > at + 46 + name_len + extra_len {
                return Err("This ZIP directory is damaged.".into());
            }
            if tag == 1 {
                let mut field = extra_at + 4;
                for value in [&mut length, &mut compressed, &mut local] {
                    if *value == u32::MAX as usize {
                        if field + 8 > end {
                            return Err("Unsupported ZIP64 directory.".into());
                        }
                        *value = u64_at(bytes, field)?;
                        field += 8;
                    }
                }
            }
            // JSZip honors Info-ZIP Unicode Path only when its CRC matches the
            // original name. Invalid UTF-8 otherwise follows TextDecoder replacement.
            if tag == 0x7075
                && len >= 5
                && bytes[extra_at + 4] == 1
                && u32_at(bytes, extra_at + 5)? == crc32fast::hash(encoded_name) as usize
            {
                unicode_name =
                    Some(String::from_utf8_lossy(&bytes[extra_at + 9..end]).into_owned());
            }
            extra_at = end;
        }
        let name = if u16_at(bytes, at + 8)? & 0x800 != 0 {
            String::from_utf8_lossy(encoded_name).into_owned()
        } else {
            unicode_name.unwrap_or_else(|| String::from_utf8_lossy(encoded_name).into_owned())
        };
        let path = resolve(&name);
        if path.encode_utf16().count() > 1024 {
            return Err("An archive path is too long to preview.".into());
        }
        let local = local
            .checked_add(shift)
            .ok_or("This ZIP directory is damaged.")?;
        if bytes.get(local..local + 4) != Some(b"PK\x03\x04") {
            return Err("This ZIP directory is damaged.".into());
        }
        let data_at = local + 30 + u16_at(bytes, local + 26)? + u16_at(bytes, local + 28)?;
        let data = bytes
            .get(
                data_at
                    ..data_at
                        .checked_add(compressed)
                        .ok_or("This ZIP directory is damaged.")?,
            )
            .ok_or("This ZIP directory is damaged.")?;
        let method = u16_at(bytes, at + 10)?;
        if !matches!(method, 0 | 8) {
            return Err("This archive uses an unsupported compression method.".into());
        }
        let dos_time = u16_at(bytes, at + 12)?;
        let dos_date = u16_at(bytes, at + 14)?;
        // JSZip constructs a UTC JavaScript Date, which normalizes zero days
        // and overflowing DOS fields rather than rejecting them.
        let month = (((dos_date >> 9) + 1980) as i32) * 12 + ((dos_date >> 5) & 15) as i32 - 1;
        let modified = chrono::NaiveDate::from_ymd_opt(
            month.div_euclid(12),
            month.rem_euclid(12) as u32 + 1,
            1,
        )
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|d| {
            d.checked_add_signed(chrono::TimeDelta::seconds(
                ((dos_date & 31) as i64 - 1) * 86400
                    + (dos_time >> 11) as i64 * 3600
                    + ((dos_time >> 5) & 63) as i64 * 60
                    + ((dos_time & 31) * 2) as i64,
            ))
        })
        .map(|d| {
            d.and_utc()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });
        let directory = path.ends_with('/') || u32_at(bytes, at + 38)? & 16 != 0;
        let member = Member {
            path: path.clone(),
            data,
            length,
            method,
            directory,
            modified,
        };
        if let Some(previous) = members.iter_mut().find(|e| e.path == path) {
            *previous = member;
        } else {
            members.push(member);
        }
        at = end;
    }
    // JSZip constructs its last-entry-wins map before boundedZip starts any
    // streams. Charging overwritten entries would reject otherwise valid ZIPs.
    let mut total = 0usize;
    let mut entries = Vec::new();
    for member in members {
        let limit = MAX_BYTES
            .saturating_sub(total)
            .min(
                bytes
                    .len()
                    .max(1)
                    .saturating_mul(MAX_RATIO)
                    .saturating_sub(total),
            )
            .min(if member.path.ends_with(".xml") {
                8 * 1024 * 1024
            } else {
                MAX_BYTES
            });
        if !member.directory && member.length > limit {
            return Err(EXPANSION_ERROR.into());
        }
        let content = if member.directory {
            Vec::new()
        } else if member.method == 0 {
            read_bounded(member.data, limit, deadline)?
        } else {
            read_bounded(
                flate2::read::DeflateDecoder::new(member.data),
                limit,
                deadline,
            )?
        };
        if !member.directory && content.len() != member.length {
            return Err("Bug : uncompressed data size mismatch".into());
        }
        total += content.len();
        entries.push(Entry {
            path: member.path,
            bytes: content,
            directory: member.directory,
            modified: member.modified,
        });
    }
    Ok(Archive { entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar(path: &str, content: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 512 + content.len().div_ceil(512) * 512 + 512];
        bytes[..path.len()].copy_from_slice(path.as_bytes());
        bytes[124..136].copy_from_slice(format!("{:011o}\0", content.len()).as_bytes());
        bytes[136..148].copy_from_slice(b"00000000000\0");
        bytes[148..156].fill(b' ');
        bytes[156] = b'0';
        let checksum: usize = bytes[..512].iter().map(|&b| b as usize).sum();
        bytes[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        bytes[512..512 + content.len()].copy_from_slice(content);
        bytes
    }

    #[test]
    fn tar_paths_are_display_only_and_entries_keep_bytes() {
        let bytes = tar("../outside.txt", b"hello");
        let archive = Archive::parse("sample.tar", &bytes).unwrap();
        assert_eq!(archive.entries.len(), 1);
        assert_eq!(archive.entries[0].path, "../outside.txt");
        assert_eq!(archive.entry("../outside.txt").unwrap(), b"hello");
        let mut damaged = bytes.clone();
        damaged[0] ^= 1;
        assert_eq!(
            Archive::parse("x.tar", &damaged).err().unwrap(),
            "The TAR header is damaged."
        );
        assert!(Archive::parse("x.tar", &[0; 512]).is_err());
    }

    #[test]
    fn gzip_expansion_and_entry_limits_are_separate() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(b"hello").unwrap();
        let archive = Archive::parse("notes.txt.gz", &encoder.finish().unwrap()).unwrap();
        assert_eq!(archive.entry("notes.txt").unwrap(), b"hello");
        let archive = Archive {
            entries: vec![Entry {
                path: "large".into(),
                bytes: vec![0; ENTRY_MAX_BYTES + 1],
                directory: false,
                modified: None,
            }],
        };
        assert_eq!(
            archive.entry("large").err().unwrap(),
            "Entry previews are limited to 10 MB."
        );
    }

    #[test]
    fn zip_directory_checks_happen_before_decoding_entries() {
        assert!(Archive::parse("bad.zip", b"PK\x03\x04").is_err());
        let mut empty = vec![0; 22];
        empty[..4].copy_from_slice(b"PK\x05\x06");
        assert!(
            Archive::parse("empty.zip", &empty)
                .unwrap()
                .entries
                .is_empty()
        );
        empty[10..12].copy_from_slice(&2049_u16.to_le_bytes());
        assert_eq!(
            Archive::parse("large.zip", &empty).err().unwrap(),
            "This archive contains more than 2,048 entries."
        );
    }

    #[test]
    fn stored_and_deflated_zip_members_are_bounded_and_resolved_without_disk_extraction() {
        use std::io::Write;
        for deflated in [false, true] {
            let name = b"parent/../notes.txt";
            let content = b"archived text";
            let payload = if deflated {
                let mut encoder =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(content).unwrap();
                encoder.finish().unwrap()
            } else {
                content.to_vec()
            };
            let mut local = vec![0; 30];
            local[..4].copy_from_slice(b"PK\x03\x04");
            local[4..6].copy_from_slice(&20_u16.to_le_bytes());
            local[8..10]
                .copy_from_slice(&u16::from(deflated).checked_mul(8).unwrap().to_le_bytes());
            local[18..22].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            local[22..26].copy_from_slice(&(content.len() as u32).to_le_bytes());
            local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
            local.extend_from_slice(name);
            local.extend_from_slice(&payload);
            let start = local.len();
            let mut central = vec![0; 46];
            central[..4].copy_from_slice(b"PK\x01\x02");
            central[10..12].copy_from_slice(&(if deflated { 8_u16 } else { 0 }).to_le_bytes());
            central[20..24].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            central[24..28].copy_from_slice(&(content.len() as u32).to_le_bytes());
            central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(name);
            local.extend_from_slice(&central);
            let mut end = vec![0; 22];
            end[..4].copy_from_slice(b"PK\x05\x06");
            end[8..10].copy_from_slice(&1_u16.to_le_bytes());
            end[10..12].copy_from_slice(&1_u16.to_le_bytes());
            end[12..16].copy_from_slice(&(central.len() as u32).to_le_bytes());
            end[16..20].copy_from_slice(&(start as u32).to_le_bytes());
            local.extend_from_slice(&end);
            let archive = Archive::parse("fixture.zip", &local).unwrap();
            assert_eq!(archive.entry("notes.txt").unwrap(), content);
            assert_eq!(archive.entries[0].path, "notes.txt");
        }
    }

    #[test]
    fn gzip_ratio_and_tar_numeric_damage_use_reference_errors() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&vec![0; 1024 * 1024]).unwrap();
        assert_eq!(
            Archive::parse("bomb.gz", &encoder.finish().unwrap())
                .err()
                .unwrap(),
            EXPANSION_ERROR
        );
        let mut bytes = tar("notes.txt", b"hello");
        bytes[124] = 0x80;
        assert_eq!(
            Archive::parse("number.tar", &bytes).err().unwrap(),
            "The TAR header is damaged."
        );
    }

    #[test]
    fn duplicate_zip_members_resolve_before_the_expansion_budget() {
        use std::io::Write;
        let content = vec![0; 20 * 1024 * 1024];
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&content).unwrap();
        let payload = encoder.finish().unwrap();
        let name = b"same.txt";
        let mut bytes = Vec::new();
        let mut directory = Vec::new();
        for _ in 0..3 {
            let local_offset = bytes.len();
            let mut local = vec![0; 30];
            local[..4].copy_from_slice(b"PK\x03\x04");
            local[8..10].copy_from_slice(&8_u16.to_le_bytes());
            local[18..22].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            local[22..26].copy_from_slice(&(content.len() as u32).to_le_bytes());
            local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&local);
            bytes.extend_from_slice(name);
            bytes.extend_from_slice(&payload);
            let mut central = vec![0; 46];
            central[..4].copy_from_slice(b"PK\x01\x02");
            central[10..12].copy_from_slice(&8_u16.to_le_bytes());
            central[20..24].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            central[24..28].copy_from_slice(&(content.len() as u32).to_le_bytes());
            central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            central[42..46].copy_from_slice(&(local_offset as u32).to_le_bytes());
            directory.extend_from_slice(&central);
            directory.extend_from_slice(name);
        }
        let start = bytes.len();
        bytes.extend_from_slice(&directory);
        let mut end = vec![0; 22];
        end[..4].copy_from_slice(b"PK\x05\x06");
        end[8..10].copy_from_slice(&3_u16.to_le_bytes());
        end[10..12].copy_from_slice(&3_u16.to_le_bytes());
        end[12..16].copy_from_slice(&(directory.len() as u32).to_le_bytes());
        end[16..20].copy_from_slice(&(start as u32).to_le_bytes());
        bytes.extend_from_slice(&end);
        let archive = Archive::parse("duplicates.zip", &bytes).unwrap();
        assert_eq!(archive.entries.len(), 1);
        assert_eq!(archive.entries[0].bytes.len(), content.len());
    }
}
