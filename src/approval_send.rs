//! KSG useComposerSend's feedback and decline-before-redirect transaction.
use super::*;
use crate::{composer_rich::Snapshot, protocol::Command};

pub(super) struct ApprovalSend {
    shown_ids: Vec<Value>,
    remaining_ids: Option<Vec<Value>>,
    pending_before: Vec<Value>,
    generation: String,
    snapshot: Snapshot,
    editor: TextEdit,
    feedback: bool,
    background: bool,
    preparation: Option<mpsc::Receiver<Result<Command, String>>>,
    prepared: Option<Command>,
    responded: bool,
    response_command: Option<uuid::Uuid>,
    response_result: Option<bool>,
    response_draft: Option<(TextEdit, Snapshot)>,
}

fn same_editor(left: &TextEdit, right: &TextEdit) -> bool {
    if left.is_composing() {
        return false;
    }
    let normalize = |edit: &TextEdit| {
        let mut edit = edit.clone();
        edit.anchor = 0;
        edit.focus = 0;
        edit.focused = false;
        edit
    };
    normalize(left) == normalize(right)
}

impl Thread {
    pub(crate) fn approval_send_pending(&self) -> bool {
        self.approval_send.is_some()
    }

    pub(super) fn write_approval_response(
        &mut self,
        request: &Value,
        response: Value,
    ) -> Result<(), String> {
        crate::approval::validate(request, &response)?;
        let decision = crate::approval::decision(&response)?;
        let feedback = response["feedback"]
            .as_str()
            .map(str::trim)
            .map(str::to_owned);
        let mut wire = if self.harness == "pi" {
            match decision {
                "accept" | "acceptForSession" => json!({"confirmed":true}),
                "decline" => json!({"confirmed":false}),
                _ => json!({"cancelled":true}),
            }
        } else {
            crate::approval::response(decision)
        };
        if let Some(text) = &feedback {
            wire["feedback"] = json!(text);
        }
        wire["type"] = json!("extension_ui_response");
        wire["id"] = request["id"].clone();
        self.pi
            .as_ref()
            .ok_or("The session is unavailable. Retry to resume it.")?
            .send(wire)?;
        if decision == "acceptForSession" && self.harness == "pi" {
            self.remember_session_approval_for(request);
        }
        if let Some(text) = feedback {
            self.messages.push(Message {
                id: format!(
                    "approval-feedback:{}:{}",
                    self.request_generation, request["id"]
                ),
                role: "You",
                text,
                images: vec![],
                context: None,
            });
        }
        Ok(())
    }
    fn approval_draft_unchanged(&self, send: &ApprovalSend) -> bool {
        if self.request_generation != send.generation || self.draft.is_composing() {
            return false;
        }
        if let Some((editor, snapshot)) = &send.response_draft {
            same_editor(&self.draft, editor)
                && Snapshot::capture_with_context(
                    &self.draft,
                    &self.draft_images,
                    &self.draft_context,
                ) == *snapshot
        } else {
            crate::composer_rich::markdown(&self.draft)
                == crate::composer_rich::markdown(&send.editor)
                && self.draft_images.iter().map(|a| &a.id).eq(send
                    .snapshot
                    .attachments
                    .iter()
                    .map(|a| &a.id))
        }
    }
}

impl State {
    pub(super) fn respond_to_approval_id(
        &mut self,
        thread_id: usize,
        request_id: &Value,
        response: Value,
    ) -> Result<(), String> {
        let index = self
            .threads
            .iter()
            .position(|t| t.id == thread_id && !t.archived())
            .ok_or("The thread is unavailable.")?;
        if self.threads[index]
            .request
            .as_ref()
            .is_some_and(|r| r["id"] == *request_id)
        {
            return self.respond_to_request(thread_id, response);
        }
        let request = self.threads[index]
            .pending_approvals()
            .into_iter()
            .find(|r| r["id"] == *request_id)
            .ok_or("Request expired.")?;
        crate::approval::validate(&request, &response)?;
        if self.threads[index].responding_request.is_some() {
            return Err("A response to this request is already in flight.".into());
        }
        if let Some(client) = &self.client {
            if !client
                .supports_capability(&self.machines.selected, crate::protocol::APPROVAL_FEEDBACK)
            {
                return Err(crate::protocol::APPROVAL_FEEDBACK_UPDATE_REQUIRED.into());
            }
            if !client.can_approve(&self.machines.selected) {
                return Err("This connection cannot answer requests.".into());
            }
            let command = client.commands_for(&self.machines.selected).ok_or("This machine is not linked.")?
                .send("request.respond", json!({"threadId":thread_id,"requestId":request_id,
                    "generation":request["runtimeGeneration"],"requestContract":true,"response":response}))?;
            self.threads[index].responding_request = Some(command);
        } else {
            self.threads[index].respond_queued_approval(request_id, response)?;
            self.checkpoint();
        }
        Ok(())
    }
    fn approval_send_command(&self) -> Option<(String, String)> {
        let t = self.current();
        let text = crate::composer_rich::markdown(&t.draft).trim().to_owned();
        if t.draft
            .pills()
            .any(|(r, _)| t.draft.text[..r.start].trim().is_empty())
            || crate::composer_tokens::starts_with_file_path(&text, &t.draft_images)
        {
            return None;
        }
        let mut parts = text.splitn(2, char::is_whitespace);
        let command = parts.next().filter(|s| s.starts_with('/'))?.to_owned();
        Some((command, parts.next().unwrap_or("").trim().to_owned()))
    }

    pub(super) fn approval_send_is_local_command(&self) -> bool {
        self.approval_send_command().is_some_and(|(command, _)| {
            command == "/model"
                || command == "/compact"
                    && crate::harness::supports_compaction(&self.current().harness)
                || matches!(command.as_str(), "/goal" | "/feedback")
                    && self.current().harness == "codex"
        })
    }

    pub(super) fn approval_send_command_error(&self) -> Option<String> {
        let (command, _) = self.approval_send_command()?;
        (!self.approval_send_is_local_command()
            && !self
                .current()
                .native
                .commands
                .iter()
                .any(|c| c.name == command.trim_start_matches('/')))
        .then(|| {
            format!(
                "Unknown {} command {command}. Use / to browse the workspace's commands.",
                if self.current().harness == "pi" {
                    "Pi"
                } else {
                    "ACP"
                }
            )
        })
    }

    /// Local commands are consumed before any approval decision, just as in useComposerSend.
    pub(super) fn handle_composer_local_command(&mut self) -> bool {
        if !self.approval_send_is_local_command() {
            return false;
        }
        let (command, arguments) = self.approval_send_command().unwrap();
        if command == "/model" {
            self.choose_slash("model".into());
            return true;
        }
        if command == "/compact" {
            if !self.current().draft_images.is_empty() {
                self.current_mut().attachment_error =
                    Some("Compaction does not accept attachments. Your draft was kept.".into());
            } else if self.current().can_compact() {
                self.current_mut().draft.replace_text("");
                self.compact_context((!arguments.is_empty()).then_some(arguments));
            }
            return true;
        }
        if self.current().harness_state["codexNativeExtensions"] != true {
            self.current_mut().error = Some("This Codex version has no verified native command protocol. Update Codex outside Shina.".into());
            return true;
        }
        if self.current().status.busy()
            && command == "/goal"
            && !matches!(arguments.as_str(), "" | "pause" | "clear")
        {
            self.current_mut().error = Some(
                "Wait for Codex to finish before running this command. Your draft was kept.".into(),
            );
            return true;
        }
        if !self.current().draft_images.is_empty() {
            self.current_mut().attachment_error =
                Some("Native commands do not accept attachments. Your draft was kept.".into());
            return true;
        }
        if self.current_mut().command(
            "native_command",
            json!({
                "name":command.trim_start_matches('/'),"arguments":arguments
            }),
        ) {
            self.current_mut().draft.replace_text("");
        }
        true
    }

    pub(super) fn begin_approval_send(&mut self, background: bool) -> bool {
        let shown = self.current().shown_approvals();
        let Some(request) = shown.first() else {
            return false;
        };
        if self.current().approval_send.is_some() || !self.current().has_draft_content() {
            return false;
        }
        if self.current().responding_request.is_some() {
            self.approval_redirect_notice(
                "An approval is already being answered, so your message wasn't sent.",
            );
            return false;
        }
        let feedback = request["responseCapability"]["type"] == "live"
            && request["responseCapability"]["supportsFeedback"] == true;
        let dead = crate::approval::not_resumable(&request);
        if !crate::approval::can_respond(&request) && !dead {
            self.current_mut().error = Some("This request is not resumable.".into());
            return false;
        }
        if let Some(client) = &self.client {
            let capable = client
                .supports_capability(&self.machines.selected, crate::protocol::COMPOSER_REQUESTS);
            let feedback_capable = client
                .supports_capability(&self.machines.selected, crate::protocol::APPROVAL_FEEDBACK);
            if !capable || !feedback_capable {
                self.current_mut().error = Some(
                    if !capable {
                        crate::protocol::REQUESTS_UPDATE_REQUIRED
                    } else {
                        crate::protocol::APPROVAL_FEEDBACK_UPDATE_REQUIRED
                    }
                    .into(),
                );
                return false;
            }
            if !dead && !client.can_approve(&self.machines.selected) {
                self.current_mut().error = Some("This connection cannot answer requests.".into());
                return false;
            }
            if !client.can_operate(&self.machines.selected) {
                self.current_mut().error = Some("This connection cannot change threads.".into());
                return false;
            }
        }
        if self.headless {
            self.current_mut().error =
                Some("Answer the pending approval before sending a new message.".into());
            return false;
        }
        let server_id = self.machine_identity(&self.machines.selected);
        let prompt = match crate::composer_tokens::prompt_for_target(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
            Some(&self.machines.selected),
            Some(&server_id),
            &self.peers,
            &|m| self.machine_label(m),
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                self.current_mut().attachment_error = Some(error);
                return false;
            }
        };
        if feedback && (!prompt.images.is_empty() || prompt.context.is_some()) {
            self.current_mut().error =
                Some("Send approval feedback without attachments or added context.".into());
            return false;
        }
        if let Err(error) = attachments::validate_prompt_images(&prompt.images) {
            self.current_mut().attachment_error = Some(error);
            return false;
        }
        if let Some(error) = self.current().validate_prompt_length(&prompt) {
            self.current_mut().submission_error = Some(error);
            return false;
        }
        if prompt.images.iter().any(Attachment::is_image)
            && (self.current().supports_images == Some(false)
                || self.current().status.busy() && self.current().supports_images != Some(true))
        {
            self.current_mut().attachment_error = Some(self.current().image_model_error());
            return false;
        }
        let snapshot = Snapshot::capture_with_context(
            &self.current().draft,
            &self.current().draft_images,
            &self.current().draft_context,
        );
        let command = Command::new(
            "thread.send",
            json!({
                "threadId":self.current().id,"text":prompt.text,"attachments":prompt.images.as_ref(),
                "context":prompt.context,"draft":prompt.draft,
                "editorRevision":uuid::Uuid::new_v4(),
                "mode":if self.current().status.busy() {
                    if self.current().can_steer() {"steer"} else {"queue"}
                } else {"prompt"}
            }),
        );
        let (sender, receiver) = mpsc::channel();
        if let Some(client) = &self.client {
            let selected = if self.machines.selected == "local" {
                Some(client)
            } else {
                client.linked.get(&self.machines.selected)
            };
            let Some(selected) = selected.filter(|client| client.commands.connected()) else {
                self.current_mut().error =
                    Some("This machine is disconnected. Your draft was kept.".into());
                return false;
            };
            let link = selected.remote.clone();
            let files = client
                .supports_capability(&self.machines.selected, crate::protocol::FILE_ATTACHMENTS);
            let wake = self.wake.clone();
            std::thread::spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| e.to_string())
                    .and_then(|rt| {
                        rt.block_on(crate::client::uploaded_command(
                            &command,
                            link.as_ref(),
                            files,
                        ))
                    });
                let _ = sender.send(result);
                wake();
            });
        } else {
            let mut prepared = command;
            let result = (|| {
                let attachments = prompt.images.to_vec();
                let mut staged = Prompt::new(prompt.text, attachments);
                staged.context = prompt.context;
                staged.draft = prompt.draft;
                attachments::stage_prompt_files(&mut staged)?;
                prepared.params["attachments"] = json!(staged.images.as_ref());
                prepared.params["context"] = json!(staged.context);
                prepared.params["draft"] = json!(staged.draft);
                Ok(prepared)
            })();
            let _ = sender.send(result);
        }
        let t = self.current_mut();
        t.approval_send = Some(ApprovalSend {
            shown_ids: shown.iter().map(|r| r["id"].clone()).collect(),
            remaining_ids: None,
            pending_before: vec![],
            generation: t.request_generation.clone(),
            editor: t.draft.clone(),
            snapshot,
            feedback,
            background,
            preparation: Some(receiver),
            prepared: None,
            responded: false,
            response_command: None,
            response_result: None,
            response_draft: None,
        });
        t.typing_guard.on_blur_or_send();
        false
    }

    fn approval_redirect_notice(&mut self, title: &str) {
        self.push_toast(
            ToastKind::Info,
            title,
            Some("Send again when it's ready.".into()),
            None,
            Some(5_000),
        );
    }

    pub(super) fn approval_response_receipt(&mut self, command: &Command, value: &Value) {
        for t in self
            .threads
            .iter_mut()
            .chain(self.machines.threads.values_mut().flatten())
        {
            if let Some(send) = &mut t.approval_send
                && send.response_command == Some(command.command_id)
            {
                send.response_result = Some(value["result"]["status"] == "accepted");
            }
        }
    }

    pub(super) fn poll_approval_sends(&mut self) -> bool {
        // Navigation never transfers an irreversible approval action to another draft.
        for (server, threads) in &mut self.machines.threads {
            if server != &self.machines.selected {
                for t in threads {
                    t.approval_send = None;
                }
            }
        }
        let active_id = self.current().id;
        for t in &mut self.threads {
            if t.id != active_id {
                t.approval_send = None;
            }
        }
        let Some(mut send) = self.current_mut().approval_send.take() else {
            return false;
        };
        let imports_pending = self.has_pending_context_imports(&self.machines.selected, active_id);
        if !self.current().approval_draft_unchanged(&send) || imports_pending {
            if send.responded && send.response_result.is_none() {
                self.current_mut().approval_send = Some(send);
                return false;
            }
            if !send.feedback {
                self.approval_redirect_notice(if send.responded {
                    "Approval declined. Your message changed, so it wasn't sent."
                } else {
                    "Your message changed, so it wasn't sent."
                });
            }
            return true;
        }
        if let Some(receiver) = send.preparation.take() {
            match receiver.try_recv() {
                Ok(Ok(prepared)) => send.prepared = Some(prepared),
                Ok(Err(error)) => {
                    self.current_mut().attachment_error = Some(error);
                    return true;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    send.preparation = Some(receiver);
                    self.current_mut().approval_send = Some(send);
                    return false;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.current_mut().error =
                        Some("Could not prepare this send. Your draft was kept.".into());
                    return true;
                }
            }
        }
        if send.remaining_ids.is_none() {
            send.response_draft = Some((
                self.current().draft.clone(),
                Snapshot::capture_with_context(
                    &self.current().draft,
                    &self.current().draft_images,
                    &self.current().draft_context,
                ),
            ));
            let pending = self.current().pending_approvals();
            send.pending_before = pending.iter().map(|r| r["id"].clone()).collect();
            send.remaining_ids = Some(
                pending
                    .iter()
                    .filter(|r| {
                        send.shown_ids.contains(&r["id"])
                            && (!send.feedback || send.shown_ids.first() == Some(&r["id"]))
                            && !crate::approval::not_resumable(r)
                    })
                    .map(|r| r["id"].clone())
                    .collect(),
            );
            if send.feedback && send.remaining_ids.as_ref().is_none_or(Vec::is_empty) {
                return true;
            }
        }
        if send.response_command.is_some() {
            match send.response_result {
                None => {
                    self.current_mut().approval_send = Some(send);
                    return false;
                }
                Some(false) => return true,
                Some(true) => {
                    send.response_command = None;
                    send.response_result = None;
                }
            }
        }
        if !send.remaining_ids.as_ref().expect("initialized").is_empty() {
            let id = send.remaining_ids.as_mut().unwrap().remove(0);
            // The target may have resolved on another client during preparation.
            if !self
                .current()
                .pending_approvals()
                .iter()
                .any(|r| r["id"] == id)
            {
                self.current_mut().approval_send = Some(send);
                return true;
            }
            let mut response = crate::approval::response("decline");
            if send.feedback {
                response["feedback"] = json!(
                    send.prepared.as_ref().expect("prepared").params["text"]
                        .as_str()
                        .unwrap_or("")
                        .trim()
                );
            }
            if let Err(error) = self.respond_to_approval_id(active_id, &id, response) {
                self.current_mut().error = Some(error);
                return true;
            }
            send.responded = true;
            send.response_command = self.current().responding_request;
            send.response_result = send.response_command.is_none().then_some(true);
            self.current_mut().approval_send = Some(send);
            return true;
        }
        if send.feedback {
            self.current_mut().draft.replace_text("");
            self.current_mut().draft_images.clear();
            self.current_mut().draft_context.clear();
            self.current_mut().retained_context = Default::default();
            return true;
        }
        if self
            .current()
            .pending_approvals()
            .iter()
            .any(|r| !send.pending_before.contains(&r["id"]))
        {
            self.approval_redirect_notice("Another approval arrived, so your message wasn't sent.");
            return true;
        }
        if self.client.is_some() {
            let prepared = send.prepared.take().expect("prepared");
            let client = self.client.as_ref().unwrap();
            let selected = if self.machines.selected == "local" {
                Some(client)
            } else {
                client.linked.get(&self.machines.selected)
            };
            let result = selected
                .ok_or("This machine is disconnected.".to_owned())
                .and_then(|client| client.send_prepared(prepared, self.wake.clone()));
            match result {
                Ok(id) => {
                    let server = self.machines.selected.clone();
                    let t = self.current_mut();
                    t.remote_send = Some((id, send.snapshot));
                    t.draft.replace_text("");
                    t.draft_images.clear();
                    t.draft_context.clear();
                    t.retained_context = Default::default();
                    if send.background {
                        t.pending_send_and_new_thread = Some((id, server));
                    }
                }
                Err(error) => self.current_mut().error = Some(error),
            }
        } else {
            if self
                .current()
                .request
                .as_ref()
                .is_some_and(crate::approval::not_resumable)
            {
                self.current_mut().request = None;
            }
            let command = send.prepared.take().expect("prepared");
            let dispatch_queues = command.params["mode"] == "queue";
            let prepared = (|| {
                let images = command.params["attachments"]
                    .as_array()
                    .ok_or("Invalid prepared attachments.")?
                    .iter()
                    .map(|v| Attachment::from_value(v).ok_or("Invalid prepared attachment."))
                    .collect::<Result<Vec<_>, _>>()?;
                let context = command
                    .params
                    .get("context")
                    .filter(|v| !v.is_null())
                    .map(crate::composer_context::decode_message_context)
                    .transpose()?;
                crate::composer_tokens::received_prompt_with_context(
                    command.params["text"]
                        .as_str()
                        .ok_or("Invalid prepared text.")?
                        .to_owned(),
                    images,
                    command
                        .params
                        .get("draft")
                        .filter(|v| !v.is_null())
                        .cloned(),
                    context,
                )
            })();
            match prepared {
                Ok(prompt) => {
                    // Stage once before decline, then submit that immutable content.
                    let t = self.current_mut();
                    t.draft = prompt.draft.as_ref().map_or_else(
                        || crate::composer_rich::draft_from_canonical(&prompt.text),
                        Snapshot::editor,
                    );
                    t.draft_images = prompt
                        .draft
                        .as_ref()
                        .map_or_else(|| prompt.images.to_vec(), |draft| draft.attachments.clone());
                    t.draft_context = prompt
                        .draft
                        .as_ref()
                        .filter(|d| !d.context.is_empty())
                        .map(|d| d.context.clone())
                        .or_else(|| prompt.context.map(|c| c.records))
                        .unwrap_or_default();
                    let previous_follow_up = t.native.follow_up;
                    t.native.follow_up = dispatch_queues;
                    if send.background {
                        self.send_background_intent(true);
                    } else {
                        self.dispatch_send_intent(false, true);
                    }
                    if let Some(t) = self.threads.iter_mut().find(|t| t.id == active_id) {
                        t.native.follow_up = previous_follow_up;
                    }
                }
                Err(error) => self.current_mut().error = Some(error),
            }
        }
        true
    }

    pub(super) fn approval_sends_pending(&self) -> bool {
        self.threads.iter().any(|t| t.approval_send.is_some())
    }
}

/// KSG owns feedback rows separately from provider history. Keep that separation on replay.
pub(super) fn history_with_approval_feedback(messages: &[Value], saved: &[Message]) -> Vec<Value> {
    let mut messages = messages.to_vec();
    let mut last_anchor = 0;
    for old in saved {
        if old.id.starts_with("approval-feedback:") {
            if !messages.iter().any(|m| m["shinaFeedbackId"] == old.id) {
                messages.insert(last_anchor.min(messages.len()), json!({
                    "role":"user","content":[{"type":"text","text":old.text}],"shinaFeedbackId":old.id
                }));
                last_anchor += 1;
            }
        } else if let Some(at) = messages
            .iter()
            .enumerate()
            .skip(last_anchor)
            .find_map(|(at, m)| (content_text(&m["content"]) == old.text).then_some(at))
        {
            last_anchor = at + 1;
        }
    }
    messages
}
