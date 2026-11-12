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
    Alternate,
}

pub fn submission_intent_for_key(
    event: &KeyEvent,
    key_repeat: bool,
    is_composing: bool,
    is_running: bool,
    send_shortcut: SendShortcut,
    prompt: &str,
) -> Option<SubmissionIntent> {
    if is_composing || key_repeat {
        return None;
    }
    if event.key != Key::Enter || event.mods.alt {
        return None;
    }
    let has_mod = event.mods.cmd || event.mods.ctrl;
    let needs_mod = requires_modifier(send_shortcut, prompt);
    if is_running && has_mod && !event.mods.logo {
        if needs_mod {
            if event.mods.shift {
                return Some(SubmissionIntent::Alternate);
            }
            return Some(SubmissionIntent::Foreground);
        } else if !event.mods.shift {
            return Some(SubmissionIntent::Alternate);
        }
    }
    if event.mods.shift {
        return None;
    }
    if needs_mod && !has_mod {
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
    let len = trimmed.chars().count();
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

/// Per-thread browsing state. Keep the whole edit so recall also preserves the caret and marks.
#[derive(Default)]
pub struct History {
    position: Option<usize>,
    saved: Option<TextEdit>,
}

impl History {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn navigate(
        &mut self,
        edit: &mut TextEdit,
        entries: &[&str],
        event: &KeyEvent,
        menu_open: bool,
    ) -> bool {
        let m = event.mods;
        if menu_open
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
        let next = if backward {
            let at = self.position.unwrap_or(entries.len());
            if at == 0 {
                return self.position.is_some();
            }
            at - 1
        } else {
            let Some(at) = self.position else {
                return false;
            };
            at + 1
        };
        if self.position.is_none() {
            self.saved = Some(edit.clone());
        }
        if next >= entries.len() {
            if let Some(saved) = self.saved.take() {
                *edit = saved;
            }
            self.reset();
        } else {
            edit.replace_text(entries[next]);
            edit.focused = true;
            self.position = Some(next);
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
    fn history_restores_the_unsent_edit_and_caret() {
        let mut history = History::default();
        let mut edit = TextEdit::new("unfinished\nsecond line");
        edit.set_caret(3);
        let entries = ["first", "second"];
        assert!(history.navigate(&mut edit, &entries, &key(Key::Up), false));
        assert_eq!(edit.text, "second");
        assert!(history.navigate(&mut edit, &entries, &key(Key::Up), false));
        assert_eq!(edit.text, "first");
        assert!(history.navigate(&mut edit, &entries, &key(Key::Down), false));
        assert_eq!(edit.text, "second");
        assert!(history.navigate(&mut edit, &entries, &key(Key::Down), false));
        assert_eq!(edit.text, "unfinished\nsecond line");
        assert_eq!(edit.focus, 3);
    }

    #[test]
    fn editing_a_recalled_prompt_does_not_discard_the_original_unsent_draft() {
        let mut history = History::default();
        let mut edit = TextEdit::new("original unsent");
        assert!(history.navigate(&mut edit, &["sent"], &key(Key::Up), false));
        edit.insert(" changed");
        assert!(history.navigate(&mut edit, &["sent"], &key(Key::Down), false));
        assert_eq!(edit.text, "original unsent");
    }

    #[test]
    fn history_respects_line_edges_menus_selections_and_modifiers() {
        let mut history = History::default();
        let mut edit = TextEdit::new("first\nlast");
        assert!(!history.navigate(&mut edit, &["sent"], &key(Key::Up), false));
        edit.set_caret(0);
        assert!(!history.navigate(&mut edit, &["sent"], &key(Key::Up), true));
        edit.set_selection(0, 2);
        assert!(!history.navigate(&mut edit, &["sent"], &key(Key::Up), false));
        edit.set_caret(0);
        let mut modified = key(Key::Up);
        modified.mods.shift = true;
        assert!(!history.navigate(&mut edit, &["sent"], &modified, false));
        assert!(!history.navigate(&mut edit, &[], &key(Key::Up), false));
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
            submission_intent_for_key(&enter, false, false, false, SendShortcut::Enter, "hi"),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(&enter, false, false, false, SendShortcut::Enter, "a\nb"),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(
                &enter,
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
                SendShortcut::ModEnterMultiline,
                "a\nb"
            ),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(&enter, false, false, false, SendShortcut::ModEnter, "hi"),
            None
        );
        assert_eq!(
            submission_intent_for_key(
                &mod_enter,
                false,
                false,
                false,
                SendShortcut::ModEnter,
                "hi"
            ),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(&shift_enter, false, false, false, SendShortcut::Enter, "hi"),
            None
        );
        assert_eq!(
            submission_intent_for_key(&alt_enter, false, false, false, SendShortcut::Enter, "hi"),
            None
        );
        // Key repeat and composition never submit.
        assert_eq!(
            submission_intent_for_key(&enter, true, false, false, SendShortcut::Enter, "hi"),
            None
        );
        assert_eq!(
            submission_intent_for_key(&enter, false, true, false, SendShortcut::Enter, "hi"),
            None
        );
        // Running thread alternate send shortcuts.
        assert_eq!(
            submission_intent_for_key(&mod_enter, false, false, true, SendShortcut::Enter, "hi"),
            Some(SubmissionIntent::Alternate)
        );
        assert_eq!(
            submission_intent_for_key(&mod_enter, false, false, true, SendShortcut::ModEnter, "hi"),
            Some(SubmissionIntent::Foreground)
        );
        assert_eq!(
            submission_intent_for_key(
                &mod_shift_enter,
                false,
                false,
                true,
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
}
