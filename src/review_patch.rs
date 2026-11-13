#![allow(dead_code)] // Used by the diff panel, which lands in a later change.

const UNSUPPORTED_DIFF_REASON: &str = "Unsupported diff format. Showing raw patch.";
const FAILED_PARSE_REASON: &str = "Failed to parse patch. Showing raw patch.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub text: String, // without the leading ' ', '+' or '-' and without the line ending
    pub old_number: Option<u32>, // None for Added
    pub new_number: Option<u32>, // None for Deleted
    pub no_newline: bool, // true when followed by "\ No newline at end of file"
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub section: String, // text after the second "@@", trimmed; may be empty
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,                  // new path, or old path for a deleted file
    pub previous_path: Option<String>, // set only for Renamed and Copied
    pub change: ChangeKind,
    pub binary: bool,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub hunks: Vec<Hunk>,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderablePatch {
    Files(Vec<FileDiff>),
    Raw { text: String, reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineStat {
    pub additions: u32,
    pub deletions: u32,
}

pub fn line_stat(files: &[FileDiff]) -> LineStat {
    let mut additions: u32 = 0;
    let mut deletions: u32 = 0;
    for file in files {
        additions = additions.saturating_add(file.additions);
        deletions = deletions.saturating_add(file.deletions);
    }
    LineStat {
        additions,
        deletions,
    }
}

pub fn fnv1a32(text: &str) -> u32 {
    const FNV_OFFSET_BASIS_32: u32 = 0x811c_9dc5;
    const FNV_PRIME_32: u32 = 0x0100_0193;

    let mut hash = FNV_OFFSET_BASIS_32;
    for code_unit in text.encode_utf16() {
        hash ^= u32::from(code_unit);
        hash = hash.wrapping_mul(FNV_PRIME_32);
    }
    hash
}

pub fn identity_key(file: &FileDiff) -> String {
    let path = if file.path.is_empty() {
        file.previous_path.as_deref().unwrap_or("")
    } else {
        file.path.as_str()
    };
    let previous = file.previous_path.as_deref().unwrap_or(path);
    let kind = match file.change {
        ChangeKind::Added => "added",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Modified => "modified",
        ChangeKind::Renamed => "renamed",
        ChangeKind::Copied => "copied",
    };
    format!("{previous}\0{path}\0{kind}")
}

pub fn all_collapsed(files: &[FileDiff], collapsed: &std::collections::HashSet<String>) -> bool {
    !files.is_empty()
        && files
            .iter()
            .all(|file| collapsed.contains(&identity_key(file)))
}

pub fn toggle_all(files: &[FileDiff], collapsed: &mut std::collections::HashSet<String>) {
    if all_collapsed(files, collapsed) {
        collapsed.clear();
    } else {
        collapsed.clear();
        for file in files {
            collapsed.insert(identity_key(file));
        }
    }
}

struct FileBuilder {
    diff_old_path: String,
    diff_new_path: String,
    minus_path: Option<String>,
    plus_path: Option<String>,
    minus_is_dev_null: bool,
    plus_is_dev_null: bool,
    has_new_file_mode: bool,
    has_deleted_file_mode: bool,
    rename_from: Option<String>,
    rename_to: Option<String>,
    copy_from: Option<String>,
    copy_to: Option<String>,
    binary: bool,
    old_mode: Option<String>,
    new_mode: Option<String>,
    hunks: Vec<Hunk>,
    additions: u32,
    deletions: u32,
}

impl FileBuilder {
    fn from_diff_git_line(line: &str) -> Self {
        let rest = line.strip_prefix("diff --git ").unwrap_or("");
        let (diff_old_path, diff_new_path) = parse_diff_git_paths(rest);
        Self {
            diff_old_path,
            diff_new_path,
            minus_path: None,
            plus_path: None,
            minus_is_dev_null: false,
            plus_is_dev_null: false,
            has_new_file_mode: false,
            has_deleted_file_mode: false,
            rename_from: None,
            rename_to: None,
            copy_from: None,
            copy_to: None,
            binary: false,
            old_mode: None,
            new_mode: None,
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
        }
    }

    fn build(self) -> FileDiff {
        let change = if self.has_new_file_mode || self.minus_is_dev_null {
            ChangeKind::Added
        } else if self.has_deleted_file_mode || self.plus_is_dev_null {
            ChangeKind::Deleted
        } else if self.rename_from.is_some() {
            ChangeKind::Renamed
        } else if self.copy_from.is_some() {
            ChangeKind::Copied
        } else {
            ChangeKind::Modified
        };

        let (path, previous_path) = match change {
            ChangeKind::Renamed => {
                let path = self
                    .rename_to
                    .or(self.plus_path)
                    .unwrap_or(self.diff_new_path);
                let prev = self
                    .rename_from
                    .or(self.minus_path)
                    .unwrap_or(self.diff_old_path);
                (path, Some(prev))
            }
            ChangeKind::Copied => {
                let path = self
                    .copy_to
                    .or(self.plus_path)
                    .unwrap_or(self.diff_new_path);
                let prev = self
                    .copy_from
                    .or(self.minus_path)
                    .unwrap_or(self.diff_old_path);
                (path, Some(prev))
            }
            ChangeKind::Deleted => {
                let path = self.minus_path.unwrap_or(self.diff_old_path);
                (path, None)
            }
            ChangeKind::Added => {
                let path = self.plus_path.unwrap_or(self.diff_new_path);
                (path, None)
            }
            ChangeKind::Modified => {
                let path = self
                    .plus_path
                    .or(self.minus_path)
                    .unwrap_or(self.diff_new_path);
                (path, None)
            }
        };

        FileDiff {
            path,
            previous_path,
            change,
            binary: self.binary,
            old_mode: self.old_mode,
            new_mode: self.new_mode,
            hunks: self.hunks,
            additions: self.additions,
            deletions: self.deletions,
        }
    }
}

struct HunkBuilder {
    hunk: Hunk,
    old_line: u32,
    new_line: u32,
    old_remaining: u32,
    new_remaining: u32,
}

pub fn parse(patch: &str) -> Option<RenderablePatch> {
    if patch.trim().is_empty() {
        return None;
    }

    let mut files = Vec::new();
    let mut current_file: Option<FileBuilder> = None;
    let mut current_hunk: Option<HunkBuilder> = None;
    let mut in_hunks = false;

    for raw_line in patch.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

        if line.starts_with("diff --git ") {
            if let Some(mut file) = current_file.take() {
                if let Some(hb) = current_hunk.take() {
                    file.hunks.push(hb.hunk);
                }
                files.push(file.build());
            }
            current_file = Some(FileBuilder::from_diff_git_line(line));
            in_hunks = false;
            continue;
        }

        let Some(file) = current_file.as_mut() else {
            continue;
        };

        if line.starts_with("@@") {
            if let Some(hb) = current_hunk.take() {
                file.hunks.push(hb.hunk);
            }
            in_hunks = true;
            let Some((old_start, old_count, new_start, new_count, section)) =
                parse_hunk_header(line)
            else {
                return Some(RenderablePatch::Raw {
                    text: patch.to_string(),
                    reason: FAILED_PARSE_REASON.to_string(),
                });
            };
            current_hunk = Some(HunkBuilder {
                hunk: Hunk {
                    old_start,
                    old_count,
                    new_start,
                    new_count,
                    section,
                    lines: Vec::new(),
                },
                old_line: old_start,
                new_line: new_start,
                old_remaining: old_count,
                new_remaining: new_count,
            });
            continue;
        }

        if in_hunks {
            if let Some(hb) = current_hunk.as_mut() {
                if line == "\\ No newline at end of file" {
                    if let Some(last) = hb.hunk.lines.last_mut() {
                        last.no_newline = true;
                    }
                    continue;
                }

                if hb.old_remaining > 0 || hb.new_remaining > 0 {
                    if let Some(text) = line.strip_prefix(' ') {
                        hb.hunk.lines.push(Line {
                            kind: LineKind::Context,
                            text: text.to_string(),
                            old_number: Some(hb.old_line),
                            new_number: Some(hb.new_line),
                            no_newline: false,
                        });
                        hb.old_line = hb.old_line.saturating_add(1);
                        hb.new_line = hb.new_line.saturating_add(1);
                        hb.old_remaining = hb.old_remaining.saturating_sub(1);
                        hb.new_remaining = hb.new_remaining.saturating_sub(1);
                    } else if line.is_empty() {
                        hb.hunk.lines.push(Line {
                            kind: LineKind::Context,
                            text: String::new(),
                            old_number: Some(hb.old_line),
                            new_number: Some(hb.new_line),
                            no_newline: false,
                        });
                        hb.old_line = hb.old_line.saturating_add(1);
                        hb.new_line = hb.new_line.saturating_add(1);
                        hb.old_remaining = hb.old_remaining.saturating_sub(1);
                        hb.new_remaining = hb.new_remaining.saturating_sub(1);
                    } else if let Some(text) = line.strip_prefix('+') {
                        hb.hunk.lines.push(Line {
                            kind: LineKind::Added,
                            text: text.to_string(),
                            old_number: None,
                            new_number: Some(hb.new_line),
                            no_newline: false,
                        });
                        hb.new_line = hb.new_line.saturating_add(1);
                        hb.new_remaining = hb.new_remaining.saturating_sub(1);
                        file.additions = file.additions.saturating_add(1);
                    } else if let Some(text) = line.strip_prefix('-') {
                        hb.hunk.lines.push(Line {
                            kind: LineKind::Deleted,
                            text: text.to_string(),
                            old_number: Some(hb.old_line),
                            new_number: None,
                            no_newline: false,
                        });
                        hb.old_line = hb.old_line.saturating_add(1);
                        hb.old_remaining = hb.old_remaining.saturating_sub(1);
                        file.deletions = file.deletions.saturating_add(1);
                    } else if let Some(finished) = current_hunk.take() {
                        file.hunks.push(finished.hunk);
                    }
                } else if let Some(finished) = current_hunk.take() {
                    file.hunks.push(finished.hunk);
                }
            }
            continue;
        }

        if let Some(mode) = line.strip_prefix("old mode ") {
            file.old_mode = Some(mode.trim().to_string());
        } else if let Some(mode) = line.strip_prefix("new mode ") {
            file.new_mode = Some(mode.trim().to_string());
        } else if let Some(mode) = line.strip_prefix("new file mode ") {
            file.has_new_file_mode = true;
            file.new_mode = Some(mode.trim().to_string());
        } else if let Some(mode) = line.strip_prefix("deleted file mode ") {
            file.has_deleted_file_mode = true;
            file.old_mode = Some(mode.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("rename from ") {
            file.rename_from = Some(decode_git_path(rest));
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            file.rename_to = Some(decode_git_path(rest));
        } else if let Some(rest) = line.strip_prefix("copy from ") {
            file.copy_from = Some(decode_git_path(rest));
        } else if let Some(rest) = line.strip_prefix("copy to ") {
            file.copy_to = Some(decode_git_path(rest));
        } else if let Some(rest) = line.strip_prefix("--- ") {
            match parse_header_path(rest, "a/") {
                HeaderPath::DevNull => {
                    file.minus_is_dev_null = true;
                    file.minus_path = None;
                }
                HeaderPath::Path(path) => {
                    file.minus_path = Some(path);
                }
            }
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            match parse_header_path(rest, "b/") {
                HeaderPath::DevNull => {
                    file.plus_is_dev_null = true;
                    file.plus_path = None;
                }
                HeaderPath::Path(path) => {
                    file.plus_path = Some(path);
                }
            }
        } else if (line.starts_with("Binary files ") && line.ends_with(" differ"))
            || line == "GIT binary patch"
        {
            file.binary = true;
        }
    }

    if let Some(mut file) = current_file.take() {
        if let Some(hb) = current_hunk.take() {
            file.hunks.push(hb.hunk);
        }
        files.push(file.build());
    }

    if files.is_empty() {
        Some(RenderablePatch::Raw {
            text: patch.to_string(),
            reason: UNSUPPORTED_DIFF_REASON.to_string(),
        })
    } else {
        Some(RenderablePatch::Files(files))
    }
}

enum HeaderPath {
    DevNull,
    Path(String),
}

fn parse_header_path(raw: &str, prefix: &str) -> HeaderPath {
    let decoded = if raw.starts_with('"') {
        decode_git_path(raw)
    } else {
        raw.split('\t').next().unwrap_or(raw).to_string()
    };

    if decoded == "/dev/null" {
        HeaderPath::DevNull
    } else {
        HeaderPath::Path(strip_git_prefix(decoded, prefix))
    }
}

fn strip_git_prefix(path: String, prefix: &str) -> String {
    path.strip_prefix(prefix)
        .map(str::to_string)
        .unwrap_or(path)
}

fn decode_git_path(raw: &str) -> String {
    if raw.starts_with('"') {
        if let Some((decoded, _)) = parse_quoted_token(raw) {
            return decoded;
        }
    }
    raw.to_string()
}

fn parse_quoted_token(s: &str) -> Option<(String, &str)> {
    let inner_and_after = s.strip_prefix('"')?;
    let bytes = inner_and_after.as_bytes();
    let mut i = 0;
    let mut escaped = false;
    while let Some(&b) = bytes.get(i) {
        if escaped {
            escaped = false;
        } else if b == b'\\' {
            escaped = true;
        } else if b == b'"' {
            let inner = inner_and_after.get(..i)?;
            let rest = inner_and_after.get(i + 1..).unwrap_or("");
            return Some((unescape_c_quoted(inner), rest));
        }
        i += 1;
    }
    None
}

fn unescape_c_quoted(inner: &str) -> String {
    let bytes = inner.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b != b'\\' {
            out.push(b);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&next) = bytes.get(i) else {
            out.push(b'\\');
            break;
        };
        match next {
            b'\\' => {
                out.push(b'\\');
                i += 1;
            }
            b'"' => {
                out.push(b'"');
                i += 1;
            }
            b't' => {
                out.push(b'\t');
                i += 1;
            }
            b'n' => {
                out.push(b'\n');
                i += 1;
            }
            b'r' => {
                out.push(b'\r');
                i += 1;
            }
            b'a' => {
                out.push(0x07);
                i += 1;
            }
            b'b' => {
                out.push(0x08);
                i += 1;
            }
            b'v' => {
                out.push(0x0b);
                i += 1;
            }
            b'f' => {
                out.push(0x0c);
                i += 1;
            }
            b'0'..=b'7' => {
                let mut val: u16 = u16::from(next - b'0');
                i += 1;
                for _ in 0..2 {
                    if let Some(&digit) = bytes.get(i) {
                        if matches!(digit, b'0'..=b'7') {
                            val = val * 8 + u16::from(digit - b'0');
                            i += 1;
                            continue;
                        }
                    }
                    break;
                }
                out.push((val & 0xff) as u8);
            }
            other => {
                out.push(b'\\');
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_diff_git_paths(rest: &str) -> (String, String) {
    if rest.starts_with('"') {
        if let Some((old_decoded, tail)) = parse_quoted_token(rest) {
            let new_raw = tail.trim_start();
            let new_decoded = decode_git_path(new_raw);
            return (
                strip_git_prefix(old_decoded, "a/"),
                strip_git_prefix(new_decoded, "b/"),
            );
        }
    }

    if rest.ends_with('"') {
        for (idx, _) in rest.match_indices(" \"") {
            let Some(left) = rest.get(..idx) else {
                continue;
            };
            let Some(right_candidate) = rest.get(idx + 1..) else {
                continue;
            };
            if let Some((new_decoded, tail)) = parse_quoted_token(right_candidate) {
                if tail.trim().is_empty() {
                    return (
                        strip_git_prefix(left.to_string(), "a/"),
                        strip_git_prefix(new_decoded, "b/"),
                    );
                }
            }
        }
    }

    for (idx, _) in rest.match_indices(" b/") {
        let Some(left) = rest.get(..idx) else {
            continue;
        };
        let Some(right) = rest.get(idx + 1..) else {
            continue;
        };
        if left.strip_prefix("a/") == right.strip_prefix("b/") {
            return (
                strip_git_prefix(left.to_string(), "a/"),
                strip_git_prefix(right.to_string(), "b/"),
            );
        }
    }

    if let Some((idx, _)) = rest.match_indices(" b/").next() {
        if let (Some(left), Some(right)) = (rest.get(..idx), rest.get(idx + 1..)) {
            return (
                strip_git_prefix(left.to_string(), "a/"),
                strip_git_prefix(right.to_string(), "b/"),
            );
        }
    }

    if let Some((left, right)) = rest.split_once(' ') {
        (
            strip_git_prefix(left.to_string(), "a/"),
            strip_git_prefix(right.to_string(), "b/"),
        )
    } else {
        (strip_git_prefix(rest.to_string(), "a/"), String::new())
    }
}

fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32, String)> {
    let after_open = line.strip_prefix("@@")?;
    let (range_part, section_part) = after_open.split_once("@@")?;
    let mut tokens = range_part.split_whitespace();
    let old_spec = tokens.next()?;
    let new_spec = tokens.next()?;
    if tokens.next().is_some() {
        return None;
    }
    let (old_start, old_count) = parse_hunk_range(old_spec, '-')?;
    let (new_start, new_count) = parse_hunk_range(new_spec, '+')?;
    Some((
        old_start,
        old_count,
        new_start,
        new_count,
        section_part.trim().to_string(),
    ))
}

fn parse_hunk_range(spec: &str, prefix: char) -> Option<(u32, u32)> {
    let body = spec.strip_prefix(prefix)?;
    if let Some((start_s, count_s)) = body.split_once(',') {
        Some((parse_ascii_u32(start_s)?, parse_ascii_u32(count_s)?))
    } else {
        Some((parse_ascii_u32(body)?, 1))
    }
}

fn parse_ascii_u32(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_files(patch: &str) -> Vec<FileDiff> {
        match parse(patch) {
            Some(RenderablePatch::Files(files)) => files,
            other => panic!("expected Files, got {other:?}"),
        }
    }

    #[test]
    fn modified_file_with_two_hunks() {
        let patch = "\
commit abc123
Author: Test <test@example.com>

diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@ fn first()
 line 1
-line 2
+line 2a
+line 2b
 line 3
@@ -10,3 +11,2 @@ fn second()
 keep
-drop
 end
";
        let files = expect_files(patch);
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert_eq!(file.path, "src/lib.rs");
        assert_eq!(file.previous_path, None);
        assert_eq!(file.change, ChangeKind::Modified);
        assert!(!file.binary);
        assert_eq!(file.old_mode, None);
        assert_eq!(file.new_mode, None);
        assert_eq!(file.additions, 2);
        assert_eq!(file.deletions, 2);
        assert_eq!(file.hunks.len(), 2);

        let h0 = &file.hunks[0];
        assert_eq!(h0.old_start, 1);
        assert_eq!(h0.old_count, 3);
        assert_eq!(h0.new_start, 1);
        assert_eq!(h0.new_count, 4);
        assert_eq!(h0.section, "fn first()");
        assert_eq!(
            h0.lines,
            vec![
                Line {
                    kind: LineKind::Context,
                    text: "line 1".to_string(),
                    old_number: Some(1),
                    new_number: Some(1),
                    no_newline: false,
                },
                Line {
                    kind: LineKind::Deleted,
                    text: "line 2".to_string(),
                    old_number: Some(2),
                    new_number: None,
                    no_newline: false,
                },
                Line {
                    kind: LineKind::Added,
                    text: "line 2a".to_string(),
                    old_number: None,
                    new_number: Some(2),
                    no_newline: false,
                },
                Line {
                    kind: LineKind::Added,
                    text: "line 2b".to_string(),
                    old_number: None,
                    new_number: Some(3),
                    no_newline: false,
                },
                Line {
                    kind: LineKind::Context,
                    text: "line 3".to_string(),
                    old_number: Some(3),
                    new_number: Some(4),
                    no_newline: false,
                },
            ]
        );

        let h1 = &file.hunks[1];
        assert_eq!(h1.old_start, 10);
        assert_eq!(h1.old_count, 3);
        assert_eq!(h1.new_start, 11);
        assert_eq!(h1.new_count, 2);
        assert_eq!(h1.section, "fn second()");
        assert_eq!(
            h1.lines,
            vec![
                Line {
                    kind: LineKind::Context,
                    text: "keep".to_string(),
                    old_number: Some(10),
                    new_number: Some(11),
                    no_newline: false,
                },
                Line {
                    kind: LineKind::Deleted,
                    text: "drop".to_string(),
                    old_number: Some(11),
                    new_number: None,
                    no_newline: false,
                },
                Line {
                    kind: LineKind::Context,
                    text: "end".to_string(),
                    old_number: Some(12),
                    new_number: Some(12),
                    no_newline: false,
                },
            ]
        );
    }

    #[test]
    fn added_and_deleted_files() {
        let patch = "\
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+alpha
+beta
diff --git a/old.txt b/old.txt
deleted file mode 100644
index 1111111..0000000
--- a/old.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-gamma
-delta
";
        let files = expect_files(patch);
        assert_eq!(files.len(), 2);

        assert_eq!(files[0].path, "new.txt");
        assert_eq!(files[0].previous_path, None);
        assert_eq!(files[0].change, ChangeKind::Added);
        assert_eq!(files[0].old_mode, None);
        assert_eq!(files[0].new_mode.as_deref(), Some("100644"));
        assert_eq!(files[0].additions, 2);
        assert_eq!(files[0].deletions, 0);

        assert_eq!(files[1].path, "old.txt");
        assert_eq!(files[1].previous_path, None);
        assert_eq!(files[1].change, ChangeKind::Deleted);
        assert_eq!(files[1].old_mode.as_deref(), Some("100644"));
        assert_eq!(files[1].new_mode, None);
        assert_eq!(files[1].additions, 0);
        assert_eq!(files[1].deletions, 2);
    }

    #[test]
    fn rename_with_changes_and_pure_rename() {
        let patch = "\
diff --git a/before.rs b/after.rs
similarity index 80%
rename from before.rs
rename to after.rs
index 1111111..2222222 100644
--- a/before.rs
+++ b/after.rs
@@ -1,2 +1,2 @@
-old
+new
 same
diff --git a/pure_old.rs b/pure_new.rs
similarity index 100%
rename from pure_old.rs
rename to pure_new.rs
";
        let files = expect_files(patch);
        assert_eq!(files.len(), 2);

        assert_eq!(files[0].path, "after.rs");
        assert_eq!(files[0].previous_path.as_deref(), Some("before.rs"));
        assert_eq!(files[0].change, ChangeKind::Renamed);
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);

        assert_eq!(files[1].path, "pure_new.rs");
        assert_eq!(files[1].previous_path.as_deref(), Some("pure_old.rs"));
        assert_eq!(files[1].change, ChangeKind::Renamed);
        assert!(files[1].hunks.is_empty());
        assert_eq!(files[1].additions, 0);
        assert_eq!(files[1].deletions, 0);
    }

    #[test]
    fn copied_file() {
        let patch = "\
diff --git a/template.txt b/copy.txt
similarity index 90%
copy from template.txt
copy to copy.txt
index 1111111..2222222 100644
--- a/template.txt
+++ b/copy.txt
@@ -1,2 +1,2 @@
 line 1
-template
+copy
";
        let files = expect_files(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "copy.txt");
        assert_eq!(files[0].previous_path.as_deref(), Some("template.txt"));
        assert_eq!(files[0].change, ChangeKind::Copied);
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);
    }

    #[test]
    fn binary_file_change() {
        let patch = "\
diff --git a/x.png b/x.png
index 1111111..2222222 100644
Binary files a/x.png and b/x.png differ
";
        let files = expect_files(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "x.png");
        assert_eq!(files[0].previous_path, None);
        assert_eq!(files[0].change, ChangeKind::Modified);
        assert!(files[0].binary);
        assert!(files[0].hunks.is_empty());
    }

    #[test]
    fn mode_only_change() {
        let patch = "\
diff --git a/bin/run.sh b/bin/run.sh
old mode 100644
new mode 100755
";
        let files = expect_files(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "bin/run.sh");
        assert_eq!(files[0].previous_path, None);
        assert_eq!(files[0].change, ChangeKind::Modified);
        assert!(!files[0].binary);
        assert_eq!(files[0].old_mode.as_deref(), Some("100644"));
        assert_eq!(files[0].new_mode.as_deref(), Some("100755"));
        assert!(files[0].hunks.is_empty());
    }

    #[test]
    fn quoted_path_with_octal_escape() {
        let patch = "\
diff --git \"a/dir/caf\\303\\251\\t\\\"1\\\".txt\" \"b/dir/caf\\303\\251\\t\\\"1\\\".txt\"
index 1111111..2222222 100644
--- \"a/dir/caf\\303\\251\\t\\\"1\\\".txt\"
+++ \"b/dir/caf\\303\\251\\t\\\"1\\\".txt\"
@@ -1 +1 @@
-a
+b
";
        let files = expect_files(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "dir/café\t\"1\".txt");
    }

    #[test]
    fn no_newline_at_end_of_file_on_last_added_line() {
        let patch = "\
diff --git a/file.txt b/file.txt
index 1111111..2222222 100644
--- a/file.txt
+++ b/file.txt
@@ -1 +1 @@
-old
+new
\\ No newline at end of file
";
        let files = expect_files(patch);
        assert_eq!(files.len(), 1);
        let lines = &files[0].hunks[0].lines;
        assert_eq!(lines.len(), 2);
        assert!(!lines[0].no_newline);
        assert!(lines[1].no_newline);
        assert_eq!(lines[1].kind, LineKind::Added);
        assert_eq!(lines[1].text, "new");
    }

    #[test]
    fn hunk_header_without_counts() {
        let patch = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old
+new
";
        let files = expect_files(patch);
        let hunk = &files[0].hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 1);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 1);
        assert_eq!(hunk.section, "");
    }

    #[test]
    fn crlf_input() {
        let patch = concat!(
            "diff --git a/win.txt b/win.txt\r\n",
            "--- a/win.txt\r\n",
            "+++ b/win.txt\r\n",
            "@@ -1,2 +1,2 @@ header\r\n",
            " keep\r\n",
            "-old\r\n",
            "+new\r\n",
        );
        let files = expect_files(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "win.txt");
        let hunk = &files[0].hunks[0];
        assert_eq!(hunk.section, "header");
        assert_eq!(hunk.lines[0].text, "keep");
        assert_eq!(hunk.lines[1].text, "old");
        assert_eq!(hunk.lines[2].text, "new");
    }

    #[test]
    fn two_files_in_one_patch() {
        let patch = "\
diff --git a/one.txt b/one.txt
--- a/one.txt
+++ b/one.txt
@@ -1 +1 @@
-a
+b
diff --git a/two.txt b/two.txt
--- a/two.txt
+++ b/two.txt
@@ -1 +1 @@
-c
+d
";
        let files = expect_files(patch);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "one.txt");
        assert_eq!(files[1].path, "two.txt");
    }

    #[test]
    fn empty_unsupported_and_broken_hunk_inputs() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("   \n\t\r\n "), None);

        let plain = "just some plain text\nwithout a git header\n";
        assert_eq!(
            parse(plain),
            Some(RenderablePatch::Raw {
                text: plain.to_string(),
                reason: "Unsupported diff format. Showing raw patch.".to_string(),
            })
        );

        let broken = "diff --git a/a.txt b/a.txt\n@@ -1,bad +1,1 @@\n-a\n";
        assert_eq!(
            parse(broken),
            Some(RenderablePatch::Raw {
                text: broken.to_string(),
                reason: "Failed to parse patch. Showing raw patch.".to_string(),
            })
        );
    }

    #[test]
    fn every_prefix_does_not_panic() {
        let sample = concat!(
            "preamble line\n",
            "diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"\r\n",
            "old mode 100644\r\n",
            "new mode 100755\r\n",
            "--- \"a/caf\\303\\251.txt\"\r\n",
            "+++ \"b/caf\\303\\251.txt\"\r\n",
            "@@ -1,3 +1,3 @@ fn demo()\r\n",
            " first\r\n",
            "-second\r\n",
            "+SECOND\r\n",
            "\r\n",
            "\\ No newline at end of file\r\n",
            "diff --git a/bin.png b/bin.png\n",
            "Binary files a/bin.png and b/bin.png differ\n",
        );

        for (byte_idx, _) in sample.char_indices() {
            let _ = parse(&sample[..byte_idx]);
        }
        let _ = parse(sample);
    }

    #[test]
    fn fnv1a32_matches_ksg_utf16() {
        assert_eq!(fnv1a32(""), 0x811c_9dc5);
        assert_eq!(fnv1a32("abc"), 0x1a47_e90b);
        assert_eq!(fnv1a32("é"), 0x6c0b_6c44);
    }

    #[test]
    fn line_stat_identity_key_and_collapse_helpers() {
        use std::collections::HashSet;

        let patch = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,3 @@
 keep
-old
+new1
+new2
diff --git a/b.rs b/c.rs
rename from b.rs
rename to c.rs
--- a/b.rs
+++ b/c.rs
@@ -1 +1 @@
-old
+new
";
        let files = expect_files(patch);
        assert_eq!(
            line_stat(&[]),
            LineStat {
                additions: 0,
                deletions: 0
            }
        );
        assert_eq!(
            line_stat(&files),
            LineStat {
                additions: 3,
                deletions: 2
            }
        );

        assert_eq!(identity_key(&files[0]), "a.rs\0a.rs\0modified");
        assert_eq!(identity_key(&files[1]), "b.rs\0c.rs\0renamed");

        let deleted_pair = FileDiff {
            path: "link.txt".to_string(),
            previous_path: None,
            change: ChangeKind::Deleted,
            binary: false,
            old_mode: None,
            new_mode: None,
            hunks: Vec::new(),
            additions: 0,
            deletions: 1,
        };
        let added_pair = FileDiff {
            change: ChangeKind::Added,
            additions: 1,
            deletions: 0,
            ..deleted_pair.clone()
        };
        assert_ne!(identity_key(&deleted_pair), identity_key(&added_pair));

        let mut collapsed = HashSet::new();
        assert!(!all_collapsed(&[], &collapsed));
        assert!(!all_collapsed(&files, &collapsed));

        collapsed.insert(identity_key(&files[0]));
        assert!(!all_collapsed(&files, &collapsed));

        collapsed.insert("stale\0key\0modified".to_string());
        toggle_all(&files, &mut collapsed);
        assert!(all_collapsed(&files, &collapsed));
        assert_eq!(collapsed.len(), 2);
        assert!(!collapsed.contains("stale\0key\0modified"));

        toggle_all(&files, &mut collapsed);
        assert!(!all_collapsed(&files, &collapsed));
        assert!(collapsed.is_empty());

        toggle_all(&[], &mut collapsed);
        assert!(collapsed.is_empty());
    }
}
