//! Composer editing rules, ported from KSG Code's chat/composerCommandKey.ts,
//! composerSubmission.ts, and composer-logic.ts.
use rune::{Key, KeyEvent, TextEdit};

pub const PROVIDER_SEND_TURN_MAX_INPUT_CHARS: usize = 120_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SendShortcut {
    #[default]
    Enter,
    ModEnterMultiline,
    ModEnter,
}

impl SendShortcut {
    pub const ALL: [Self; 3] = [Self::Enter, Self::ModEnterMultiline, Self::ModEnter];

    pub fn label(self) -> String {
        let mod_label = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl"
        };
        match self {
            Self::Enter => "Enter".to_owned(),
            Self::ModEnterMultiline => format!("{mod_label} + Enter for multiline prompts"),
            Self::ModEnter => format!("{mod_label} + Enter always"),
        }
    }
}

pub fn requires_modifier(send_shortcut: SendShortcut, prompt: &str) -> bool {
    match send_shortcut {
        SendShortcut::Enter => false,
        SendShortcut::ModEnter => true,
        SendShortcut::ModEnterMultiline => prompt.contains('\n') || prompt.contains('\r'),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionIntent {
    Foreground,
    Background,
    Alternate,
}

#[cfg(test)]
pub fn submission_intent_for_key(
    event: &KeyEvent,
    key_repeat: bool,
    is_composing: bool,
    is_running: bool,
    is_draft_thread: bool,
    send_shortcut: SendShortcut,
    prompt: &str,
) -> Option<SubmissionIntent> {
    let mut ctx = crate::keybindings::ShortcutContext::desktop();
    ctx.composer_focus = true;
    ctx.draft_thread_route = is_draft_thread;
    ctx.turn_running = is_running;
    ctx.composer_requires_modifier = requires_modifier(send_shortcut, prompt);
    let defaults = crate::keybindings::default_resolved_keybindings();
    let command = crate::keybindings::resolve_shortcut_command(
        event,
        &defaults,
        &ctx,
        crate::keybindings::Platform::current(),
        None,
        None,
    );
    submission_intent_for_command(
        event,
        key_repeat,
        is_composing,
        is_running,
        is_draft_thread,
        send_shortcut,
        prompt,
        command,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn submission_intent_for_command(
    event: &KeyEvent,
    key_repeat: bool,
    is_composing: bool,
    is_running: bool,
    is_draft_thread: bool,
    send_shortcut: SendShortcut,
    prompt: &str,
    command: Option<&str>,
) -> Option<SubmissionIntent> {
    if is_composing || key_repeat {
        return None;
    }
    if command == Some("composer.sendAlternate") && is_running {
        return Some(SubmissionIntent::Alternate);
    }
    if command == Some("composer.sendBackground") && is_draft_thread {
        return Some(SubmissionIntent::Background);
    }
    if command == Some("composer.sendAndNewThread") && !is_draft_thread {
        return Some(SubmissionIntent::Background);
    }
    if command.is_some() {
        return None;
    }
    if event.key != Key::Enter || event.mods.shift || event.mods.alt {
        return None;
    }
    let has_mod = event.mods.cmd || event.mods.ctrl;
    if requires_modifier(send_shortcut, prompt) && !has_mod {
        return None;
    }
    Some(SubmissionIntent::Foreground)
}

pub fn format_count_en_us(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Ported from KSG Code's `getComposerPromptLengthValidationMessage`.
pub fn prompt_length_validation_message(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    let len = trimmed.encode_utf16().count();
    let excess = len.saturating_sub(PROVIDER_SEND_TURN_MAX_INPUT_CHARS);
    if excess == 0 {
        return None;
    }
    let unit = if excess == 1 {
        "character"
    } else {
        "characters"
    };
    Some(format!(
        "Prompt is {} {unit} over the {}-character limit. Shorten or split it before sending.",
        format_count_en_us(excess),
        format_count_en_us(PROVIDER_SEND_TURN_MAX_INPUT_CHARS),
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub marker: char,
    pub range: std::ops::Range<usize>,
    pub query: String,
}

/// KSG Code's composer trigger grammar. Work in UTF-8 caret bytes, never character indices.
pub fn trigger(edit: &TextEdit) -> Option<Trigger> {
    if edit.anchor != edit.focus || edit.is_composing() || edit.active_block() == rune::Block::Code
    {
        return None;
    }
    let before = edit.text.get(..edit.focus)?;
    let start = before
        .rfind(char::is_whitespace)
        .map_or(0, |i| i + before[i..].chars().next().unwrap().len_utf8());
    let token = &before[start..];
    let marker = token.chars().next()?;
    if !matches!(marker, '/' | '@' | '$')
        || edit
            .pills()
            .any(|(range, _)| range.start < edit.focus && range.end > start)
    {
        return None;
    }
    Some(Trigger {
        marker,
        range: start..edit.focus,
        query: token[1..].to_owned(),
    })
}

const ATTACHMENT_ONLY_BOOTSTRAP_PROMPT: &str =
    "Please review the attached context before proceeding.";
const PLAN_IMPLEMENTATION_PROMPT_PREFIX: &str = "PLEASE IMPLEMENT THIS PLAN:\n";

/// Ported from KSG Code's `recallableComposerPrompt` and `buildComposerPromptHistoryEntries`.
pub fn recallable_prompt(message_text: &str) -> String {
    let mut prompt = message_text.trim();
    if let Some(rest) = prompt.strip_prefix("Ultrathink:\n") {
        prompt = rest;
    }
    let trimmed = prompt.trim();
    if trimmed == ATTACHMENT_ONLY_BOOTSTRAP_PROMPT
        || trimmed.starts_with(PLAN_IMPLEMENTATION_PROMPT_PREFIX)
    {
        return String::new();
    }
    trimmed.to_owned()
}

pub fn build_history_entries(messages: &[crate::model::Message]) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for message in messages {
        if message.role != "You" {
            continue;
        }
        let prompt = recallable_prompt(&message.text);
        if prompt.is_empty() {
            continue;
        }
        if entries.last().is_some_and(|prev| prev == &prompt) {
            continue;
        }
        entries.push(prompt);
    }
    entries
}

pub const COMPOSER_TYPING_IDLE_MS: u64 = 1500;

/// Ported from KSG Code's `composerTypingGuard.ts` and `useComposerTypingGuard.ts`.
#[derive(Default)]
pub struct TypingGuard {
    seen: std::collections::HashSet<String>,
    held: std::collections::HashSet<String>,
    last_typed_at: Option<std::time::Instant>,
}

impl TypingGuard {
    pub fn on_type(&mut self, now: std::time::Instant) {
        self.last_typed_at = Some(now);
    }

    pub fn on_blur_or_send(&mut self) {
        self.held.clear();
        self.last_typed_at = None;
    }

    pub fn on_requests(&mut self, request_ids: &[String], focused: bool, now: std::time::Instant) {
        let idle = std::time::Duration::from_millis(COMPOSER_TYPING_IDLE_MS);
        self.held.retain(|id| request_ids.contains(id));
        for id in request_ids {
            if !self.seen.contains(id)
                && focused
                && self
                    .last_typed_at
                    .is_some_and(|at| now.saturating_duration_since(at) < idle)
            {
                self.held.insert(id.clone());
            }
            self.seen.insert(id.clone());
        }
    }

    pub fn release_if_idle(&mut self, now: std::time::Instant) -> bool {
        if self.held.is_empty() {
            return false;
        }
        let idle = std::time::Duration::from_millis(COMPOSER_TYPING_IDLE_MS);
        if self
            .last_typed_at
            .is_none_or(|at| now.saturating_duration_since(at) >= idle)
        {
            self.held.clear();
            self.last_typed_at = None;
            return true;
        }
        false
    }

    pub fn is_held(&self, request_id: &str) -> bool {
        self.held.contains(request_id)
    }

    pub fn deadline(&self) -> Option<std::time::Instant> {
        if self.held.is_empty() {
            return None;
        }
        let at = self.last_typed_at?;
        Some(at + std::time::Duration::from_millis(COMPOSER_TYPING_IDLE_MS))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DismissedTrigger {
    pub marker: char,
    pub range_start: usize,
}

impl DismissedTrigger {
    pub fn matches(&self, trigger: &Trigger) -> bool {
        self.marker == trigger.marker && self.range_start == trigger.range.start
    }
}

/// Ported from KSG Code's `normalizeSearchQuery`, `scoreSubsequenceMatch`, `scoreQueryMatch`,
/// `scoreSlashCommandItem`, `searchSlashCommandItems`, and `scoreProviderSkill`.
pub fn normalize_search_query(input: &str, trim_leading_slashes: bool) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let stripped = if trim_leading_slashes {
        trimmed.trim_start_matches('/')
    } else {
        trimmed
    };
    stripped.to_lowercase()
}

pub fn score_subsequence_match(value: &str, query: &str, ignore_position: bool) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
    }
    let q_chars: Vec<char> = query.chars().collect();
    let v_chars: Vec<char> = value.chars().collect();
    let mut q_idx = 0usize;
    let mut first_match: Option<usize> = None;
    let mut prev_match: Option<usize> = None;
    let mut gap_penalty = 0usize;
    for (v_idx, &ch) in v_chars.iter().enumerate() {
        if ch != q_chars[q_idx] {
            continue;
        }
        if first_match.is_none() {
            first_match = Some(v_idx);
        }
        if let Some(prev) = prev_match {
            gap_penalty += v_idx - prev - 1;
        }
        prev_match = Some(v_idx);
        q_idx += 1;
        if q_idx == q_chars.len() {
            let first = first_match.unwrap_or(0);
            let span_penalty = v_idx - first + 1 - q_chars.len();
            if ignore_position {
                return Some(gap_penalty * 3 + span_penalty);
            }
            let length_penalty = (v_chars.len().saturating_sub(q_chars.len())).min(64);
            return Some(first * 2 + gap_penalty * 3 + span_penalty + length_penalty);
        }
    }
    None
}

fn find_boundary_match_index(value: &str, query: &str, markers: &[&str]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for marker in markers {
        let needle = format!("{marker}{query}");
        if let Some(byte_idx) = value.find(&needle) {
            let match_char_idx = value[..byte_idx].chars().count() + marker.chars().count();
            if best.is_none_or(|b| match_char_idx < b) {
                best = Some(match_char_idx);
            }
        }
    }
    best
}

#[allow(clippy::too_many_arguments)]
pub fn score_query_match(
    value: &str,
    query: &str,
    exact_base: usize,
    prefix_base: Option<usize>,
    boundary_base: Option<usize>,
    includes_base: Option<usize>,
    fuzzy_base: Option<usize>,
    boundary_markers: &[&str],
) -> Option<usize> {
    if value.is_empty() || query.is_empty() {
        return None;
    }
    if value == query {
        return Some(exact_base);
    }
    let val_len = value.chars().count();
    let q_len = query.chars().count();
    let penalty = val_len.saturating_sub(q_len).min(64);
    if let Some(base) = prefix_base
        && value.starts_with(query)
    {
        return Some(base + penalty);
    }
    if let Some(base) = boundary_base
        && let Some(boundary_idx) = find_boundary_match_index(value, query, boundary_markers)
    {
        return Some(base + boundary_idx * 2 + penalty);
    }
    if let Some(base) = includes_base
        && let Some(byte_idx) = value.find(query)
    {
        let char_idx = value[..byte_idx].chars().count();
        return Some(base + char_idx * 2 + penalty);
    }
    if let Some(base) = fuzzy_base
        && let Some(fuzzy_score) = score_subsequence_match(value, query, false)
    {
        return Some(base + fuzzy_score);
    }
    None
}

pub fn score_slash_command(
    name: &str,
    description: &str,
    query: &str,
    is_skill: bool,
) -> Option<usize> {
    if is_skill {
        if query == "skill" {
            return Some(0);
        }
        let skill_query = query.strip_prefix("skill:").unwrap_or(query);
        let skill_name = name.strip_prefix("skill:").unwrap_or(name).to_lowercase();
        let desc = description.to_lowercase();
        if skill_query.is_empty() {
            return Some(0);
        }
        let s1 = score_query_match(
            &skill_name,
            skill_query,
            0,
            Some(2),
            Some(4),
            Some(6),
            Some(100),
            &["-", "_", "/"],
        );
        let s2 = score_query_match(
            &desc,
            skill_query,
            30,
            Some(32),
            Some(34),
            Some(36),
            None,
            &[" ", "-", "_", "/"],
        );
        let best = [s1, s2].into_iter().flatten().min();
        if best.is_some() {
            return best;
        }
        return "skill".starts_with(query).then_some(usize::MAX);
    }

    let primary = name.to_lowercase();
    let desc = description.to_lowercase();
    let s1 = score_query_match(
        &primary,
        query,
        0,
        Some(2),
        Some(4),
        Some(6),
        Some(100),
        &["-", "_", "/"],
    );
    let s2 = score_query_match(
        &desc,
        query,
        20,
        Some(22),
        Some(24),
        Some(26),
        None,
        &[" ", "-", "_", "/"],
    );
    [s1, s2].into_iter().flatten().min()
}

pub fn rank_slash_commands(
    commands: Vec<crate::model::pi_session::SlashCommand>,
    query: &str,
) -> Vec<crate::model::pi_session::SlashCommand> {
    let normalized = normalize_search_query(query, true);
    if normalized.is_empty() {
        return commands;
    }
    let mut ranked: Vec<(usize, String, crate::model::pi_session::SlashCommand)> = commands
        .into_iter()
        .filter_map(|cmd| {
            let is_skill = cmd.name.starts_with("skill:") || cmd.source == "skill";
            let score = score_slash_command(&cmd.name, &cmd.description, &normalized, is_skill)?;
            let kind_rank = if cmd.source == "Shina" {
                '0'
            } else if !is_skill {
                '1'
            } else {
                '2'
            };
            let tie = format!("{kind_rank}\0{}\0{}", cmd.name, cmd.source);
            Some((score, tie, cmd))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    ranked.into_iter().map(|(_, _, cmd)| cmd).collect()
}

/// Per-thread browsing state. Keep the whole edit so recall also preserves the caret and marks.
#[derive(Default)]
pub struct History {
    position: Option<usize>,
    recalled: Option<String>,
    saved: Option<TextEdit>,
}

impl History {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn navigate(
        &mut self,
        edit: &mut TextEdit,
        entries: &[String],
        event: &KeyEvent,
        menu_open: bool,
        has_attachments: bool,
        has_pending_request: bool,
        caret_on_visual_edge: Option<bool>,
    ) -> bool {
        let m = event.mods;
        if menu_open
            || has_pending_request
            || has_attachments
            || m.shift
            || m.alt
            || m.cmd
            || m.ctrl
            || edit.is_composing()
            || edit.anchor != edit.focus
            || !matches!(event.key, Key::Up | Key::Down)
        {
            return false;
        }
        let caret = edit.focus.min(edit.text.len());
        let backward = event.key == Key::Up;
        if backward && edit.text[..caret].contains('\n')
            || !backward && edit.text[caret..].contains('\n')
        {
            return false;
        }
        if caret_on_visual_edge == Some(false) {
            return false;
        }
        // Check if we are currently in an active recalled position matching the current text.
        let active_index = self
            .position
            .filter(|_| self.recalled.as_deref() == Some(edit.text.as_str()))
            .filter(|&i| i < entries.len());
        if backward && active_index.is_none() && !edit.text.is_empty() {
            return false;
        }
        let next = if backward {
            let at = active_index.unwrap_or(entries.len());
            if at == 0 {
                return active_index.is_some();
            }
            at - 1
        } else {
            let Some(at) = active_index else {
                return false;
            };
            at + 1
        };
        if active_index.is_none() {
            self.saved = Some(edit.clone());
        }
        if next >= entries.len() {
            if let Some(saved) = self.saved.take() {
                *edit = saved;
            } else {
                edit.replace_text("");
            }
            self.reset();
        } else {
            edit.replace_text(&entries[next]);
            edit.focused = true;
            self.position = Some(next);
            self.recalled = Some(entries[next].clone());
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            mods: Default::default(),
        }
    }

    #[test]
    fn history_starts_only_from_empty_composer_without_attachments() {
        let mut history = History::default();
        let entries = vec!["first".to_owned(), "second".to_owned()];

        // Non-empty draft blocks starting history recall.
        let mut edit = TextEdit::new("unfinished");
        edit.set_caret(0);
        assert!(!history.navigate(
            &mut edit,
            &entries,
            &key(Key::Up),
            false,
            false,
            false,
            None
        ));
        assert_eq!(edit.text, "unfinished");

        // Attachments block starting history recall even when text is empty.
        let mut empty = TextEdit::new("");
        assert!(!history.navigate(
            &mut empty,
            &entries,
            &key(Key::Up),
            false,
            true,
            false,
            None
        ));

        // Pending request blocks history recall.
        assert!(!history.navigate(
            &mut empty,
            &entries,
            &key(Key::Up),
            false,
            false,
            true,
            None
        ));

        // Empty composer with no attachments recalls the latest entry, then Down restores empty.
        assert!(history.navigate(
            &mut empty,
            &entries,
            &key(Key::Up),
            false,
            false,
            false,
            None
        ));
        assert_eq!(empty.text, "second");
        assert!(history.navigate(
            &mut empty,
            &entries,
            &key(Key::Up),
            false,
            false,
            false,
            None
        ));
        assert_eq!(empty.text, "first");
        assert!(history.navigate(
            &mut empty,
            &entries,
            &key(Key::Down),
            false,
            false,
            false,
            None
        ));
        assert_eq!(empty.text, "second");
        assert!(history.navigate(
            &mut empty,
            &entries,
            &key(Key::Down),
            false,
            false,
            false,
            None
        ));
        assert_eq!(empty.text, "");
    }

    #[test]
    fn editing_a_recalled_prompt_exits_history_browsing() {
        let mut history = History::default();
        let mut edit = TextEdit::new("");
        let entries = vec!["sent".to_owned()];
        assert!(history.navigate(
            &mut edit,
            &entries,
            &key(Key::Up),
            false,
            false,
            false,
            None
        ));
        assert_eq!(edit.text, "sent");
        edit.insert(" changed");
        // Edited recalled prompt no longer matches position.recalled, so Down does not overwrite it.
        assert!(!history.navigate(
            &mut edit,
            &entries,
            &key(Key::Down),
            false,
            false,
            false,
            None
        ));
        assert_eq!(edit.text, "sent changed");
    }

    #[test]
    fn history_respects_line_edges_visual_edges_menus_selections_and_modifiers() {
        let mut history = History::default();
        let entries = vec!["first\nlast".to_owned(), "other".to_owned()];
        let mut edit = TextEdit::new("");
        assert!(history.navigate(
            &mut edit,
            &entries,
            &key(Key::Up),
            false,
            false,
            false,
            None
        ));
        assert_eq!(edit.text, "other");
        assert!(history.navigate(
            &mut edit,
            &entries,
            &key(Key::Up),
            false,
            false,
            false,
            None
        ));
        assert_eq!(edit.text, "first\nlast");
        // Caret at end of multi-line recalled prompt cannot step backward on Up.
        assert!(!history.navigate(
            &mut edit,
            &entries,
            &key(Key::Up),
            false,
            false,
            false,
            None
        ));
        edit.set_caret(0);
        // Caret at start of multi-line recalled prompt cannot step forward on Down.
        assert!(!history.navigate(
            &mut edit,
            &entries,
            &key(Key::Down),
            false,
            false,
            false,
            None
        ));
        // Visual edge false (wrapped line) blocks navigation even without logical newlines.
        let mut wrapped = TextEdit::new("");
        assert!(history.navigate(
            &mut wrapped,
            &[String::from("one long wrapped line")],
            &key(Key::Up),
            false,
            false,
            false,
            Some(true)
        ));
        assert!(!history.navigate(
            &mut wrapped,
            &[String::from("one long wrapped line")],
            &key(Key::Up),
            false,
            false,
            false,
            Some(false)
        ));
        // Menu open, selection, modifiers, empty entries.
        let mut blank = TextEdit::new("");
        assert!(!history.navigate(
            &mut blank,
            &entries,
            &key(Key::Up),
            true,
            false,
            false,
            None
        ));
        blank.replace_text("x");
        blank.set_selection(0, 1);
        assert!(!history.navigate(
            &mut blank,
            &entries,
            &key(Key::Up),
            false,
            false,
            false,
            None
        ));
        let mut empty = TextEdit::new("");
        let mut modified = key(Key::Up);
        modified.mods.shift = true;
        assert!(!history.navigate(&mut empty, &entries, &modified, false, false, false, None));
        assert!(!history.navigate(&mut empty, &[], &key(Key::Up), false, false, false, None));
    }

    #[test]
    fn history_filters_generated_prompts_and_collapses_consecutive_duplicates() {
        let msg = |role: &'static str, text: &str| crate::model::Message {
            id: uuid::Uuid::new_v4().to_string(),
            role,
            text: text.to_owned(),
            images: vec![],
            context: None,
        };
        let messages = vec![
            msg("You", "hello"),
            msg("Assistant", "hi"),
            msg("You", "hello"),
            msg("You", ATTACHMENT_ONLY_BOOTSTRAP_PROMPT),
            msg("You", "PLEASE IMPLEMENT THIS PLAN:\nstep 1"),
            msg("You", "Ultrathink:\n  deep thought  "),
            msg("You", "deep thought"),
            msg("You", "final"),
        ];
        assert_eq!(
            build_history_entries(&messages),
            vec![
                "hello".to_owned(),
                "deep thought".to_owned(),
                "final".to_owned(),
            ]
        );
    }

    #[test]
    fn trigger_uses_the_caret_token_and_whitespace_boundary() {
        for text in ["/", "please /rev", "猫\n/rev", "x\u{2003}/review"] {
            let t = trigger(&TextEdit::new(text)).unwrap();
            assert_eq!(t.marker, '/');
            assert_eq!(&text[t.range.clone()], format!("/{}", t.query));
        }
        for text in ["https://example", "a/review", "/review ", "plain"] {
            assert!(trigger(&TextEdit::new(text)).is_none(), "{text}");
        }
        let mut edit = TextEdit::new("before /rev after");
        edit.set_caret(11);
        assert_eq!(trigger(&edit).unwrap().range, 7..11);
    }

    #[test]
    fn submission_policy_matches_ksg_rules() {
        let enter = KeyEvent {
            key: Key::Enter,
            mods: Default::default(),
        };
        let mut mod_enter = enter.clone();
        mod_enter.mods.cmd = true;
        let mut shift_enter = enter.clone();
        shift_enter.mods.shift = true;
        let mut mod_shift_enter = mod_enter.clone();
        mod_shift_enter.mods.shift = true;
        let mut alt_enter = enter.clone();
        alt_enter.mods.alt = true;

        assert_eq!(
            submission_intent_for_key(
                &enter,
                false,
                false,
                false,
                false,
                SendShortcut::Enter,
                "hi"
            ),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(
                &enter,
                false,
                false,
                false,
                false,
                SendShortcut::Enter,
                "a\nb"
            ),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(
                &enter,
                false,
                false,
                false,
                false,
                SendShortcut::ModEnterMultiline,
                "hi"
            ),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(
                &enter,
                false,
                false,
                false,
                false,
                SendShortcut::ModEnterMultiline,
                "a\nb"
            ),
            None
        );
        assert_eq!(
            submission_intent_for_key(
                &mod_enter,
                false,
                false,
                false,
                false,
                SendShortcut::ModEnterMultiline,
                "a\nb"
            ),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(
                &enter,
                false,
                false,
                false,
                false,
                SendShortcut::ModEnter,
                "hi"
            ),
            None
        );
        assert_eq!(
            submission_intent_for_key(
                &mod_enter,
                false,
                false,
                false,
                false,
                SendShortcut::ModEnter,
                "hi"
            ),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(
                &shift_enter,
                false,
                false,
                false,
                false,
                SendShortcut::Enter,
                "hi"
            ),
            None
        );
        assert_eq!(
            submission_intent_for_key(
                &alt_enter,
                false,
                false,
                false,
                false,
                SendShortcut::Enter,
                "hi"
            ),
            None
        );
        // Key repeat and composition never submit.
        assert_eq!(
            submission_intent_for_key(&enter, true, false, false, false, SendShortcut::Enter, "hi"),
            None
        );
        assert_eq!(
            submission_intent_for_key(&enter, false, true, false, false, SendShortcut::Enter, "hi"),
            None
        );
        // Running thread alternate send shortcuts.
        assert_eq!(
            submission_intent_for_key(
                &mod_enter,
                false,
                false,
                true,
                false,
                SendShortcut::Enter,
                "hi"
            ),
            Some(SubmissionIntent::Alternate)
        );
        assert_eq!(
            submission_intent_for_key(
                &mod_enter,
                false,
                false,
                true,
                false,
                SendShortcut::ModEnter,
                "hi"
            ),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(
                &mod_shift_enter,
                false,
                false,
                true,
                false,
                SendShortcut::ModEnter,
                "hi"
            ),
            Some(SubmissionIntent::Alternate)
        );
    }

    #[test]
    fn prompt_length_validation_matches_ksg_messages() {
        assert_eq!(
            prompt_length_validation_message(&"x".repeat(PROVIDER_SEND_TURN_MAX_INPUT_CHARS)),
            None
        );
        assert_eq!(
            prompt_length_validation_message(&format!(
                "  {}  \n",
                "x".repeat(PROVIDER_SEND_TURN_MAX_INPUT_CHARS)
            )),
            None
        );
        assert_eq!(
            prompt_length_validation_message(&"x".repeat(PROVIDER_SEND_TURN_MAX_INPUT_CHARS + 1)),
            Some(
                "Prompt is 1 character over the 120,000-character limit. Shorten or split it before sending."
                    .into()
            )
        );
        assert_eq!(
            prompt_length_validation_message(&"x".repeat(PROVIDER_SEND_TURN_MAX_INPUT_CHARS + 1234)),
            Some(
                "Prompt is 1,234 characters over the 120,000-character limit. Shorten or split it before sending."
                    .into()
            )
        );
    }

    #[test]
    fn typing_guard_holds_for_1500ms_and_releases_on_blur_send_or_idle() {
        let mut guard = TypingGuard::default();
        let t0 = std::time::Instant::now();
        let reqs = vec!["req-1".to_owned()];

        // Unfocused composer never holds.
        guard.on_type(t0);
        guard.on_requests(&reqs, false, t0 + std::time::Duration::from_millis(100));
        assert!(!guard.is_held("req-1"));

        // Focused + recent typing holds new request.
        let reqs2 = vec!["req-2".to_owned()];
        guard.on_type(t0);
        guard.on_requests(&reqs2, true, t0 + std::time::Duration::from_millis(200));
        assert!(guard.is_held("req-2"));

        // Re-evaluating the same request id does not re-hold once seen, even if typing continues.
        guard.on_blur_or_send();
        assert!(!guard.is_held("req-2"));
        guard.on_type(t0 + std::time::Duration::from_millis(300));
        guard.on_requests(&reqs2, true, t0 + std::time::Duration::from_millis(400));
        assert!(!guard.is_held("req-2"));

        // Idle timeout at 1500ms releases held request.
        let reqs3 = vec!["req-3".to_owned()];
        let t1 = t0 + std::time::Duration::from_secs(5);
        guard.on_type(t1);
        guard.on_requests(&reqs3, true, t1 + std::time::Duration::from_millis(100));
        assert!(guard.is_held("req-3"));
        assert!(!guard.release_if_idle(t1 + std::time::Duration::from_millis(1499)));
        assert!(guard.is_held("req-3"));
        assert!(guard.release_if_idle(t1 + std::time::Duration::from_millis(1500)));
        assert!(!guard.is_held("req-3"));

        // Request arriving at or after 1500ms since last type is not held.
        let reqs4 = vec!["req-4".to_owned()];
        guard.on_type(t1);
        guard.on_requests(
            &reqs4,
            true,
            t1 + std::time::Duration::from_millis(COMPOSER_TYPING_IDLE_MS),
        );
        assert!(!guard.is_held("req-4"));
    }

    #[test]
    fn slash_command_ranking_matches_ksg_order_and_tie_breaks() {
        let cmd =
            |name: &str, description: &str, source: &str| crate::model::pi_session::SlashCommand {
                name: name.to_owned(),
                description: description.to_owned(),
                source: source.to_owned(),
            };
        let commands = vec![
            cmd("my-review-helper", "Unrelated", "ACP"),
            cmd("code-review", "Unrelated", "ACP"),
            cmd("review-all", "Unrelated", "ACP"),
            cmd("review", "Exact match", "ACP"),
            cmd("other", "Run review now", "ACP"),
            cmd("r_e_v_i_e_w", "Fuzzy match", "ACP"),
        ];
        let ranked: Vec<String> = rank_slash_commands(commands, "review")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(
            ranked,
            vec![
                "review",           // exact (0)
                "review-all",       // prefix (2 + 4 = 6)
                "code-review",      // word boundary after '-' (4 + 10 + 5 = 19)
                "my-review-helper", // word boundary after '-' (4 + 6 + 10 = 20)
                "other",            // description word boundary (24 + 8 + 8 = 40)
                "r_e_v_i_e_w",      // fuzzy subsequence (100 + ...)
            ]
        );

        // Tie-break order: Shina built-in < provider slash command < skill, then alphabetical.
        let tied = vec![
            cmd("skill:alpha", "", "skill"),
            cmd("beta", "", "ACP"),
            cmd("alpha", "", "ACP"),
            cmd("model", "", "Shina"),
        ];
        let tied_empty: Vec<String> = rank_slash_commands(tied, "")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(tied_empty, vec!["skill:alpha", "beta", "alpha", "model"]);
    }

    #[test]
    fn dismissed_trigger_persists_across_edits_until_identity_changes() {
        let dismissed = DismissedTrigger {
            marker: '/',
            range_start: 0,
        };
        let mut edit = TextEdit::new("/mod");
        let t1 = trigger(&edit).unwrap();
        assert!(dismissed.matches(&t1));

        // Typing more in the same trigger keeps it dismissed.
        edit.replace_text("/model");
        let t2 = trigger(&edit).unwrap();
        assert!(dismissed.matches(&t2));

        // Moving to a different trigger position or marker clears dismissal.
        edit.replace_text("hi /mod");
        let t3 = trigger(&edit).unwrap();
        assert!(!dismissed.matches(&t3));

        edit.replace_text("@mod");
        let t4 = trigger(&edit).unwrap();
        assert!(!dismissed.matches(&t4));
    }

    #[test]
    fn background_and_new_thread_submission_intents_match_ksg_rules() {
        let enter = KeyEvent {
            key: Key::Enter,
            mods: Default::default(),
        };
        let mut mod_enter = enter.clone();
        mod_enter.mods.cmd = true;
        let mut mod_alt_enter = mod_enter.clone();
        mod_alt_enter.mods.alt = true;

        let defaults = crate::keybindings::default_resolved_keybindings();
        let resolve = |event: &KeyEvent,
                       rules: &[crate::keybindings::ResolvedKeybindingRule],
                       is_draft_thread: bool,
                       is_running: bool,
                       send_shortcut: SendShortcut,
                       key_repeat: bool,
                       is_composing: bool| {
            let mut ctx = crate::keybindings::ShortcutContext::desktop();
            ctx.composer_focus = true;
            ctx.draft_thread_route = is_draft_thread;
            ctx.turn_running = is_running;
            ctx.composer_requires_modifier = requires_modifier(send_shortcut, "hi");
            let command = crate::keybindings::resolve_shortcut_command(
                event,
                rules,
                &ctx,
                crate::keybindings::Platform::current(),
                None,
                None,
            );
            submission_intent_for_command(
                event,
                key_repeat,
                is_composing,
                is_running,
                is_draft_thread,
                send_shortcut,
                "hi",
                command,
            )
        };

        // Draft thread: mod+enter and mod+alt+enter both yield Background regardless of send_shortcut.
        for shortcut in SendShortcut::ALL {
            assert_eq!(
                resolve(&mod_enter, &defaults, true, false, shortcut, false, false),
                Some(SubmissionIntent::Background),
                "{shortcut:?}"
            );
            assert_eq!(
                resolve(
                    &mod_alt_enter,
                    &defaults,
                    true,
                    false,
                    shortcut,
                    false,
                    false
                ),
                Some(SubmissionIntent::Background),
                "{shortcut:?}"
            );
        }

        // Existing thread (idle or running): mod+alt+enter yields Background; mod+enter does not.
        assert_eq!(
            resolve(
                &mod_alt_enter,
                &defaults,
                false,
                false,
                SendShortcut::Enter,
                false,
                false
            ),
            Some(SubmissionIntent::Background)
        );
        assert_eq!(
            resolve(
                &mod_alt_enter,
                &defaults,
                false,
                true,
                SendShortcut::ModEnter,
                false,
                false
            ),
            Some(SubmissionIntent::Background)
        );
        assert_eq!(
            resolve(
                &mod_enter,
                &defaults,
                false,
                false,
                SendShortcut::ModEnter,
                false,
                false
            ),
            Some(SubmissionIntent::Foreground)
        );

        // Key repeat and IME composition never submit.
        assert_eq!(
            resolve(
                &mod_enter,
                &defaults,
                true,
                false,
                SendShortcut::Enter,
                true,
                false
            ),
            None
        );
        assert_eq!(
            resolve(
                &mod_alt_enter,
                &defaults,
                false,
                false,
                SendShortcut::Enter,
                false,
                true
            ),
            None
        );

        // Custom rebinding overrides defaults for sendBackground and sendAndNewThread.
        let mut custom = Vec::new();
        crate::keybindings::upsert_custom_rule(
            &mut custom,
            "composer.sendBackground",
            "alt+b",
            Some("composerFocus && draftThreadRoute"),
            None,
        )
        .unwrap();
        crate::keybindings::upsert_custom_rule(
            &mut custom,
            "composer.sendAndNewThread",
            "alt+n",
            Some("composerFocus && !draftThreadRoute"),
            None,
        )
        .unwrap();
        let merged = crate::keybindings::merge_with_default_keybindings(&custom);
        let alt_b = KeyEvent {
            key: Key::Text("b".into()),
            mods: rune::Mods {
                alt: true,
                ..Default::default()
            },
        };
        let alt_n = KeyEvent {
            key: Key::Text("n".into()),
            mods: rune::Mods {
                alt: true,
                ..Default::default()
            },
        };
        assert_eq!(
            resolve(
                &alt_b,
                &merged,
                true,
                false,
                SendShortcut::ModEnter,
                false,
                false
            ),
            Some(SubmissionIntent::Background)
        );
        assert_eq!(
            resolve(
                &alt_b,
                &merged,
                false,
                false,
                SendShortcut::ModEnter,
                false,
                false
            ),
            None
        );
        assert_eq!(
            resolve(
                &alt_n,
                &merged,
                false,
                false,
                SendShortcut::ModEnter,
                false,
                false
            ),
            Some(SubmissionIntent::Background)
        );
        assert_eq!(
            resolve(
                &mod_enter,
                &merged,
                true,
                false,
                SendShortcut::ModEnter,
                false,
                false
            ),
            Some(SubmissionIntent::Foreground)
        );
    }
}
