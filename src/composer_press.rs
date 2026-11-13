//! Pressing a composer pill previews its file, ported from KSG Code's
//! `ComposerMentionNodeView` (`onClick={() => actions.openMention(path)}`, label
//! `Preview {path}`) and `ComposerSkillNodeView`, which is pressable only when
//! the skill has a path. KSG opens the file in a right panel; Shina has no
//! right panel yet, so the attachment preview dialog is the settled substitute.
use crate::composer_tokens::Token;
use crate::model::State;
use crate::ui::attachment_preview::RemoteFile;
use serde_json::json;
use std::path::{Path, PathBuf};

/// What pressing a pill does. The mapping is pure so every token kind is
/// unit-tested against KSG's behavior; the effects live on `State`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PillAction {
    /// Open this path in the attachment preview dialog. It never joins the
    /// draft's attachments.
    Preview(PathBuf),
    /// KSG's `composerSourceLink`: pressing a citation chip's body scrolls the
    /// timeline to the cited passage.
    JumpToCitation(crate::composer_context::AssistantCitation),
    /// Directories have no preview in KSG's viewer, and thread, model and
    /// machine pills are inert there too.
    Nothing,
}

pub(crate) fn action(s: &State, id: &str) -> PillAction {
    if let Some((kind, context_id)) = crate::composer_context::parse_composer_context_href(id)
        && kind == "response"
        && let Some(record) = s
            .current()
            .draft_context
            .iter()
            .find(|r| r.context_id == context_id)
        && let crate::composer_context::ComposerContextData::Response { citation, .. } =
            &record.data
    {
        return PillAction::JumpToCitation(citation.clone());
    }
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
        if let PillAction::JumpToCitation(citation) = action(self, id) {
            // `on_input_pill_press` passes no `Tx`, so the jump waits a frame
            // for `composer_ui::tick`, which has one.
            self.composer_ui.pending_citation = Some(citation);
            return;
        }
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
        } else {
            self.fetch_remote_preview(RemoteFile {
                machine,
                cwd: cwd.unwrap_or_default(),
                path,
                request: 0,
            });
        }
    }

    /// Open a transient preview for one local path. Missing or unreadable
    /// files land on the dialog's existing failure state with retry.
    pub(crate) fn preview_mention_path(&mut self, path: &Path) {
        match crate::attachments::reference_file(path).and_then(|file| {
            crate::attachments::validate_file_metadata(&file.name, &file.mime, file.bytes)
                .map(|_| file)
        }) {
            Ok(attachment) => self.preview_mention(attachment, None),
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
                self.preview_mention(attachment, None);
                if let Some(preview) = &mut self.attachment_preview {
                    preview.error = Some(error);
                }
            }
        }
    }

    /// `workspace.readFile` keeps the same press working when the draft targets
    /// a linked machine; the reply's blob id downloads through `/v1/blobs`.
    pub(crate) fn fetch_remote_preview(&mut self, remote: RemoteFile) {
        let client = self
            .client
            .as_ref()
            .and_then(|client| client.linked.get(&remote.machine));
        let Some(client) = client else { return };
        if !client.has_capability(crate::protocol::WORKSPACE_READ_FILE) {
            let label = self.machine_label(&remote.machine);
            self.composer_notices.push(
                crate::composer_notices::Notice::new(
                    "mention-preview-update",
                    crate::composer_notices::Variant::Warning,
                    "alert",
                    format!("Update Shina on {label} to preview its files"),
                )
                .dismiss_label("Dismiss"),
            );
            return;
        }
        client.call_rpc(
            format!("mention:preview:{}", self.file_preview_seq + 1),
            "workspace.readFile",
            json!({"cwd": remote.cwd, "path": remote.path.to_string_lossy()}),
        );
        self.file_preview_seq += 1;
        let remote = RemoteFile {
            request: self.file_preview_seq,
            ..remote
        };
        let name = remote
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        // A foreign path must never be opened locally, so the placeholder
        // carries no path; the preview's `remote` lets Retry re-fetch.
        let attachment = placeholder(format!("mention-remote-{}", remote.request), name, None);
        self.preview_mention(attachment, Some(remote));
        if let Some(preview) = &mut self.attachment_preview {
            preview.loading = true;
        }
    }

    /// A `mention:preview:{request}` RPC answer: apply it only while the
    /// matching remote preview is still on screen.
    pub(crate) fn apply_file_preview_result(
        &mut self,
        request: u64,
        result: Result<serde_json::Value, String>,
    ) {
        let Some(preview) = &self.attachment_preview else {
            return;
        };
        let Some(remote) = preview.remote.clone() else {
            return;
        };
        if remote.request != request || preview.thread != self.current().id {
            return;
        }
        let cancel = preview.cancel.clone();
        let descriptor = result.and_then(|descriptor| {
            let id = descriptor["id"]
                .as_str()
                .filter(|id| crate::attachments::safe_file_id(id))
                .ok_or("Invalid remote file descriptor.")?
                .to_owned();
            let name = descriptor["name"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .ok_or("Invalid remote file descriptor.")?
                .to_owned();
            let mime = descriptor["mime"]
                .as_str()
                .filter(|mime| !mime.trim().is_empty())
                .ok_or("Invalid remote file descriptor.")?
                .to_owned();
            let bytes = descriptor["bytes"]
                .as_u64()
                .filter(|bytes| *bytes > 0 && *bytes <= crate::attachments::MAX_FILE_BYTES)
                .ok_or("Invalid remote file length.")?;
            Ok((id, name, mime, bytes))
        });
        let (id, name, mime, bytes) = match descriptor {
            Ok(descriptor) => descriptor,
            Err(error) => {
                if let Some(preview) = &mut self.attachment_preview {
                    preview.loading = false;
                    preview.error = Some(error);
                }
                return;
            }
        };
        let (Some(link), Some(handle)) = (
            self.attachment_preview_link_on(&remote.machine),
            self.background.handle.clone(),
        ) else {
            if let Some(preview) = &mut self.attachment_preview {
                preview.loading = false;
                preview.error =
                    Some("The environment it came from is not reachable right now.".into());
            }
            return;
        };
        let profile = self.data_dir.clone();
        std::thread::spawn(move || {
            let fetched = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
                .and_then(|runtime| {
                    runtime.block_on(crate::client::fetch_blob_file(
                        &link, &profile, &id, &name, &mime, bytes,
                    ))
                });
            handle.update(move |state, _| {
                let Some(preview) = &mut state.attachment_preview else {
                    return;
                };
                if !std::sync::Arc::ptr_eq(&preview.cancel, &cancel) {
                    return;
                }
                preview.loading = false;
                match fetched {
                    Ok(attachment) => {
                        preview.items[preview.index] = attachment;
                        preview.load(state.wake.clone());
                        state.finish_preview_load();
                    }
                    Err(error) => preview.error = Some(error),
                }
            });
        });
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
        assert!(preview.transient && preview.remote.is_none() && preview.error.is_none());
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
