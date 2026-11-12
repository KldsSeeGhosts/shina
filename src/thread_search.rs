//! Ported from KSG Code's `ThreadSearch.ts` and `packages/shared/src/paletteSearch.ts`.
use serde::{Deserialize, Serialize};
use std::ops::Range;

pub const THREAD_SEARCH_QUERY_MAX_LENGTH: usize = 200;
pub const THREAD_SEARCH_DEBOUNCE_MS: u64 = 120;
pub const MESSAGE_TOKEN_MIN_LENGTH: usize = 3;
pub const MESSAGE_WORD_START_LENGTH: usize = 4;
pub const SNIPPET_LENGTH: usize = 240;
pub const SNIPPET_LEAD: usize = 24;
pub const MESSAGE_HIT_LIMIT: usize = 2000;
pub const MESSAGE_PREFIX: f64 = 0.8;
pub const MID_WORD: f64 = 0.3;
pub const MESSAGE_PHRASE_BONUS: f64 = 0.25;
pub const MESSAGE_NEAR_BONUS: f64 = 0.1;
pub const MESSAGE_NEAR_SPAN: usize = 60;

#[derive(Clone, Debug, PartialEq)]
pub struct MessageSearchTokens {
    pub long: Vec<String>,
    pub short: Vec<String>,
    pub words: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageMatchGrade {
    pub quality: f64,
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchHit {
    pub thread_id: usize,
    pub project_id: Option<usize>,
    pub entry_id: String,
    pub source: String,
    pub snippet: String,
    pub quality: f64,
    pub hits: usize,
    pub updated: u64,
}

fn is_cjk(c: char) -> bool {
    let code = c as u32;
    (0x3040..=0x30ff).contains(&code)
        || (0x3400..=0x4dbf).contains(&code)
        || (0x4e00..=0x9fff).contains(&code)
        || (0xf900..=0xfaff).contains(&code)
}

fn has_cjk_word(token: &str) -> bool {
    token.chars().any(|c| {
        let code = c as u32;
        is_cjk(c) || (0xac00..=0xd7af).contains(&code)
    })
}

fn is_symbol(c: char) -> bool {
    let code = c as u32;
    (0x2600..=0x27bf).contains(&code) || (0x1f300..=0x1faff).contains(&code)
}

fn strip_latin_diacritic(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'a',
        'ç' | 'Ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
        'ñ' | 'Ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'u',
        'ý' | 'ÿ' | 'Ý' => 'y',
        _ => c,
    }
}

fn fold_char(c: char) -> char {
    let base = strip_latin_diacritic(c);
    if base == 'ς' {
        'σ'
    } else if base == '\u{2018}' || base == '\u{2019}' {
        '\''
    } else {
        base.to_lowercase().next().unwrap_or(base)
    }
}

fn fold_text_chars(text: &str) -> (String, Vec<usize>) {
    let mut folded = String::with_capacity(text.len());
    let mut byte_map = Vec::with_capacity(text.len() + 1);
    for (byte_idx, c) in text.char_indices() {
        let fc = fold_char(c);
        let before = folded.len();
        folded.push(fc);
        for _ in 0..(folded.len() - before) {
            byte_map.push(byte_idx);
        }
    }
    byte_map.push(text.len());
    (folded, byte_map)
}

fn fold_query_token(token: &str) -> String {
    token.chars().map(fold_char).collect()
}

fn trim_wrapping_quotes(query: &str) -> &str {
    query
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim()
}

pub fn message_search_tokens(query: &str) -> MessageSearchTokens {
    let cleaned = trim_wrapping_quotes(query);
    let words: Vec<String> = cleaned
        .split_whitespace()
        .map(|w| {
            w.chars()
                .map(|c| {
                    if c == '\u{2018}' || c == '\u{2019}' {
                        '\''
                    } else if c == 'ς' {
                        'σ'
                    } else {
                        c.to_lowercase().next().unwrap_or(c)
                    }
                })
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let searchable = |token: &str| {
        let len = token.chars().count();
        len >= MESSAGE_TOKEN_MIN_LENGTH || (len == 2 && has_cjk_word(token))
    };
    let long = words.iter().filter(|t| searchable(t)).cloned().collect();
    let short = words.iter().filter(|t| !searchable(t)).cloned().collect();
    MessageSearchTokens { long, short, words }
}

/// Replace `!?[label](url)` markdown links with `label` so URLs are not matched.
pub fn strip_markdown_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let is_img = bytes[i] == b'!' && bytes.get(i + 1) == Some(&b'[');
        if bytes[i] == b'[' || is_img {
            let label_start = i + if is_img { 2 } else { 1 };
            if let Some(close_rel) = text[label_start..].find(']') {
                let label_end = label_start + close_rel;
                if !text[label_start..label_end].contains('\n')
                    && bytes.get(label_end + 1) == Some(&b'(')
                    && let Some(paren_rel) = text[label_end + 2..].find(')')
                {
                    let url = &text[label_end + 2..label_end + 2 + paren_rel];
                    if !url.contains(char::is_whitespace) {
                        out.push_str(&text[label_start..label_end]);
                        i = label_end + 2 + paren_rel + 1;
                        continue;
                    }
                }
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn is_word_start_chars(
    prev2: Option<char>,
    prev: Option<char>,
    ch: char,
    next: Option<char>,
) -> bool {
    if !ch.is_alphanumeric() && !is_symbol(ch) {
        return false;
    }
    if prev.is_some_and(|p| p == '\'' || p == '’') && prev2.is_some_and(char::is_alphanumeric) {
        return false;
    }
    match prev {
        None => true,
        Some(p) if !p.is_alphanumeric() => true,
        Some(p) if p.is_lowercase() && ch.is_uppercase() => true,
        Some(p)
            if p.is_uppercase() && ch.is_uppercase() && next.is_some_and(char::is_lowercase) =>
        {
            true
        }
        Some(p) if p.is_numeric() != ch.is_numeric() => true,
        _ => is_cjk(ch),
    }
}

fn starts_word_at(chars: &[char], idx: usize) -> bool {
    let ch = chars[idx];
    if !ch.is_alphanumeric() && !is_symbol(ch) {
        return true;
    }
    let prev2 = idx.checked_sub(2).map(|i| chars[i]);
    let prev = idx.checked_sub(1).map(|i| chars[i]);
    let next = chars.get(idx + 1).copied();
    is_word_start_chars(prev2, prev, ch, next)
}

fn ends_word_at(chars: &[char], end: usize) -> bool {
    end >= chars.len() || !chars[end].is_alphanumeric() || starts_word_at(chars, end)
}

fn is_hex_id(s: &[char]) -> bool {
    s.len() >= 7 && s.iter().all(|c| c.is_ascii_hexdigit()) && s.iter().any(|c| c.is_ascii_digit())
}

fn find_message_token(
    src_chars: &[char],
    folded_chars: &[char],
    token_chars: &[char],
    byte_offsets: &[usize],
) -> Option<(Vec<usize>, f64)> {
    if token_chars.is_empty() || token_chars.len() > folded_chars.len() {
        return None;
    }
    let mut best_quality = 0.0f64;
    let mut positions = Vec::new();
    let mut run_end = 0usize;
    let mut run_hex = false;
    let mut run_start = 0usize;
    for idx in 0..=folded_chars.len() - token_chars.len() {
        if &folded_chars[idx..idx + token_chars.len()] != token_chars {
            continue;
        }
        if idx + token_chars.len() > run_end {
            let mut s = idx;
            while s > 0 && src_chars[s - 1].is_alphanumeric() {
                s -= 1;
            }
            let mut e = idx + token_chars.len();
            while e < src_chars.len() && src_chars[e].is_alphanumeric() {
                e += 1;
            }
            run_start = s;
            run_end = e;
            run_hex = is_hex_id(&src_chars[s..e]);
        }
        if run_hex && run_start < idx {
            continue;
        }
        let q = if !starts_word_at(src_chars, idx) {
            MID_WORD
        } else if ends_word_at(src_chars, idx + token_chars.len()) {
            1.0
        } else {
            MESSAGE_PREFIX
        };
        let byte_pos = byte_offsets[idx];
        if q > best_quality {
            best_quality = q;
            positions.clear();
            positions.push(byte_pos);
        } else if (q - best_quality).abs() < f64::EPSILON {
            positions.push(byte_pos);
        }
    }
    (!positions.is_empty()).then_some((positions, best_quality))
}

fn find_phrase_offset(
    src_chars: &[char],
    folded_chars: &[char],
    byte_offsets: &[usize],
    words: &[String],
) -> Option<usize> {
    if words.len() <= 1 {
        return None;
    }
    let word_chars: Vec<Vec<char>> = words
        .iter()
        .map(|w| fold_query_token(w).chars().collect())
        .collect();
    if word_chars.iter().any(Vec::is_empty) {
        return None;
    }
    let first = &word_chars[0];
    if first.len() > folded_chars.len() {
        return None;
    }
    for idx in 0..=folded_chars.len() - first.len() {
        if &folded_chars[idx..idx + first.len()] != first.as_slice() {
            continue;
        }
        if idx > 0 && src_chars[idx - 1].is_alphanumeric() {
            continue;
        }
        let mut cursor = idx + first.len();
        let mut matched = true;
        for w in &word_chars[1..] {
            let ws_start = cursor;
            while cursor < folded_chars.len() && folded_chars[cursor].is_whitespace() {
                cursor += 1;
            }
            if cursor == ws_start || cursor + w.len() > folded_chars.len() {
                matched = false;
                break;
            }
            if &folded_chars[cursor..cursor + w.len()] != w.as_slice() {
                matched = false;
                break;
            }
            cursor += w.len();
        }
        if matched && (cursor >= src_chars.len() || !src_chars[cursor].is_alphanumeric()) {
            return Some(byte_offsets[idx]);
        }
    }
    None
}

fn shortest_span(places: &[Vec<usize>]) -> (usize, usize) {
    let mut hits: Vec<(usize, usize)> = places
        .iter()
        .enumerate()
        .flat_map(|(w, at)| at.iter().map(move |&pos| (pos, w)))
        .collect();
    hits.sort_by_key(|&(pos, _)| pos);
    if hits.is_empty() {
        return (0, usize::MAX);
    }
    let mut counts = vec![0usize; places.len()];
    let mut covered = 0usize;
    let mut best = (hits[0].0, usize::MAX);
    let mut left = 0usize;
    for &(pos, word) in &hits {
        counts[word] += 1;
        if counts[word] == 1 {
            covered += 1;
        }
        while covered == places.len() {
            let first = hits[left];
            if pos - first.0 < best.1 {
                best = (first.0, pos - first.0);
            }
            counts[first.1] -= 1;
            if counts[first.1] == 0 {
                covered -= 1;
            }
            left += 1;
        }
    }
    best
}

pub fn grade_message(text: &str, tokens: &MessageSearchTokens) -> Option<MessageMatchGrade> {
    if tokens.long.is_empty() {
        return None;
    }
    let src_chars: Vec<char> = text.chars().collect();
    let mut byte_offsets: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    byte_offsets.push(text.len());
    let folded_chars: Vec<char> = src_chars.iter().map(|&c| fold_char(c)).collect();

    let mut quality = 1.0f64;
    let mut positions = Vec::with_capacity(tokens.words.len());
    for token in &tokens.words {
        let folded_tok: Vec<char> = fold_query_token(token).chars().collect();
        let must_start_word = token.chars().count() < MESSAGE_WORD_START_LENGTH;
        let grades = tokens.long.contains(token);
        let (at, q) = find_message_token(&src_chars, &folded_chars, &folded_tok, &byte_offsets)?;
        if q == MID_WORD && must_start_word {
            return None;
        }
        if grades {
            quality = quality.min(q);
        }
        positions.push(at);
    }
    if let Some(offset) =
        find_phrase_offset(&src_chars, &folded_chars, &byte_offsets, &tokens.words)
    {
        return Some(MessageMatchGrade {
            quality: quality + MESSAGE_PHRASE_BONUS,
            offset,
        });
    }
    let (near_start, near_len) = shortest_span(&positions);
    if tokens.long.len() > 1 && near_len <= MESSAGE_NEAR_SPAN {
        quality += MESSAGE_NEAR_BONUS;
    }
    Some(MessageMatchGrade {
        quality,
        offset: near_start,
    })
}

pub fn build_search_snippet(text: &str, match_start: usize) -> String {
    let mut start = match_start.min(text.len());
    let lead_chars = text[..start]
        .chars()
        .rev()
        .take(SNIPPET_LEAD)
        .map(|c| c.len_utf8())
        .sum::<usize>();
    start = start.saturating_sub(lead_chars);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    if start > 0
        && let Some(ws) = text[start..match_start.min(text.len())].find(char::is_whitespace)
    {
        start += ws + text[start + ws..].chars().next().map_or(1, char::len_utf8);
    }
    let raw: String = text[start..].chars().take(SNIPPET_LENGTH * 2).collect();
    let raw_end = start + raw.len();
    let cleaned = raw.replace("**", "").replace('`', "");
    let collapsed: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut shown: String = collapsed.chars().take(SNIPPET_LENGTH - 2).collect();
    let cut = shown.len() < collapsed.len() || raw_end < text.len();
    if start > 0 {
        shown = format!("…{shown}");
    }
    if cut {
        shown.push('…');
    }
    shown
}

pub fn find_snippet_token_ranges(snippet: &str, query: &str) -> Vec<Range<usize>> {
    let tokens = message_search_tokens(query);
    if tokens.long.is_empty() || snippet.is_empty() {
        return Vec::new();
    }
    let src_chars: Vec<char> = snippet.chars().collect();
    let mut byte_offsets: Vec<usize> = snippet.char_indices().map(|(b, _)| b).collect();
    byte_offsets.push(snippet.len());
    let folded_chars: Vec<char> = src_chars.iter().map(|&c| fold_char(c)).collect();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for token in &tokens.long {
        let tok: Vec<char> = fold_query_token(token).chars().collect();
        if tok.is_empty() || tok.len() > folded_chars.len() {
            continue;
        }
        let needs_word_start = token.chars().count() < MESSAGE_WORD_START_LENGTH;
        let own_boundary = tok
            .first()
            .is_some_and(|c| !c.is_alphanumeric() && !is_symbol(*c));
        let mut idx = 0;
        while idx + tok.len() <= folded_chars.len() {
            if &folded_chars[idx..idx + tok.len()] == tok.as_slice() {
                if !needs_word_start || own_boundary || starts_word_at(&src_chars, idx) {
                    ranges.push(byte_offsets[idx]..byte_offsets[idx + tok.len()]);
                    idx += tok.len();
                    continue;
                }
            }
            idx += 1;
        }
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in ranges {
        if let Some(last) = merged.last_mut()
            && r.start <= last.end
        {
            last.end = last.end.max(r.end);
        } else {
            merged.push(r);
        }
    }
    merged
}

/// Ported from KSG `threadFindStore.ts::findForMessageMatch`.
pub fn find_for_message_match(
    snippet: &str,
    entry_id: &str,
    query: &str,
) -> (String, Option<crate::thread_find::ThreadFindStart>) {
    let start = (!entry_id.is_empty()).then(|| crate::thread_find::ThreadFindStart {
        entry_id: entry_id.to_owned(),
        occurrence: 0,
    });
    let tokens = message_search_tokens(query);
    let src_chars: Vec<char> = snippet.chars().collect();
    let mut byte_offsets: Vec<usize> = snippet.char_indices().map(|(b, _)| b).collect();
    byte_offsets.push(snippet.len());
    let folded_chars: Vec<char> = src_chars.iter().map(|&c| fold_char(c)).collect();
    if let Some(offset) =
        find_phrase_offset(&src_chars, &folded_chars, &byte_offsets, &tokens.words)
    {
        // Reconstruct matched phrase slice up to the end of the last word.
        let mut end = offset;
        let char_pos = byte_offsets.iter().position(|&b| b == offset).unwrap_or(0);
        let mut cursor = char_pos;
        for (idx, w) in tokens.words.iter().enumerate() {
            if idx > 0 {
                while cursor < folded_chars.len() && folded_chars[cursor].is_whitespace() {
                    cursor += 1;
                }
            }
            cursor = (cursor + w.chars().count()).min(src_chars.len());
            end = byte_offsets[cursor];
        }
        return (snippet[offset..end].to_owned(), start);
    }
    let word = tokens
        .long
        .iter()
        .max_by_key(|t| t.chars().count())
        .map(String::as_str)
        .unwrap_or(query.trim());
    let ranges = find_snippet_token_ranges(snippet, word);
    let chosen = ranges
        .iter()
        .find(|r| {
            !snippet[r.end..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
        })
        .or_else(|| ranges.first());
    let q = chosen
        .map(|r| snippet[r.clone()].to_owned())
        .unwrap_or_else(|| word.to_owned());
    (q, start)
}

pub fn rank_message_matches(matches: &mut [ThreadSearchHit]) {
    let weight = |hits: usize| 1.0 + (1.0 + hits as f64).log2();
    matches.sort_by(|left, right| {
        let left_word = left.quality >= MESSAGE_PREFIX;
        let right_word = right.quality >= MESSAGE_PREFIX;
        right_word
            .cmp(&left_word)
            .then_with(|| {
                right
                    .quality
                    .partial_cmp(&left.quality)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                weight(right.hits)
                    .partial_cmp(&weight(left.hits))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| right.updated.cmp(&left.updated))
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
}

#[allow(dead_code)]
pub fn fold_for_title(text: &str) -> String {
    fold_text_chars(text).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_matcher_follow_ksg_rules_including_word_starts_hex_ids_and_phrases() {
        let tokens = message_search_tokens("  \"fts cache\"  ");
        assert_eq!(tokens.long, vec!["fts", "cache"]);
        assert!(tokens.short.is_empty());

        // 3-char token "fts" must start a word: fails inside "drafts", matches "fts5".
        let fts_only = message_search_tokens("fts");
        assert!(grade_message("searching drafts", &fts_only).is_none());
        assert!(grade_message("using fts5 index", &fts_only).is_some());

        // Hex IDs >= 7 chars ignore internal matches, e.g. "cafe" inside "7cafe52bb8".
        let cafe = message_search_tokens("cafe");
        assert!(grade_message("commit 7cafe52bb8", &cafe).is_none());
        assert!(grade_message("visit Café today", &cafe).is_some());

        // Phrase bonus > near bonus > far apart.
        let phrase = message_search_tokens("lakebed perf");
        let g_phrase = grade_message("tuning lakebed perf today", &phrase).unwrap();
        let g_near = grade_message("tuning lakebed storage perf today", &phrase).unwrap();
        let far_text = format!("lakebed {} perf", "filler ".repeat(20));
        let g_far = grade_message(&far_text, &phrase).unwrap();
        assert!(g_phrase.quality > g_near.quality);
        assert!(g_near.quality > g_far.quality);

        // Markdown link URLs are stripped before matching.
        let url_only = message_search_tokens("secreturl");
        let stripped = strip_markdown_links("See [this doc](https://secreturl.example/path)");
        assert!(grade_message(&stripped, &url_only).is_none());
    }

    #[test]
    fn snippet_strips_markdown_markers_and_leads_match() {
        let text = format!(
            "{} Look at **secret** `needle` in the middle of a very long paragraph {}",
            "prefix ".repeat(15),
            "suffix ".repeat(40)
        );
        let tokens = message_search_tokens("secret needle");
        let grade = grade_message(&text, &tokens).unwrap();
        let snippet = build_search_snippet(&text, grade.offset);
        assert!(snippet.starts_with('…'));
        assert!(snippet.ends_with('…'));
        assert!(snippet.contains("secret needle"));
        assert!(!snippet.contains("**"));
        assert!(!snippet.contains('`'));
        assert!(snippet.chars().count() <= SNIPPET_LENGTH);
    }

    #[test]
    fn ranking_prefers_whole_word_and_higher_hit_counts() {
        let mut hits = vec![
            ThreadSearchHit {
                thread_id: 1,
                project_id: None,
                entry_id: "m1".into(),
                source: "user".into(),
                snippet: "midword".into(),
                quality: MID_WORD,
                hits: 10,
                updated: 500,
            },
            ThreadSearchHit {
                thread_id: 2,
                project_id: None,
                entry_id: "m2".into(),
                source: "user".into(),
                snippet: "one hit".into(),
                quality: 1.0,
                hits: 1,
                updated: 400,
            },
            ThreadSearchHit {
                thread_id: 3,
                project_id: None,
                entry_id: "m3".into(),
                source: "assistant".into(),
                snippet: "many hits".into(),
                quality: 1.0,
                hits: 5,
                updated: 300,
            },
        ];
        rank_message_matches(&mut hits);
        assert_eq!(hits[0].thread_id, 3);
        assert_eq!(hits[1].thread_id, 2);
        assert_eq!(hits[2].thread_id, 1);
    }
}
