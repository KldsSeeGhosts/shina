//! Pressing a composer pill previews its file, ported from KSG Code's
//! `ComposerMentionNodeView` (`onClick={() => actions.openMention(path)}`, label
//! `Preview {path}`) and `ComposerSkillNodeView`, which is pressable only when
//! the skill has a path. KSG opens the file in a right panel; Shina has no
//! right panel yet, so the attachment preview dialog is the settled substitute.
use crate::composer_tokens::Token;
use crate::model::State;
use std::path::{Path, PathBuf};

/// What pressing a pill does. The mapping is pure so every token kind is
/// unit-tested against KSG's behavior; the effects live on `State`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PillAction {
    /// Open this path in the attachment preview dialog. It never joins the
    /// draft's attachments.
    Preview(PathBuf),
    /// Directories have no preview in KSG's viewer, and thread, model and
    /// machine pills are inert there too.
    Nothing,
}

pub(crate) fn action(s: &State, id: &str) -> PillAction {
    match Token::decode(id) {
        Some(Token::File {
            path,
            directory: false,
            ..
        }) => PillAction::Preview(path),
        // A skill chip opens its instruction file only when the inventory
        // recorded a path, matching KSG's `ComposerSkillNodeView`.
        Some(Token::Skill { name, .. }) => s
            .current()
            .native
            .skills
            .iter()
            .find(|skill| skill.name == name)
            .and_then(|skill| skill.path.clone())
            .map_or(PillAction::Nothing, PillAction::Preview),
        _ => PillAction::Nothing,
    }
}

/// A file that is missing, unreadable or still downloading shows the dialog's
/// existing failure state with retry.
fn placeholder(id: String, name: String, path: Option<PathBuf>) -> crate::attachments::Attachment {
    crate::attachments::Attachment {
        id,
        mime: crate::attachment_intake::native_mime(&name).into(),
        name,
        width: 0,
        height: 0,
        bytes: 0,
        path,
        asset_id: None,
        uploaded: None,
        source: None,
    }
}

impl State {
    /// `El::on_input_pill_press` on the composer field lands here. The pill id
    /// decodes to the token, so resolution always uses the current draft and
    /// skill inventory rather than stale pointer-down state.
    pub(crate) fn press_composer_pill(&mut self, id: &str) {
        let PillAction::Preview(path) = action(self, id) else {
            return;
        };
        let (machine, project) = {
            let t = self.current();
            (t.machine.clone(), t.project)
        };
        let cwd = crate::ui::composer_menu::mention_cwd(self, &machine, project);
        // Relative mention paths resolve against the thread's working directory.
        let path = if path.is_absolute() {
            path
        } else {
            match cwd.as_deref() {
                Some(cwd) => Path::new(cwd).join(&path),
                None => return,
            }
        };
        if machine == "local" {
            self.preview_mention_path(&path);
        }
    }

    /// Open a transient preview for one local path. Missing or unreadable
    /// files land on the dialog's existing failure state with retry.
    pub(crate) fn preview_mention_path(&mut self, path: &Path) {
        match crate::attachments::reference_file(path).and_then(|file| {
            crate::attachments::validate_file_metadata(&file.name, &file.mime, file.bytes)
                .map(|_| file)
        }) {
            Ok(attachment) => self.preview_mention(attachment),
            Err(error) => {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let attachment = placeholder(
                    format!("mention-{}", path.to_string_lossy()),
                    name,
                    Some(path.to_path_buf()),
                );
                self.preview_mention(attachment);
                if let Some(preview) = &mut self.attachment_preview {
                    preview.error = Some(error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pill_press_maps_each_token_kind() {
        let mut s = State::new(".".into(), false);
        s.current_mut().native.skills = vec![
            crate::composer_skills::Skill {
                name: "review".into(),
                description: "Review".into(),
                path: Some("/skills/review/SKILL.md".into()),
                user_only: false,
            },
            crate::composer_skills::Skill {
                name: "pathless".into(),
                description: "No path".into(),
                path: None,
                user_only: false,
            },
        ];
        let file = Token::File {
            path: "/work/notes.txt".into(),
            directory: false,
            source: None,
        }
        .id();
        assert_eq!(
            action(&s, &file),
            PillAction::Preview("/work/notes.txt".into())
        );
        let directory = Token::File {
            path: "/work/src".into(),
            directory: true,
            source: None,
        }
        .id();
        assert_eq!(action(&s, &directory), PillAction::Nothing);
        let skill = Token::Skill {
            name: "review".into(),
            harness: "codex".into(),
        }
        .id();
        assert_eq!(
            action(&s, &skill),
            PillAction::Preview("/skills/review/SKILL.md".into())
        );
        let pathless = Token::Skill {
            name: "pathless".into(),
            harness: "codex".into(),
        }
        .id();
        assert_eq!(action(&s, &pathless), PillAction::Nothing);
        let unknown = Token::Skill {
            name: "missing".into(),
            harness: "codex".into(),
        }
        .id();
        assert_eq!(action(&s, &unknown), PillAction::Nothing);
        let thread = Token::Thread {
            id: 4,
            title: "Fix".into(),
            machine: None,
            extra: Default::default(),
        }
        .id();
        assert_eq!(action(&s, &thread), PillAction::Nothing);
        let model = Token::Model {
            provider_instance: "cpa".into(),
            model: "opus".into(),
            label: "Opus".into(),
        }
        .id();
        assert_eq!(action(&s, &model), PillAction::Nothing);
        let machine = Token::Machine {
            machine: "studio".into(),
            label: "Studio".into(),
        }
        .id();
        assert_eq!(action(&s, &machine), PillAction::Nothing);
        assert_eq!(action(&s, "not-a-pill"), PillAction::Nothing);
    }

    #[test]
    fn pressing_a_file_pill_previews_without_touching_the_draft() {
        let directory =
            std::env::temp_dir().join(format!("shina-pill-unit-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("note.txt");
        std::fs::write(&file, "preview body").unwrap();
        let mut s = State::new(".".into(), false);
        let token = Token::File {
            path: file.clone(),
            directory: false,
            source: None,
        };
        s.press_composer_pill(&token.id());
        let preview = s.attachment_preview.as_ref().unwrap();
        assert!(preview.transient && preview.error.is_none());
        assert_eq!(preview.items[preview.index].name, "note.txt");
        assert!(s.current().draft_images.is_empty());
        // A missing file lands on the dialog's failure state.
        let missing = Token::File {
            path: directory.join("missing.txt"),
            directory: false,
            source: None,
        };
        s.press_composer_pill(&missing.id());
        let preview = s.attachment_preview.as_ref().unwrap();
        assert!(preview.error.is_some());
        assert!(s.current().draft_images.is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
