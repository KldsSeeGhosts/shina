//! Self-contained pill ids survive Rune undo and UI snapshot persistence.
use crate::attachments::{self, Attachment, Prompt};
use rune::TextEdit;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Token {
    File {
        path: std::path::PathBuf,
        directory: bool,
    },
    Thread {
        id: usize,
        title: String,
    },
    Skill {
        name: String,
        harness: String,
    },
}

impl Token {
    pub fn id(&self) -> String {
        format!("shina:{}", serde_json::to_string(self).unwrap())
    }
    pub fn decode(id: &str) -> Option<Self> {
        if id.len() > 8192 {
            return None;
        }
        serde_json::from_str(id.strip_prefix("shina:")?).ok()
    }
    pub fn source(&self) -> String {
        match self {
            Self::File { path, .. } => path.to_string_lossy().into_owned(),
            Self::Thread { id, title } => format!(
                "Shina thread {}, read_thread({})",
                serde_json::to_string(title).unwrap(),
                serde_json::json!({"thread_id":id})
            ),
            // Codex and ACP advertise $name. Pi's bridge expands these exactly as KSG does.
            Self::Skill { name, .. } => format!("${name}"),
        }
    }
}

pub fn prompt(edit: &TextEdit, images: &[Attachment]) -> Result<Prompt, String> {
    let mut attachments = images.to_vec();
    for (_, id) in edit.pills() {
        if let Some(Token::File { path, .. }) = Token::decode(id) {
            let attachment = attachments::reference_file(&path)?;
            if !attachments.iter().any(|a| a.id == attachment.id) {
                attachments.push(attachment);
            }
        }
    }
    attachments::validate_prompt_images(&attachments)?;
    Ok(Prompt::new(
        crate::composer_rich::markdown(edit),
        attachments,
    ))
}

/// A server receives serialized text, not the client's pill marks. Do not interpret a leading
/// attached path as a slash command. Require the full path and its token boundary.
pub fn starts_with_file_path(text: &str, attachments: &[Attachment]) -> bool {
    attachments
        .iter()
        .filter_map(|a| a.path.as_ref())
        .any(|path| {
            text.strip_prefix(path.to_string_lossy().as_ref())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leading_file_pills_remain_paths_after_server_serialization() {
        let root = crate::store::directory().unwrap();
        let path = root.join("a file.txt");
        std::fs::write(&path, "content").unwrap();
        let attachment = attachments::reference_file(&path).unwrap();
        let path = attachment.path.as_ref().unwrap().display().to_string();
        assert!(starts_with_file_path(
            &format!("{path} please review"),
            &[attachment.clone()]
        ));
        assert!(starts_with_file_path(&path, &[attachment.clone()]));
        assert!(!starts_with_file_path(
            &format!("{path}-other"),
            &[attachment.clone()]
        ));
        assert!(!starts_with_file_path("/compact", &[attachment]));
    }
    #[test]
    fn pills_are_atomic_undoable_and_serialize_through_existing_attachment_paths() {
        let root = crate::store::directory().unwrap();
        let path = root.join("mention file.txt");
        std::fs::write(&path, "content").unwrap();
        let token = Token::File {
            path: path.clone(),
            directory: false,
        };
        let mut edit = crate::composer_rich::draft("read @men");
        edit.undo_group(|e| {
            e.replace_with_pill(5..9, token.id(), "mention file.txt");
            e.insert(" ");
        });
        let prompt = prompt(&edit, &[]).unwrap();
        assert_eq!(prompt.text, format!("read {} ", path.display()));
        assert_eq!(prompt.images.len(), 1);
        assert!(
            attachments::pi_message(&prompt)
                .unwrap()
                .contains("[Attached file \"mention file.txt\" is saved at:")
        );
        edit.undo();
        assert_eq!(edit.text, "read @men");
        edit.redo();
        let restored = crate::composer_rich::restore(&edit.text, Some(&edit.to_html()));
        assert_eq!(prompt.text, crate::composer_rich::markdown(&restored));
        let thread = Token::Thread {
            id: 42,
            title: "Fix \"quoted\" title".into(),
        };
        assert_eq!(
            thread.source(),
            "Shina thread \"Fix \\\"quoted\\\" title\", read_thread({\"thread_id\":42})"
        );
    }
}
