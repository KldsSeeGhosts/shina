//! Composer editing rules, ported from KSG Code's chat/composerCommandKey.ts.
use rune::{Key, KeyEvent, TextEdit};

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
}
