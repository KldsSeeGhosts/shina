//! KSG question attachment drafts, preparation, and native answer references.
use super::*;
use crate::composer_context::{
    ComposerContextData, ComposerContextRecord, RetainedContextPayloads,
};
use crate::protocol::Command;

#[derive(Clone, Default)]
pub(super) struct Draft {
    pub editor: TextEdit,
    pub files: Vec<Attachment>,
    pub context: Vec<ComposerContextRecord>,
    retained: RetainedContextPayloads,
    pub pending: usize,
}

pub(super) struct Submission {
    request: Value,
    generation: String,
    receiver: mpsc::Receiver<Result<Command, String>>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Resolution {
    pub request: Value,
    pub generation: String,
    pub reason: String,
}

impl Thread {
    /// KSG questionAttachments.returnUnsentAnswersToThreadDraft.
    pub(super) fn abandon_question(&mut self, reason: &str) {
        let Some(request) = self.request.as_ref().map(|r| r["id"].clone()) else {
            return;
        };
        if self.native.questionnaire.is_none() {
            return;
        }
        self.return_unsent_question_answers();
        if !self
            .question_resolutions
            .iter()
            .any(|r| r.request == request && r.generation == self.request_generation)
        {
            self.question_resolutions.push(Resolution {
                request,
                generation: self.request_generation.clone(),
                reason: reason.into(),
            });
        }
    }

    pub(super) fn return_unsent_question_answers(&mut self) {
        self.save_question_editor();
        let Some(form) = &self.native.questionnaire else {
            return;
        };
        let mut drafts = Vec::new();
        let mut contexts = Vec::new();
        for (at, answer) in form.answers.iter().enumerate() {
            let text = answer.other.trim();
            if !answer.touched || text.is_empty() {
                continue;
            }
            drafts.push(text.to_owned());
            let references = crate::composer_context::collect_composer_context_references(text);
            if let Some(draft) = self
                .question_scope_at(at)
                .and_then(|key| self.question_drafts.get(&key))
            {
                contexts.extend(
                    draft
                        .context
                        .iter()
                        .filter(|r| {
                            r.kind() == "thread"
                                && references
                                    .iter()
                                    .any(|reference| reference.context_id == r.context_id)
                        })
                        .cloned(),
                );
            }
        }
        if drafts.is_empty() {
            return;
        }
        let ordinary = crate::composer_rich::markdown(&self.draft);
        if ordinary.trim().is_empty() {
            self.draft = crate::composer_rich::draft_from_canonical(&drafts.join("\n\n"));
        } else {
            let end = self.draft.text.trim_end().len();
            self.draft.set_selection(end, self.draft.text.len());
            self.draft.insert("");
            self.draft = crate::composer_rich::prepend(
                &self.draft,
                &crate::composer_rich::draft_from_canonical(&drafts.join("\n\n")),
            );
        }
        for record in contexts {
            if !self
                .draft_context
                .iter()
                .any(|r| r.context_id == record.context_id)
            {
                self.draft_context.push(record);
            }
        }
    }
    pub(super) fn current_question_text(&self) -> String {
        if self
            .native
            .questionnaire
            .as_ref()
            .is_some_and(|f| f.questions[f.index].initial_answer.is_some())
        {
            self.reply.text.clone()
        } else {
            crate::composer_rich::markdown(&self.reply)
        }
    }
    pub(crate) fn question_submission_pending(&self) -> bool {
        self.question_submit_requested || self.question_submission.is_some()
    }
    pub(crate) fn question_scope(&self) -> Option<String> {
        if self.request_held_by_typing_guard() {
            return None;
        }
        let form = self.native.questionnaire.as_ref()?;
        Some(
            json!([
                self.request_generation,
                self.request.as_ref()?["id"],
                form.questions[form.index].id
            ])
            .to_string(),
        )
    }

    fn question_scope_at(&self, index: usize) -> Option<String> {
        Some(
            json!([
                self.request_generation,
                self.request.as_ref()?["id"],
                self.native.questionnaire.as_ref()?.questions.get(index)?.id
            ])
            .to_string(),
        )
    }

    fn question_scope_valid(&self, scope: &str) -> bool {
        self.native.questionnaire.as_ref().is_some_and(|form| {
            (0..form.questions.len()).any(|i| self.question_scope_at(i).as_deref() == Some(scope))
        })
    }

    pub(crate) fn question_files(&self) -> &[Attachment] {
        self.question_scope()
            .and_then(|key| self.question_drafts.get(&key))
            .map_or(&[], |draft| draft.files.as_slice())
    }

    pub(crate) fn question_reserved_count(&self) -> usize {
        self.question_drafts
            .iter()
            .filter(|(key, _)| self.question_scope_valid(key))
            .map(|(key, d)| {
                d.pending
                    + if self.answer_intake && self.answer_scope.as_ref() == Some(key) {
                        0
                    } else {
                        d.files.len()
                    }
            })
            .sum::<usize>()
            + if self.answer_intake {
                self.draft_images.len()
            } else {
                0
            }
    }

    pub(super) fn save_question_editor(&mut self) {
        let Some(key) = self.question_scope() else {
            return;
        };
        let text = self.current_question_text();
        let draft = self.question_drafts.entry(key).or_default();
        draft.editor = self.reply.clone();
        if let Some(form) = &mut self.native.questionnaire {
            if form.answers[form.index].other != text {
                form.edit(text);
            }
            form.answers[form.index].attachment_count = draft.files.len();
            form.answers[form.index].attachments_blocked = draft.pending > 0;
        }
    }

    pub(super) fn load_question_editor(&mut self) {
        if let Some(draft) = self
            .question_scope()
            .and_then(|key| self.question_drafts.get(&key))
        {
            self.reply = draft.editor.clone();
            self.reply.focused = false;
        }
    }

    pub(super) fn request_question_submission(&mut self) -> bool {
        self.save_question_editor();
        let has_files = self.question_drafts.iter().any(|(key, d)| {
            self.question_scope_valid(key)
                && (!d.files.is_empty() || !d.context.is_empty() || d.pending > 0)
        });
        if !has_files {
            return false;
        }
        if self
            .question_drafts
            .iter()
            .any(|(key, d)| self.question_scope_valid(key) && d.pending > 0)
        {
            self.attachment_error = Some("Still bringing attached context into this message. Send again once its chip resolves.".into());
        } else {
            self.question_submit_requested = true;
        }
        true
    }

    /// Receives server-owned files, never paths on the answering desktop.
    pub(super) fn respond_question_with_files(
        &mut self,
        answers: Value,
        files: &Value,
    ) -> Result<(), String> {
        if !files.is_null()
            && (!files.is_object()
                || files
                    .as_object()
                    .is_some_and(|m| m.values().any(|v| !v.is_array())))
        {
            return Err("Invalid question attachments.".into());
        }
        let answers: Vec<pi_session::questionnaire::Answer> =
            serde_json::from_value(answers).map_err(|e| e.to_string())?;
        let form = self
            .native
            .questionnaire
            .as_mut()
            .ok_or("Questionnaire expired.")?;
        if answers.len() != form.questions.len() || answers.is_empty() {
            return Err("Invalid questionnaire answers.".into());
        }
        let mut by_question = Vec::new();
        let mut all = Vec::new();
        for question in &form.questions {
            let mut owned = Vec::new();
            if let Some(values) = files.get(&question.id).and_then(Value::as_array) {
                if values.len() > attachments::MAX_ATTACHMENTS {
                    return Err(format!(
                        "You can attach up to {} files per message.",
                        attachments::MAX_ATTACHMENTS
                    ));
                }
                if !question.allow_custom && !values.is_empty() {
                    return Err("This question cannot accept attachments.".into());
                }
                for value in values {
                    let id = value["id"].as_str().ok_or("Invalid answer attachment.")?;
                    if !attachments::safe_id(id) && !attachments::safe_file_id(id) {
                        return Err("Answer attachments must be uploaded first.".into());
                    }
                    let root = attachments::directory()
                        .ok_or("The attachment directory is unavailable.")?
                        .canonicalize()
                        .map_err(|e| e.to_string())?;
                    let path = root.join(id).canonicalize().map_err(|_| {
                        format!(
                            "Attachment '{}' is no longer available. Attach it again.",
                            value["name"].as_str().unwrap_or("file")
                        )
                    })?;
                    if !path.starts_with(&root) {
                        return Err("Invalid answer attachment path.".into());
                    }
                    let mut value = value.clone();
                    if attachments::safe_file_id(id) {
                        value["path"] = json!(path);
                    }
                    let attachment =
                        Attachment::from_value(&value).ok_or("Invalid answer attachment.")?;
                    if attachment.is_image() {
                        attachments::read(&attachment)?;
                    } else {
                        attachments::transfer_file_bytes(&attachment)?;
                    }
                    all.push(attachment.clone());
                    owned.push(attachment);
                }
            }
            by_question.push((question.id.clone(), owned));
        }
        for file in &all {
            attachments::validate_prompt_images(std::slice::from_ref(file))?;
        }
        form.answers = answers;
        for (question, answer) in form.questions.iter().zip(&mut form.answers) {
            answer.attachment_count = by_question
                .iter()
                .find(|(id, _)| id == &question.id)
                .map_or(0, |(_, files)| files.len());
            answer.attachments_blocked = false;
            if answer
                .selected
                .iter()
                .any(|value| !question.options.iter().any(|o| o.value == *value))
            {
                return Err("Invalid questionnaire option.".into());
            }
        }
        let mut normalized = form
            .resolved_answers()
            .ok_or("Answer each question before sending.")?;
        for (id, files) in by_question {
            let references = files
                .iter()
                .map(|file| {
                    let path = attachments::path_for(file)
                        .ok_or("The attachment is no longer available.")?;
                    let extracted = crate::question_document::ensure_named(&path, &file.name);
                    Ok(format!(
                        "Attached {} {}: {}{}",
                        if file.is_image() { "image" } else { "file" },
                        json!(file.name),
                        json!(path.to_string_lossy()),
                        extracted
                            .map(|path| format!(
                                " (text extracted to: {})",
                                json!(path.to_string_lossy())
                            ))
                            .unwrap_or_default()
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?
                .join("\n");
            if references.is_empty() {
                continue;
            }
            let answer = normalized.get_mut(&id).ok_or("Unknown answer question.")?;
            append_references(answer, references);
        }
        let response = form
            .response_with_answers(&normalized)
            .ok_or("This request is not resumable.")?;
        let request = self.request.as_ref().map(|r| r["id"].clone());
        self.respond(response);
        if self.request.as_ref().map(|r| r["id"].clone()) == request {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "Could not answer request.".into()));
        }
        self.question_drafts.clear();
        Ok(())
    }
}

pub(super) fn append_references(answer: &mut Value, references: String) {
    if let Some(array) = answer.as_array_mut() {
        array.push(json!(references));
    } else if let Some(text) = answer.as_str().filter(|s| !s.is_empty()) {
        *answer = json!(format!("{text}\n\n{references}"));
    } else {
        *answer = json!(references);
    }
}

fn project_threads(answer: &str, records: &[ComposerContextRecord]) -> String {
    crate::composer_context::replace_composer_context_references(answer, |reference| {
        let Some(record) = records
            .iter()
            .find(|r| r.context_id == reference.context_id)
        else {
            return reference.source.clone();
        };
        let ComposerContextData::Thread {
            environment_id,
            thread_id,
            ..
        } = &record.data
        else {
            return reference.source.clone();
        };
        let label = reference
            .label
            .chars()
            .map(|c| {
                if matches!(c, '[' | ']' | '\\' | '\r' | '\n') {
                    ' '
                } else {
                    c
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let href = format!("t3-thread://v2/{environment_id}/{thread_id}");
        let destination = if href.contains(['(', ')']) {
            format!("<{href}>")
        } else {
            href.clone()
        };
        if label.is_empty() {
            format!("<{href}>")
        } else {
            format!("[{label}]({destination})")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn question_reference_suffixes_preserve_string_and_array_answers() {
        for (mut answer, expected) in [
            (json!(""), json!("Attached file \"a\": \"path\"")),
            (
                json!("Answer"),
                json!("Answer\n\nAttached file \"a\": \"path\""),
            ),
            (
                json!(["B", "A"]),
                json!(["B", "A", "Attached file \"a\": \"path\""]),
            ),
        ] {
            append_references(&mut answer, "Attached file \"a\": \"path\"".into());
            assert_eq!(answer, expected);
        }
    }
    #[test]
    fn question_thread_references_project_only_referenced_records_to_native_links() {
        let records = vec![ComposerContextRecord {
            version: 1,
            context_id: "thread_ref".into(),
            label: "Linked".into(),
            data: ComposerContextData::Thread {
                environment_id: "environment".into(),
                thread_id: "12".into(),
                title: "Linked".into(),
            },
        }];
        assert_eq!(
            project_threads("See [linked](t3-context://v1/thread/thread_ref)", &records),
            "See [linked](t3-thread://v2/environment/12)"
        );
        assert_eq!(
            project_threads("Deleted chip retained for undo", &records),
            "Deleted chip retained for undo"
        );
    }
}

impl State {
    pub(crate) fn with_question_draft<R>(
        &mut self,
        index: usize,
        scope: String,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if !self.threads.get(index)?.question_scope_valid(&scope) {
            return None;
        }
        let current = self.threads[index].question_scope().as_ref() == Some(&scope);
        let mut answer = self.threads[index]
            .question_drafts
            .get(&scope)
            .cloned()
            .unwrap_or_default();
        if current {
            answer.editor = self.threads[index].reply.clone();
        }
        let t = &mut self.threads[index];
        let ordinary = Draft {
            editor: std::mem::replace(&mut t.draft, answer.editor),
            files: std::mem::replace(&mut t.draft_images, answer.files),
            context: std::mem::replace(&mut t.draft_context, answer.context),
            retained: std::mem::replace(&mut t.retained_context, answer.retained),
            pending: 0,
        };
        t.answer_intake = true;
        t.answer_scope = Some(scope.clone());
        let result = f(self);
        let t = &mut self.threads[index];
        t.answer_intake = false;
        t.answer_scope = None;
        let answer = Draft {
            editor: std::mem::replace(&mut t.draft, ordinary.editor),
            files: std::mem::replace(&mut t.draft_images, ordinary.files),
            context: std::mem::replace(&mut t.draft_context, ordinary.context),
            retained: std::mem::replace(&mut t.retained_context, ordinary.retained),
            pending: t.question_drafts.get(&scope).map_or(0, |d| d.pending),
        };
        if current {
            t.reply = answer.editor.clone();
        }
        if let Some(form) = &mut t.native.questionnaire {
            for at in 0..form.questions.len() {
                let key = json!([
                    t.request_generation,
                    t.request.as_ref().map(|r| &r["id"]),
                    form.questions[at].id
                ])
                .to_string();
                if key == scope {
                    let text = if form.questions[at].initial_answer.is_some() {
                        answer.editor.text.clone()
                    } else {
                        crate::composer_rich::markdown(&answer.editor)
                    };
                    if form.answers[at].other != text {
                        form.edit_at(at, text);
                    }
                    form.answers[at].attachment_count = answer.files.len();
                    form.answers[at].attachments_blocked = answer.pending > 0;
                }
            }
        }
        t.question_drafts.insert(scope, answer);
        self.checkpoint();
        Some(result)
    }

    pub(crate) fn question_attachments_allowed(&self, index: usize) -> bool {
        let t = &self.threads[index];
        t.questionnaire_can_respond()
            && t.native
                .questionnaire
                .as_ref()
                .is_some_and(|f| f.questions[f.index].allow_custom)
            && self.client.as_ref().is_none_or(|c| {
                c.supports_capability(
                    &self.machines.selected,
                    crate::protocol::QUESTION_ATTACHMENTS,
                )
            })
    }

    pub(super) fn poll_question_answers(&mut self) -> bool {
        let mut changed = false;
        for index in 0..self.threads.len() {
            if !std::mem::take(&mut self.threads[index].question_submit_requested) {
                continue;
            }
            let server = self.machines.selected.clone();
            if !self.threads[index].questionnaire_can_respond()
                || self.client.as_ref().is_some_and(|c| {
                    !c.supports_capability(&server, crate::protocol::QUESTION_ATTACHMENTS)
                })
            {
                self.threads[index].attachment_error =
                    Some("This question cannot accept attachments.".into());
                continue;
            }
            let t = &mut self.threads[index];
            let Some(request) = t.request.clone() else {
                continue;
            };
            let Some(form) = &t.native.questionnaire else {
                continue;
            };
            let mut answers = form.answers.clone();
            let mut files = Vec::new();
            let mut counts = Vec::new();
            for (at, q) in form.questions.iter().enumerate() {
                let draft = t
                    .question_scope_at(at)
                    .and_then(|key| t.question_drafts.get(&key));
                if let Some(draft) = draft {
                    answers[at].other = project_threads(&answers[at].other, &draft.context);
                    files.extend(draft.files.iter().cloned());
                    counts.push((q.id.clone(), draft.files.len()));
                } else {
                    counts.push((q.id.clone(), 0));
                }
            }
            let mut command = Command::new(
                "request.respond",
                json!({"threadId":t.id,"requestId":request["id"],
                "generation":request["runtimeGeneration"].as_str().unwrap_or(&t.request_generation),
                "requestContract":true,"questionContract":true,"questionAttachments":true,
                "answers":answers,"response":{"value":""},"questionTextById":form.questions.iter()
                    .map(|q|(q.id.clone(),json!(q.text))).collect::<serde_json::Map<_,_>>()}),
            );
            let remote = self
                .client
                .as_ref()
                .and_then(|c| {
                    if server == "local" {
                        Some(c)
                    } else {
                        c.linked.get(&server)
                    }
                })
                .and_then(|c| c.remote.clone());
            let supported = self
                .client
                .as_ref()
                .is_none_or(|c| c.supports_capability(&server, crate::protocol::FILE_ATTACHMENTS));
            let directory = self.data_dir.clone();
            let (tx, rx) = mpsc::channel();
            let wake = self.wake.clone();
            t.question_submission = Some(Submission {
                request: request["id"].clone(),
                generation: t.request_generation.clone(),
                receiver: rx,
            });
            std::thread::spawn(move || {
                let prepare = || -> Result<Command, String> {
                    let mut prompt = Prompt::new(String::new(), files);
                    if remote.is_none() {
                        let mut staged = Vec::new();
                        for file in prompt.images.iter() {
                            let mut single = Prompt::new(String::new(), vec![file.clone()]);
                            attachments::stage_prompt_files(&mut single)?;
                            staged.extend(single.images.iter().cloned());
                        }
                        prompt.images = staged.into();
                    }
                    if remote.is_none() {
                        for file in prompt.images.iter() {
                            if let Some(path) = attachments::path_for(file) {
                                crate::question_document::ensure_named(&path, &file.name);
                            }
                        }
                    }
                    let upload = Command::new(
                        "thread.send",
                        json!({"text":"","attachments":prompt.images.as_ref(),
                        "draft":crate::composer_rich::Snapshot::capture(&TextEdit::default(),&prompt.images)}),
                    );
                    let prepared = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| e.to_string())?
                        .block_on(crate::client::uploaded_command(
                            &upload,
                            remote.as_ref(),
                            supported,
                        ))?;
                    let files = prepared.params["attachments"]
                        .as_array()
                        .ok_or("Invalid answer attachments.")?;
                    let mut offset = 0;
                    let mut by_question = serde_json::Map::new();
                    for (id, count) in counts {
                        by_question.insert(id, json!(files[offset..offset + count]));
                        offset += count;
                    }
                    command.params["attachmentsByQuestionId"] = json!(by_question);
                    Ok(command)
                };
                #[cfg(test)]
                let result = crate::store::with_directory(directory, prepare);
                #[cfg(not(test))]
                let result = {
                    let _ = directory;
                    prepare()
                };
                let _ = tx.send(result);
                wake();
            });
            changed = true;
        }
        let mut results = Vec::new();
        for (server, threads) in std::iter::once((&self.machines.selected, &mut self.threads))
            .chain(self.machines.threads.iter_mut())
        {
            for t in threads {
                if let Some(submission) = t.question_submission.take() {
                    match submission.receiver.try_recv() {
                        Ok(result) => results.push((
                            server.clone(),
                            t.id,
                            submission.request,
                            submission.generation,
                            result,
                        )),
                        Err(mpsc::TryRecvError::Empty) => t.question_submission = Some(submission),
                        Err(_) => {
                            t.attachment_error =
                                Some("Could not prepare question attachments.".into())
                        }
                    }
                }
            }
        }
        for (server, id, request, generation, result) in results {
            let threads = if server == self.machines.selected {
                Some(&mut self.threads)
            } else {
                self.machines.threads.get_mut(&server)
            };
            let Some(t) = threads.and_then(|ts| ts.iter_mut().find(|t| t.id == id)) else {
                continue;
            };
            if t.request_generation != generation
                || !t.request.as_ref().is_some_and(|r| r["id"] == request)
            {
                continue;
            }
            changed = true;
            match result {
                Err(error) => t.attachment_error = Some(error),
                Ok(command) => {
                    if let Some(client) = &self.client {
                        let selected = if server == "local" {
                            Some(client)
                        } else {
                            client.linked.get(&server)
                        };
                        match selected
                            .ok_or("This machine is disconnected.".to_owned())
                            .and_then(|c| c.send_prepared(command, self.wake.clone()))
                        {
                            Ok(id) => t.responding_request = Some(id),
                            Err(error) => t.error = Some(error),
                        }
                    } else if let Err(error) = t.respond_question_with_files(
                        command.params["answers"].clone(),
                        &command.params["attachmentsByQuestionId"],
                    ) {
                        t.attachment_error = Some(error);
                    }
                }
            }
        }
        changed
    }
}
