//! Self-contained pill ids survive Rune undo and UI snapshot persistence.
use crate::attachments::{self, Attachment, Prompt};
use rune::TextEdit;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Token {
    File {
        path: std::path::PathBuf,
        directory: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    Thread {
        id: usize,
        title: String,
        #[serde(default)]
        machine: Option<String>,
        #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
        extra: serde_json::Map<String, serde_json::Value>,
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
    #[cfg(test)]
    pub fn source(&self) -> String {
        self.source_for(None, &|machine| machine.to_owned())
    }
    pub fn source_for(
        &self,
        target_machine: Option<&str>,
        _machine_name: &dyn Fn(&str) -> String,
    ) -> String {
        match self {
            Self::File { path, source, .. } => source
                .clone()
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            Self::Thread {
                id, title, machine, ..
            } => {
                if let (Some(owner), Some(target)) = (machine.as_deref(), target_machine)
                    && owner != target
                {
                    format!(
                        "Shina thread {}, read_thread({})",
                        serde_json::to_string(title).unwrap(),
                        serde_json::json!({"machine":owner,"threadId":id})
                    )
                } else {
                    format!(
                        "Shina thread {}, read_thread({})",
                        serde_json::to_string(title).unwrap(),
                        serde_json::json!({"thread_id":id})
                    )
                }
            }
            // Codex and ACP advertise $name. Pi's bridge expands these exactly as KSG does.
            Self::Skill { name, .. } => format!("${name}"),
        }
    }
}

pub fn collect_inline_context_ids_from_edit(edit: &TextEdit) -> std::collections::HashSet<String> {
    edit.pills()
        .filter_map(|(_, id)| {
            crate::composer_context::parse_composer_context_href(id).map(|(_, cid)| cid)
        })
        .collect()
}

pub fn stamp_thread_owner(edit: &mut TextEdit, owner: &str) {
    let updates: Vec<_> = edit
        .pills()
        .filter_map(|(range, id)| match Token::decode(id) {
            Some(Token::Thread {
                id,
                title,
                machine: None,
                extra,
            }) => Some((
                range,
                Token::Thread {
                    id,
                    title: title.clone(),
                    machine: Some(owner.to_owned()),
                    extra,
                }
                .id(),
                title,
            )),
            _ => None,
        })
        .collect();
    for (range, new_id, label) in updates {
        edit.replace_with_pill(range, new_id, &label);
    }
}

#[cfg(test)]
pub fn prompt(edit: &TextEdit, images: &[Attachment]) -> Result<Prompt, String> {
    prompt_for(edit, images, None, &|machine| machine.to_owned())
}

#[cfg(test)]
pub fn prompt_for(
    edit: &TextEdit,
    images: &[Attachment],
    target_machine: Option<&str>,
    machine_name: &dyn Fn(&str) -> String,
) -> Result<Prompt, String> {
    prompt_for_with_context(edit, images, &[], target_machine, machine_name)
}

pub fn prompt_for_with_context(
    edit: &TextEdit,
    images: &[Attachment],
    draft_context: &[crate::composer_context::ComposerContextRecord],
    target_machine: Option<&str>,
    machine_name: &dyn Fn(&str) -> String,
) -> Result<Prompt, String> {
    prompt_for_target(
        edit,
        images,
        draft_context,
        target_machine,
        None,
        &[],
        machine_name,
    )
}

pub fn prompt_for_target(
    edit: &TextEdit,
    images: &[Attachment],
    draft_context: &[crate::composer_context::ComposerContextRecord],
    target_machine: Option<&str>,
    target_server_id: Option<&str>,
    peers: &[crate::peer::Peer],
    machine_name: &dyn Fn(&str) -> String,
) -> Result<Prompt, String> {
    validate_pill_kinds(edit)?;
    let mut attachments = images.to_vec();
    let referenced_ids = collect_inline_context_ids_from_edit(edit);
    if let Some(target) = target_machine {
        if let Some((record, reason)) =
            crate::composer_context::thread_contexts_outside_environment(
                &referenced_ids,
                draft_context,
                target,
                target_server_id,
                peers,
            )
            .into_iter()
            .next()
            && let crate::composer_context::ComposerContextData::Thread {
                environment_id,
                title,
                ..
            } = record.data
        {
            return Err(crate::composer_context::format_outside_thread_error(
                &title,
                &machine_name(&environment_id),
                reason,
            ));
        }
    }
    for (_, id) in edit.pills() {
        // Ported from KSG Code's `threadContextsOutsideEnvironment` and its use in
        // `useComposerSend.ts`: block only when the owner machine is unlinked or lacks
        // `thread-read-v2`, naming which.
        if let Some(Token::Thread {
            title,
            machine: Some(owner),
            ..
        }) = Token::decode(id)
            && let Some(target) = target_machine
            && let Some(reason) = crate::composer_context::outside_thread_reason(
                &owner,
                target,
                target_server_id,
                peers,
            )
        {
            return Err(crate::composer_context::format_outside_thread_error(
                &title,
                &machine_name(&owner),
                reason,
            ));
        }
        if let Some(Token::File { path, .. }) = Token::decode(id) {
            let attachment = attachments::reference_file(&path)?;
            if !attachments.iter().any(|a| a.id == attachment.id) {
                attachments.push(attachment);
            }
        }
    }
    attachments::validate_prompt_images(&attachments)?;
    let context = crate::composer_context::build_message_context(
        &referenced_ids,
        draft_context,
        &attachments,
    );
    let mut prompt = Prompt::with_context(
        crate::composer_rich::markdown_for(edit, target_machine, machine_name),
        attachments,
        context,
    );
    prompt.draft = Some(crate::composer_rich::Snapshot::capture_with_context(
        edit,
        images,
        draft_context,
    ));
    Ok(prompt)
}

fn validate_pill_kinds(edit: &TextEdit) -> Result<(), String> {
    if edit
        .pills()
        .any(|(_, id)| id.starts_with("shina:") && Token::decode(id).is_none())
    {
        return Err("Update required. This draft contains a mention this version of Shina cannot send. Update Shina or remove that mention.".into());
    }
    Ok(())
}

/// Validate the additive editor snapshot against the provider text and admitted attachments.
/// Missing snapshots remain valid for older peers. Unknown snapshot fields are ignored.
#[cfg(test)]
pub fn received_prompt(
    text: String,
    images: Vec<Attachment>,
    snapshot: Option<serde_json::Value>,
) -> Result<Prompt, String> {
    received_prompt_with_context(text, images, snapshot, None)
}

pub fn received_prompt_with_context(
    text: String,
    images: Vec<Attachment>,
    snapshot: Option<serde_json::Value>,
    context: Option<crate::composer_context::MessageContext>,
) -> Result<Prompt, String> {
    let mut prompt = Prompt::with_context(text, images, context);
    if let Some(value) = snapshot.filter(|value| !value.is_null()) {
        let mut snapshot: crate::composer_rich::Snapshot =
            serde_json::from_value(value).map_err(|_| "Invalid composer document.")?;
        let editor = snapshot.editor();
        validate_pill_kinds(&editor)?;
        if crate::composer_rich::markdown(&editor) != prompt.text {
            return Err("The composer document does not match the submitted message.".into());
        }
        snapshot.attachments = snapshot
            .attachments
            .iter()
            .map(|attachment| {
                prompt
                    .images
                    .iter()
                    .find(|image| image.id == attachment.id)
                    .cloned()
                    .ok_or("Composer attachment is not in the submitted message.")
            })
            .collect::<Result<_, _>>()?;
        if prompt.context.is_none() && !snapshot.context.is_empty() {
            let referenced = collect_inline_context_ids_from_edit(&editor);
            prompt.context = crate::composer_context::build_message_context(
                &referenced,
                &snapshot.context,
                &prompt.images,
            );
        }
        prompt.draft = Some(snapshot);
    }
    Ok(prompt)
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
    fn unknown_pill_kinds_require_an_update_without_discarding_the_document() {
        let mut edit = crate::composer_rich::draft("Future context");
        edit.replace_with_pill(
            0..14,
            r#"shina:{"kind":"futureContext","id":7}"#,
            "Future context",
        );
        assert!(prompt(&edit, &[]).unwrap_err().contains("Update required"));
        let snapshot = crate::composer_rich::Snapshot::capture(&edit, &[]);
        assert!(
            received_prompt(edit.text.clone(), vec![], Some(serde_json::json!(snapshot)))
                .unwrap_err()
                .contains("Update required")
        );
        assert_eq!(edit.pills().count(), 1);
    }
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
            source: None,
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
            machine: Some("local".into()),
            extra: Default::default(),
        };
        assert_eq!(
            thread.source(),
            "Shina thread \"Fix \\\"quoted\\\" title\", read_thread({\"thread_id\":42})"
        );
        assert_eq!(
            thread.source_for(Some("local"), &|m| m.to_owned()),
            "Shina thread \"Fix \\\"quoted\\\" title\", read_thread({\"thread_id\":42})"
        );
        assert_eq!(
            thread.source_for(Some("server-b"), &|m| if m == "local" {
                "Local".into()
            } else {
                m.into()
            }),
            "Shina thread \"Fix \\\"quoted\\\" title\", read_thread({\"machine\":\"local\",\"threadId\":42})"
        );

        // Missing owner field is treated as owned by its draft's machine; unknown fields are kept.
        let legacy_and_future =
            r#"shina:{"kind":"thread","id":9,"title":"Old","futureField":{"nested":1}}"#;
        let decoded = Token::decode(legacy_and_future)
            .expect("decode token with missing machine and unknown field");
        let mut edit = crate::composer_rich::draft("see @t");
        edit.replace_with_pill(4..6, decoded.id(), "Old");
        stamp_thread_owner(&mut edit, "server-a");
        let (_, stamped_id) = edit.pills().next().unwrap();
        let stamped_json: serde_json::Value =
            serde_json::from_str(stamped_id.strip_prefix("shina:").unwrap()).unwrap();
        assert_eq!(stamped_json["machine"], "server-a");
        assert_eq!(stamped_json["futureField"]["nested"], 1);
    }
}
