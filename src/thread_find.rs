//! Ported from KSG Code's `ThreadFind.ts`, `threadSearch.ts`, and `threadFindText.ts`.
use serde::{Deserialize, Serialize};
use std::ops::Range;

pub const THREAD_FIND_QUERY_MAX_LENGTH: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadFindStart {
    pub entry_id: String,
    pub occurrence: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadFindMatch {
    pub entry_id: String,
    pub occurrence: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadFindNavEntry {
    pub entry_id: String,
    pub count: usize,
    pub start_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadFindResult {
    pub snapshot_sequence: u64,
    pub total_matches: usize,
    pub active_index: usize,
    pub navigation: Vec<ThreadFindNavEntry>,
    #[serde(rename = "match")]
    pub active_match: Option<ThreadFindMatch>,
}

pub struct SearchDocument<'a> {
    pub entry_id: &'a str,
    pub role: &'a str,
    pub text: &'a str,
}

pub fn proposed_plan_markdown(tool: &serde_json::Value) -> Option<&str> {
    if tool["kind"] != "plan" && tool["name"] != "plan" {
        return None;
    }
    let input = &tool["input"];
    input["markdown"]
        .as_str()
        .or_else(|| input["text"].as_str())
        .or_else(|| input["plan"].as_str())
}

/// KSG `proposedPlanText.ts`. The title and displayed body are separate find regions.
pub fn plan_parts(markdown: &str) -> (String, String) {
    static HEADING: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?m)^\s{0,3}#{1,6}\s+(.+)$").unwrap());
    static FIRST_HEADING: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^\s{0,3}#{1,6}\s+").unwrap());
    let title = HEADING
        .captures(markdown)
        .map(|capture| capture[1].trim().to_owned())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Proposed plan".into());
    let lines: Vec<_> = markdown
        .trim_end()
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    let mut start = usize::from(
        lines
            .first()
            .is_some_and(|line| FIRST_HEADING.is_match(line)),
    );
    while lines.get(start).is_some_and(|line| line.trim().is_empty()) {
        start += 1;
    }
    if lines.get(start).is_some_and(|line| {
        HEADING
            .captures(line)
            .is_some_and(|capture| capture[1].trim().eq_ignore_ascii_case("summary"))
    }) {
        start += 1;
        while lines.get(start).is_some_and(|line| line.trim().is_empty()) {
            start += 1;
        }
    }
    (title, lines[start..].join("\n"))
}

fn fold_apostrophes_with_map(src: &str) -> (String, Vec<usize>) {
    let mut folded = String::with_capacity(src.len());
    let mut map = Vec::with_capacity(src.len() + 1);
    for (byte_idx, ch) in src.char_indices() {
        if ch == '\u{2018}' || ch == '\u{2019}' {
            folded.push('\'');
            map.push(byte_idx);
        } else {
            let start = folded.len();
            folded.push(ch);
            for offset in 0..(folded.len() - start) {
                map.push(byte_idx + offset);
            }
        }
    }
    map.push(src.len());
    (folded, map)
}

fn fold_query_apostrophes(query: &str) -> String {
    if !query.contains(['\u{2018}', '\u{2019}']) {
        return query.to_owned();
    }
    query
        .chars()
        .map(|c| {
            if c == '\u{2018}' || c == '\u{2019}' {
                '\''
            } else {
                c
            }
        })
        .collect()
}

/// Rendered-text Markdown find matching KSG's `threadSearch` (case-insensitive,
/// Greek final sigma `'ς'` -> `'σ'`, curly apostrophes `'‘'`/`'’'` -> `'\''`,
/// skipping inline formatting markers and link destinations).
pub fn find_markdown_occurrences(src: &str, query: &str) -> Vec<Vec<Range<usize>>> {
    let query = query.trim();
    if src.is_empty() || query.is_empty() {
        return Vec::new();
    }
    let folded_query = fold_query_apostrophes(query);
    if !src.contains(['\u{2018}', '\u{2019}']) {
        return rune_kit::markdown::find(src, &folded_query, rune_kit::markdown::Case::Insensitive);
    }
    let (folded_src, map) = fold_apostrophes_with_map(src);
    let hits = rune_kit::markdown::find(
        &folded_src,
        &folded_query,
        rune_kit::markdown::Case::Insensitive,
    );
    hits.into_iter()
        .map(|ranges| {
            ranges
                .into_iter()
                .map(|r| map[r.start]..map[r.end])
                .collect()
        })
        .collect()
}

fn fold_char_for_plain(c: char, out: &mut String) {
    if c == 'ς' {
        out.push('σ');
    } else if c == '\u{2018}' || c == '\u{2019}' {
        out.push('\'');
    } else {
        out.extend(c.to_lowercase());
    }
}

/// Case-insensitive plain-text occurrence scan with byte offsets into `src`.
pub fn find_plain_occurrences(src: &str, query: &str) -> Vec<Range<usize>> {
    let query = query.trim();
    if src.is_empty() || query.is_empty() {
        return Vec::new();
    }
    let mut folded_query = String::with_capacity(query.len());
    for c in query.chars() {
        fold_char_for_plain(c, &mut folded_query);
    }
    if folded_query.is_empty() {
        return Vec::new();
    }
    let mut folded_src = String::with_capacity(src.len());
    let mut byte_map = Vec::with_capacity(src.len() + 1);
    for (byte_idx, ch) in src.char_indices() {
        let before = folded_src.len();
        fold_char_for_plain(ch, &mut folded_src);
        let added = folded_src.len() - before;
        for i in 0..added {
            byte_map.push(if i == 0 { byte_idx } else { byte_idx });
        }
    }
    byte_map.push(src.len());

    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor + folded_query.len() <= folded_src.len() {
        let Some(pos) = folded_src[cursor..].find(&folded_query) else {
            break;
        };
        let start = cursor + pos;
        let end = start + folded_query.len();
        // Map back to valid char boundaries in src.
        let src_start = byte_map[start];
        let mut src_end = byte_map[end];
        if src_end <= src_start {
            // Character expanded on lowercase; find end of that char.
            src_end = src[src_start..]
                .chars()
                .next()
                .map_or(src.len(), |c| src_start + c.len_utf8());
        }
        out.push(src_start..src_end);
        cursor = end;
    }
    out
}

pub fn count_document_occurrences(role: &str, text: &str, query: &str) -> usize {
    if role == "plan" {
        let (title, body) = plan_parts(text);
        find_plain_occurrences(&title, query).len() + find_markdown_occurrences(&body, query).len()
    } else if role == "Shina" {
        find_plain_occurrences(text, query).len()
    } else {
        find_markdown_occurrences(text, query).len()
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    #[test]
    fn plans_use_the_displayed_title_and_body_without_a_hidden_summary_heading() {
        assert_eq!(
            plan_parts("# Title\n\n## Summary\n\nBody\n"),
            ("Title".into(), "Body".into())
        );
        assert_eq!(plan_parts("Body"), ("Proposed plan".into(), "Body".into()));
        assert_eq!(
            count_document_occurrences("plan", "Body", "proposed plan"),
            1
        );
        assert_eq!(
            count_document_occurrences("plan", "# Title\n\nBody", "Title Body"),
            0
        );
        for key in ["markdown", "text", "plan"] {
            let mut input = serde_json::json!({});
            input[key] = serde_json::json!("# Title\n\nBody");
            let tool = serde_json::json!({"kind":"plan","input":input});
            assert_eq!(proposed_plan_markdown(&tool), Some("# Title\n\nBody"));
        }
        assert!(
            proposed_plan_markdown(&serde_json::json!({
                "kind":"plan","input":{"steps":[{"text":"Todo"}]}
            }))
            .is_none()
        );
    }
}

pub fn search_documents(
    documents: &[SearchDocument<'_>],
    query: &str,
    start: Option<&ThreadFindStart>,
    offset: i64,
    index: Option<usize>,
    snapshot_sequence: u64,
) -> ThreadFindResult {
    let query = query.trim();
    let query: String = query.chars().take(THREAD_FIND_QUERY_MAX_LENGTH).collect();
    if query.is_empty() {
        return ThreadFindResult {
            snapshot_sequence,
            total_matches: 0,
            active_index: 0,
            navigation: Vec::new(),
            active_match: None,
        };
    }

    let counted: Vec<(&str, usize)> = documents
        .iter()
        .map(|doc| {
            (
                doc.entry_id,
                count_document_occurrences(doc.role, doc.text, &query),
            )
        })
        .collect();
    let total_matches: usize = counted.iter().map(|(_, count)| *count).sum();

    let mut requested_index = index.unwrap_or(0);
    if index.is_none()
        && let Some(start) = start
        && let Some(start_pos) = counted.iter().position(|(id, _)| *id == start.entry_id)
    {
        let prior: usize = counted[..start_pos].iter().map(|(_, c)| *c).sum();
        requested_index = prior + start.occurrence.min(counted[start_pos].1);
        if requested_index >= total_matches {
            requested_index = 0;
        }
    }

    let active_index = if total_matches == 0 {
        0
    } else if index.is_none() {
        ((requested_index as i64 + offset).rem_euclid(total_matches as i64)) as usize
    } else {
        requested_index.min(total_matches - 1)
    };

    let mut remaining = active_index;
    let mut selected: Option<ThreadFindMatch> = None;
    for &(entry_id, count) in &counted {
        if count == 0 {
            continue;
        }
        if remaining < count {
            selected = Some(ThreadFindMatch {
                entry_id: entry_id.to_owned(),
                occurrence: remaining,
            });
            break;
        }
        remaining -= count;
    }

    let mut start_index = 0usize;
    let mut entries = Vec::new();
    for &(entry_id, count) in &counted {
        if count > 0 {
            entries.push(ThreadFindNavEntry {
                entry_id: entry_id.to_owned(),
                count,
                start_index,
            });
            start_index += count;
        }
    }

    let selected_entry = selected
        .as_ref()
        .and_then(|m| entries.iter().position(|e| e.entry_id == m.entry_id))
        .unwrap_or(0);
    let navigation = if entries.len() <= 17 {
        entries
    } else {
        (0..17)
            .map(|i| {
                let idx = (selected_entry + i + entries.len() - 8) % entries.len();
                entries[idx].clone()
            })
            .collect()
    };

    ThreadFindResult {
        snapshot_sequence,
        total_matches,
        active_index,
        navigation,
        active_match: selected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_matches_markdown_case_insensitively_folds_apostrophes_and_skips_formatting() {
        let src = "First **target phrase** with `target` and [link label](https://target.example) and Bob’s code.";
        let hits = find_markdown_occurrences(src, "target phrase");
        assert_eq!(hits.len(), 1);
        let word_hits = find_markdown_occurrences(src, "target");
        // Must match in "**target phrase**" and "`target`", NOT in the URL "https://target.example".
        assert_eq!(word_hits.len(), 2);
        let apostrophe_hits = find_markdown_occurrences(src, "Bob's");
        assert_eq!(apostrophe_hits.len(), 1);
        let sigma = find_markdown_occurrences("τελος", "ΤΕΛΟΣ");
        assert_eq!(sigma.len(), 1);
        let sigma_accented = find_markdown_occurrences("τέλος", "ΤΈΛΟΣ");
        assert_eq!(sigma_accented.len(), 1);
    }

    #[test]
    fn thread_find_orders_chronologically_wraps_around_and_builds_17_entry_navigation_window() {
        let texts: Vec<String> = (0..25)
            .map(|i| format!("Message {i} with needle inside"))
            .collect();
        let docs: Vec<SearchDocument<'_>> = texts
            .iter()
            .enumerate()
            .map(|(i, text)| SearchDocument {
                entry_id: Box::leak(format!("e-{i}").into_boxed_str()),
                role: if i % 2 == 0 { "You" } else { "Pi" },
                text,
            })
            .collect();

        let first = search_documents(&docs, "needle", None, 0, None, 10);
        assert_eq!(first.total_matches, 25);
        assert_eq!(first.active_index, 0);
        assert_eq!(first.active_match.as_ref().unwrap().entry_id, "e-0");
        assert_eq!(first.navigation.len(), 17);

        // Stepping backwards from 0 wraps to last (24).
        let prev = search_documents(
            &docs,
            "needle",
            Some(&ThreadFindStart {
                entry_id: "e-0".into(),
                occurrence: 0,
            }),
            -1,
            None,
            10,
        );
        assert_eq!(prev.active_index, 24);
        assert_eq!(prev.active_match.as_ref().unwrap().entry_id, "e-24");

        // Stepping forwards from 24 wraps to 0.
        let next = search_documents(
            &docs,
            "needle",
            Some(&ThreadFindStart {
                entry_id: "e-24".into(),
                occurrence: 0,
            }),
            1,
            None,
            10,
        );
        assert_eq!(next.active_index, 0);
        assert_eq!(next.active_match.as_ref().unwrap().entry_id, "e-0");
    }
}
