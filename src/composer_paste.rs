//! KSG Code's client-runtime/textPaste.ts and chat/useComposerAttachmentIntake.ts.
use crate::attachments::{self, Attachment};
use rune::{KeyEvent, TextEdit};

pub const THRESHOLD: usize = 32 * 1024;

pub fn should_fold(text: &str) -> bool {
    text.len() >= THRESHOLD
}

/// KSG's handlePaste ignores HTML and parses the normalized plain-text Markdown.
/// File mentions use this draft's workspace, not this process's working directory.
pub fn insert(state: &mut crate::model::State, text: &str) {
    let thread = state.current();
    let root = state.project(thread.project).map(|p| p.path.clone());
    let harness = thread.harness.clone();
    let skills = thread.native.skills.clone();
    let path_query = crate::composer::trigger(&thread.draft).is_some_and(|trigger| {
        trigger.marker == '@'
            && !thread
                .native
                .slash_dismissed
                .as_ref()
                .is_some_and(|d| d.matches(&trigger))
    });
    let thread = state.current_mut();
    crate::composer_rich::paste_with_path_query(&mut thread.draft, text, path_query, &|source| {
        if let Some(occ) = crate::composer_context::collect_composer_context_references(source)
            .into_iter()
            .next()
            && occ.source == source
        {
            return Some((
                crate::composer_context::format_composer_context_href(&occ.kind, &occ.context_id),
                occ.label,
            ));
        }
        if let Some(path) = crate::composer_rich::pasted_mention_path(source) {
            let path = std::path::PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                root.as_ref()?.join(path)
            };
            let label = path.file_name()?.to_string_lossy().into_owned();
            let token = crate::composer_tokens::Token::File {
                directory: source.ends_with('/'),
                path,
                source: Some(source.into()),
            };
            Some((token.id(), label))
        } else {
            let name = &source[source.chars().next()?.len_utf8()..];
            let skill = skills.iter().find(|skill| skill.name == name)?;
            let token = crate::composer_tokens::Token::Skill {
                name: skill.name.clone(),
                harness: harness.clone(),
            };
            Some((token.id(), skill.name.clone()))
        }
    });
    thread.composer_rules.pasted();
    thread.update_submission_error();
}

pub fn stage(text: &str, existing: &[Attachment]) -> Result<Attachment, String> {
    let name = (1..)
        .map(|n| {
            if n == 1 {
                "pasted-text.txt".into()
            } else {
                format!("pasted-text-{n}.txt")
            }
        })
        .find(|name: &String| !existing.iter().any(|a| a.name.eq_ignore_ascii_case(name)))
        .unwrap();
    let directory = attachments::directory()
        .ok_or("The attachment directory is unavailable.")?
        .join(format!("paste-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(name);
    std::fs::write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
    let mut attachment = attachments::attach_file(&path)?;
    attachment.mime = "text/plain".into();
    Ok(attachment)
}

struct Paste {
    at: TextEdit,
    attachment: Attachment,
}

/// Rune records document edits, but cannot record an external attachment tray. Insert a tray
/// undo step between document undo groups. Compare content, not the caret, so moving the caret
/// does not detach the step from its paste. Never remove the immutable file on undo.
#[derive(Default)]
pub struct PasteHistory {
    undo: Vec<Paste>,
    redo: Vec<Paste>,
}

fn same_document(a: &TextEdit, b: &TextEdit) -> bool {
    a.text == b.text
        && a.marks() == b.marks()
        && a.blocks() == b.blocks()
        && a.soft_breaks() == b.soft_breaks()
}

impl PasteHistory {
    #[cfg(test)]
    pub fn record(&mut self, edit: &mut TextEdit, attachment: Attachment) {
        edit.break_history_group();
        self.redo.clear();
        self.undo.push(Paste {
            at: edit.clone(),
            attachment,
        });
        if self.undo.len() > attachments::MAX_ATTACHMENTS {
            self.undo.remove(0);
        }
    }

    pub fn key(
        &mut self,
        edit: &mut TextEdit,
        attachments: &mut Vec<Attachment>,
        ev: &KeyEvent,
    ) -> bool {
        let undo = ev.is_command('z') && !ev.mods.shift;
        let redo = ev.is_command('z') && ev.mods.shift || ev.is_command('y');
        if undo {
            while self
                .undo
                .last()
                .is_some_and(|p| !attachments.contains(&p.attachment))
            {
                self.undo.pop();
            }
            if self.undo.last().is_some_and(|p| same_document(&p.at, edit)) {
                let paste = self.undo.pop().unwrap();
                attachments.retain(|a| a.id != paste.attachment.id);
                edit.break_history_group();
                self.redo.push(paste);
                return true;
            }
        } else if redo {
            if self.redo.last().is_some_and(|p| same_document(&p.at, edit))
                && attachments.len() < attachments::MAX_ATTACHMENTS
            {
                let paste = self.redo.pop().unwrap();
                attachments.push(paste.attachment.clone());
                self.undo.push(paste);
                edit.break_history_group();
                return true;
            }
        } else if matches!(
            ev.key,
            rune::Key::Text(_) | rune::Key::Backspace | rune::Key::Delete | rune::Key::Enter
        ) {
            self.redo.clear();
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_is_utf8_bytes_and_staged_text_is_exact_and_uniquely_named() {
        assert!(!should_fold(&"x".repeat(THRESHOLD - 1)));
        assert!(should_fold(&"x".repeat(THRESHOLD)));
        let text = "猫".repeat(THRESHOLD / 3 + 1);
        assert!(should_fold(&text));
        let a = stage(&text, &[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(a.path.as_ref().unwrap()).unwrap(),
            text
        );
        let b = stage("second", &[a]).unwrap();
        assert_eq!(b.name, "pasted-text-2.txt");
    }

    #[test]
    fn attachment_undo_and_redo_interleave_with_document_undo() {
        let mut edit = TextEdit::new("draft");
        let attachment = stage("pasted", &[]).unwrap();
        let mut tray = vec![attachment.clone()];
        let mut history = PasteHistory::default();
        history.record(&mut edit, attachment);
        edit.insert("later");
        let undo = KeyEvent {
            key: rune::Key::Command('z'),
            mods: rune::Mods {
                cmd: true,
                ..Default::default()
            },
        };
        assert!(!history.key(&mut edit, &mut tray, &undo));
        edit.undo();
        assert_eq!(edit.text, "draft");
        assert!(history.key(&mut edit, &mut tray, &undo));
        assert!(tray.is_empty());
        let mut redo = undo.clone();
        redo.mods.shift = true;
        assert!(history.key(&mut edit, &mut tray, &redo));
        assert_eq!(tray.len(), 1);
        assert!(!history.key(&mut edit, &mut tray, &redo));
        edit.redo();
        assert_eq!(edit.text, "draftlater");
    }
}
