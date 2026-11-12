//! Ported from KSG Code's useQueuedRunEdit, queuedMessageEdit and queuedRunEdit store.
use super::*;

pub struct QueuedEdit {
    pub input_id: usize,
    pub(super) identity: uuid::Uuid,
    generation: String,
    revision: String,
    original: crate::composer_rich::Snapshot,
    pub(super) ordinary: TextEdit,
    pub(super) ordinary_images: Vec<Attachment>,
    ordinary_error: Option<String>,
}

pub(super) fn queue_revision(prompt: &Prompt) -> String {
    crate::protocol::revision(
        &json!({"text":prompt.text,"attachments":prompt.images.as_ref(),
        "draft":prompt.draft}),
    )
}

impl State {
    pub(super) fn apply_queue_receipt(
        &mut self,
        server: &str,
        command: &crate::protocol::Command,
        value: &Value,
    ) {
        let threads = if server == self.machines.selected {
            Some(&mut self.threads)
        } else {
            self.machines.threads.get_mut(server)
        };
        let Some(thread) = threads.and_then(|threads| {
            threads
                .iter_mut()
                .find(|thread| json!(thread.id) == command.params["threadId"])
        }) else {
            return;
        };
        if command.method == "queue.update" {
            thread.finish_queued_save(command.command_id, value["result"]["status"] == "accepted");
        } else if value["result"]["status"] == "accepted" {
            thread.cancel_queued_edit();
            for prompt in value["result"]["result"]["prompts"]
                .as_array()
                .into_iter()
                .flatten()
                .rev()
            {
                if let Ok(images) = serde_json::from_value(prompt["attachments"].clone())
                    && let Ok(prompt) = crate::composer_tokens::received_prompt(
                        string(prompt, "text").into(),
                        images,
                        prompt.get("draft").cloned(),
                    )
                {
                    thread.restore_prompt(&prompt);
                }
            }
        }
    }
}

impl Thread {
    pub fn queued_edit_saving(&self) -> bool {
        self.queued_save.is_some()
    }
    /// Editing is entirely client-owned until Save. The queue item remains runnable.
    pub fn edit_queued(&mut self, id: usize) {
        if self.request.is_some() {
            self.error = Some("Answer the question to edit queued messages".into());
            return;
        }
        if !self.queue_edit_supported {
            self.error = Some(crate::protocol::COMPOSER_UPDATE_REQUIRED.into());
            return;
        }
        let Some(prompt) = self
            .native
            .queue
            .iter()
            .find(|q| q.id == id)
            .map(|q| q.prompt.clone())
        else {
            return;
        };
        self.cancel_queued_edit();
        let original = prompt.draft.clone().unwrap_or_else(|| {
            crate::composer_rich::Snapshot::capture(
                &crate::composer_rich::draft(&prompt.text),
                &prompt.images,
            )
        });
        let ordinary = std::mem::replace(&mut self.draft, original.editor());
        let ordinary_images =
            std::mem::replace(&mut self.draft_images, original.attachments.clone());
        self.queued_edit = Some(QueuedEdit {
            input_id: id,
            identity: uuid::Uuid::new_v4(),
            generation: self.request_generation.clone(),
            revision: queue_revision(&prompt),
            original,
            ordinary,
            ordinary_images,
            ordinary_error: self.attachment_error.take(),
        });
        self.draft.focused = true;
        self.draft.set_caret(self.draft.text.len());
        self.prompt_history.reset();
        self.paste_history = Default::default();
        self.composer_rules = Default::default();
    }

    pub fn cancel_queued_edit(&mut self) {
        if let Some(edit) = self.queued_edit.take() {
            self.draft = edit.ordinary;
            self.draft_images = edit.ordinary_images;
            self.attachment_error = edit.ordinary_error;
            self.draft.focused = true;
            self.prompt_history.reset();
            self.paste_history = Default::default();
            self.composer_rules = Default::default();
        }
    }

    /// A saving edit pauses the queue-exit watcher. Its receipt decides the outcome.
    pub(super) fn reconcile_queued_edit(&mut self) {
        let Some(edit) = &self.queued_edit else {
            return;
        };
        if self
            .queued_save
            .is_some_and(|(_, identity)| identity == edit.identity)
            || edit.generation == self.request_generation
                && self.native.queue.iter().any(|q| q.id == edit.input_id)
        {
            return;
        }
        let changed = crate::composer_rich::Snapshot::capture(&self.draft, &self.draft_images)
            != edit.original;
        let recover = changed
            && self.has_draft_content()
            && edit.ordinary.text.trim().is_empty()
            && edit.ordinary_images.is_empty();
        let editor = self.draft.clone();
        let images = self.draft_images.clone();
        self.cancel_queued_edit();
        if recover {
            self.draft = editor;
            self.draft_images = images;
        }
        if changed {
            self.error = Some(format!(
                "Queued message is no longer queued. Your unsaved edit was {}.",
                if recover {
                    "kept in the composer"
                } else {
                    "discarded"
                }
            ));
        }
    }

    /// The server serializes updates with run start/removal and compares runtime and content IDs.
    pub(super) fn update_queued(
        &mut self,
        id: usize,
        generation: &str,
        revision: &str,
        prompt: Prompt,
    ) -> Result<(), String> {
        if generation != self.request_generation {
            return Err("The queue belongs to an earlier runtime generation.".into());
        }
        if prompt.text.trim().is_empty() && prompt.images.is_empty() {
            return Err("A queued message cannot be empty.".into());
        }
        attachments::validate_prompt_images(&prompt.images)?;
        let queued = self
            .native
            .queue
            .iter_mut()
            .find(|q| q.id == id)
            .ok_or("This message is no longer queued.")?;
        if queue_revision(&queued.prompt) != revision {
            return Err(
                "This queued message was changed by another client. Cancel and edit it again."
                    .into(),
            );
        }
        queued.prompt = prompt;
        Ok(())
    }

    pub fn save_queued_edit(&mut self) {
        if self.queued_save.is_some() || self.request.is_some() || !self.has_draft_content() {
            return;
        }
        let Some(edit) = &self.queued_edit else {
            return;
        };
        let id = edit.input_id;
        let identity = edit.identity;
        let generation = edit.generation.clone();
        let revision = edit.revision.clone();
        let existing: Vec<_> = self
            .native
            .queue
            .iter()
            .find(|q| q.id == id)
            .map(|q| q.prompt.images.iter().map(|a| a.id.clone()).collect())
            .unwrap_or_default();
        let prompt = match crate::composer_tokens::prompt_for(
            &self.draft,
            &self.draft_images,
            Some(&self.machine),
            &str::to_owned,
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.attachment_error = Some(error);
                return;
            }
        };
        if let Some(remote) = &self.remote {
            match remote.send(
                "queue.update",
                json!({"threadId":self.id,"inputId":id,
                "generation":generation,"expectedRevision":revision,"text":prompt.text,
                "attachments":prompt.images.as_ref(),"draft":prompt.draft,"existingAttachmentIds":existing}),
            ) {
                Ok(command) => self.queued_save = Some((command, identity)),
                Err(error) => self.error = Some(error),
            }
        } else {
            match self.update_queued(id, &generation, &revision, prompt) {
                Ok(()) => self.cancel_queued_edit(),
                Err(error) => {
                    self.error = Some(error);
                    self.reconcile_queued_edit();
                }
            }
        }
    }

    pub(super) fn finish_queued_save(&mut self, command: uuid::Uuid, accepted: bool) {
        let Some((pending, identity)) = self.queued_save else {
            return;
        };
        if pending != command {
            return;
        }
        self.queued_save = None;
        if self
            .queued_edit
            .as_ref()
            .is_none_or(|edit| edit.identity != identity)
        {
            return;
        }
        if accepted {
            self.cancel_queued_edit();
        } else {
            self.error = Some("Could not save the edited queued message.".into());
            self.reconcile_queued_edit();
        }
    }
}
